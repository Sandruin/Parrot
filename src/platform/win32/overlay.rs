use std::ffi::c_void;

use anyhow::{Context, Result, bail};
use tiny_skia::Pixmap;
use windows::Win32::Foundation::{
    COLORREF, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HDC, ReleaseDC, SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, HWND_TOPMOST, RegisterClassW,
    SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowDisplayAffinity, SetWindowPos,
    ShowWindow, ULW_ALPHA, UnregisterClassW, UpdateLayeredWindow, WDA_EXCLUDEFROMCAPTURE, WM_DISPLAYCHANGE,
    WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use super::injector::virtual_screen;
use crate::model::{OverlayScene, Rect};
use crate::platform::overlay_render::{render, window_rect};

const CLASS_NAME: PCWSTR = w!("MacroRecorderOverlay");

/// Set this environment variable to keep the overlay visible to screen capture, for manual checks.
const CAPTURABLE_ENV: &str = "MACRO_OVERLAY_CAPTURABLE";

/// Virtual-screen rectangle, cached because every `show` needs it while `WM_DISPLAYCHANGE` is rare.
static VIRTUAL_SCREEN: std::sync::Mutex<Option<Rect>> = std::sync::Mutex::new(None);

fn screen_rect() -> Rect {
    let mut cached = VIRTUAL_SCREEN.lock().unwrap_or_else(|e| e.into_inner());
    *cached.get_or_insert_with(virtual_screen)
}

fn refresh_screen_rect() {
    let rect = virtual_screen();
    *VIRTUAL_SCREEN.lock().unwrap_or_else(|e| e.into_inner()) = Some(rect);
    log::debug!("overlay virtual screen is now {rect:?}");
}

/// Click-through layered window the service thread uses to draw an [`OverlayScene`] on the desktop.
pub struct Overlay {
    hwnd: HWND,
    instance: HINSTANCE,
    visible: bool,
}

impl Overlay {
    /// Registers the window class on first use and creates the still invisible overlay window.
    pub fn new() -> Result<Self> {
        // SAFETY: a null module name asks for this executable's own instance handle.
        let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }.context("GetModuleHandleW")?.into();
        register_class(instance)?;

        let hwnd = create_window(instance)?;
        if std::env::var_os(CAPTURABLE_ENV).is_some() {
            log::info!("{CAPTURABLE_ENV} is set, the overlay stays visible to screen capture");
        } else {
            // SAFETY: plain Win32 call on the window just created.
            if let Err(e) = unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) } {
                log::warn!("SetWindowDisplayAffinity failed, the overlay will show in captures: {e}");
            }
        }
        log::debug!("overlay window created");
        Ok(Self { hwnd, instance, visible: false })
    }

    /// Renders the scene into a layered bitmap and shows it without taking focus.
    pub fn show(&mut self, scene: &OverlayScene) -> Result<()> {
        let Some(rect) = window_rect(scene, screen_rect()) else {
            self.hide();
            return Ok(());
        };
        // SAFETY: moves and sizes our own window, never activating it.
        unsafe {
            SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                rect.x,
                rect.y,
                rect.w,
                rect.h,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        }
        .context("SetWindowPos for the overlay")?;

        let pixmap = render(scene, rect)?;
        push_bitmap(self.hwnd, rect, pixmap)?;
        // SAFETY: plain Win32 call, a no-op once the window is already visible.
        let _ = unsafe { ShowWindow(self.hwnd, SW_SHOWNOACTIVATE) };
        self.visible = true;
        Ok(())
    }

    pub fn hide(&mut self) {
        if !self.visible {
            return;
        }
        // SAFETY: plain Win32 call on our own window.
        let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
        self.visible = false;
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        // SAFETY: the window was created on this thread and is destroyed exactly once.
        unsafe {
            if let Err(e) = DestroyWindow(self.hwnd) {
                log::debug!("DestroyWindow for the overlay failed: {e}");
            }
            if let Err(e) = UnregisterClassW(CLASS_NAME, Some(self.instance)) {
                log::debug!("UnregisterClassW for the overlay class failed: {e}");
            }
        }
    }
}

fn register_class(instance: HINSTANCE) -> Result<()> {
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        hInstance: instance,
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    // SAFETY: `class` lives across the call and the procedure lives for the whole process.
    if unsafe { RegisterClassW(&class) } != 0 {
        return Ok(());
    }
    let error = windows::core::Error::from_thread();
    if error.code() == ERROR_CLASS_ALREADY_EXISTS.to_hresult() {
        return Ok(());
    }
    Err(error).context("RegisterClassW for the overlay class")
}

fn create_window(instance: HINSTANCE) -> Result<HWND> {
    // SAFETY: the class was registered above and both string arguments are static wide literals.
    unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            CLASS_NAME,
            w!("Macro Recorder Overlay"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance),
            None,
        )
    }
    .context("CreateWindowExW for the overlay")
}

extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_DISPLAYCHANGE {
        refresh_screen_rect();
    }
    // SAFETY: the default handler is safe for every message of a window we own.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Converts the premultiplied pixmap to BGRA, wraps it in a top-down DIB and pushes it to the window.
fn push_bitmap(hwnd: HWND, rect: Rect, pixmap: Pixmap) -> Result<()> {
    let mut pixels = pixmap.take();
    for chunk in pixels.as_chunks_mut::<4>().0 {
        chunk.swap(0, 2);
    }

    // SAFETY: the screen DC is released on every path below.
    let screen = unsafe { GetDC(None) };
    if screen.is_invalid() {
        bail!("GetDC(None) failed: {}", windows::core::Error::from_thread());
    }
    let result = blend(screen, hwnd, rect, &pixels);
    // SAFETY: releases the DC obtained above.
    unsafe { ReleaseDC(None, screen) };
    result
}

fn blend(screen: HDC, hwnd: HWND, rect: Rect, pixels: &[u8]) -> Result<()> {
    // SAFETY: every GDI object created here is selected out and deleted before returning.
    unsafe {
        let mem = CreateCompatibleDC(Some(screen));
        if mem.is_invalid() {
            bail!("CreateCompatibleDC failed: {}", windows::core::Error::from_thread());
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: rect.w,
                biHeight: -rect.h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bitmap = match CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) if !bits.is_null() => bitmap,
            Ok(_) => {
                let _ = DeleteDC(mem);
                bail!("CreateDIBSection returned no pixel pointer");
            }
            Err(e) => {
                let _ = DeleteDC(mem);
                return Err(e).context("CreateDIBSection for the overlay bitmap");
            }
        };
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());

        let previous = SelectObject(mem, bitmap.into());
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let destination = POINT { x: rect.x, y: rect.y };
        let size = SIZE { cx: rect.w, cy: rect.h };
        let source = POINT { x: 0, y: 0 };
        let outcome = UpdateLayeredWindow(
            hwnd,
            Some(screen),
            Some(&destination),
            Some(&size),
            Some(mem),
            Some(&source),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );

        SelectObject(mem, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem);
        outcome.context("UpdateLayeredWindow for the overlay")
    }
}
