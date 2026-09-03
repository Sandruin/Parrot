use std::ffi::c_void;

use anyhow::{Result, bail};
use image::RgbaImage;
use windows::Win32::Foundation::{LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors, GetDC, GetDIBits, HDC, HMONITOR, ReleaseDC,
    SRCCOPY, SelectObject,
};
use windows::core::BOOL;

use super::injector::virtual_screen;
use crate::model::Rect;
use crate::platform::ScreenCapture;

/// GDI screen capture; a single `BitBlt` from the screen DC also covers regions spanning monitors.
#[derive(Default)]
pub struct Win32Capture;

impl ScreenCapture for Win32Capture {
    fn virtual_screen(&self) -> Rect {
        virtual_screen()
    }

    fn monitors(&self) -> Vec<Rect> {
        let mut out: Vec<Rect> = Vec::new();
        // SAFETY: the callback only pushes into the Vec we pass by pointer, which outlives the call.
        let _ = unsafe {
            EnumDisplayMonitors(None, None, Some(monitor_proc), LPARAM(&mut out as *mut Vec<Rect> as isize))
        };
        if out.is_empty() {
            out.push(virtual_screen());
        }
        out
    }

    fn capture(&self, region: Rect) -> Result<RgbaImage> {
        if region.w <= 0 || region.h <= 0 {
            bail!("capture region {region:?} is empty");
        }
        // SAFETY: the screen DC is released on every path below.
        let screen = unsafe { GetDC(None) };
        if screen.is_invalid() {
            bail!("GetDC(None) failed: {}", windows::core::Error::from_thread());
        }
        let result = blit(screen, region);
        // SAFETY: releases the DC obtained above.
        unsafe { ReleaseDC(None, screen) };
        result
    }
}

unsafe extern "system" fn monitor_proc(
    _monitor: HMONITOR,
    _dc: HDC,
    rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    // SAFETY: `monitors` passes a pointer to a live Vec and the system passes a valid monitor rectangle.
    unsafe {
        let out = &mut *(lparam.0 as *mut Vec<Rect>);
        let r = *rect;
        out.push(Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top));
    }
    TRUE
}

fn blit(screen: HDC, region: Rect) -> Result<RgbaImage> {
    let (w, h) = (region.w, region.h);
    // SAFETY: every GDI object created here is selected out and deleted before returning.
    unsafe {
        let mem = CreateCompatibleDC(Some(screen));
        if mem.is_invalid() {
            bail!("CreateCompatibleDC failed: {}", windows::core::Error::from_thread());
        }
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        if bitmap.is_invalid() {
            let _ = DeleteDC(mem);
            bail!("CreateCompatibleBitmap failed: {}", windows::core::Error::from_thread());
        }
        let previous = SelectObject(mem, bitmap.into());

        let mut pixels = vec![0u8; (w as usize) * (h as usize) * 4];
        let outcome = (|| -> Result<()> {
            BitBlt(mem, 0, 0, w, h, Some(screen), region.x, region.y, SRCCOPY | CAPTUREBLT)?;
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let lines = GetDIBits(
                mem,
                bitmap,
                0,
                h as u32,
                Some(pixels.as_mut_ptr() as *mut c_void),
                &mut info,
                DIB_RGB_COLORS,
            );
            if lines != h {
                bail!("GetDIBits returned {lines} of {h} lines");
            }
            Ok(())
        })();

        SelectObject(mem, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem);
        outcome?;

        for chunk in pixels.as_chunks_mut::<4>().0 {
            chunk.swap(0, 2);
            chunk[3] = 255;
        }
        RgbaImage::from_raw(w as u32, h as u32, pixels)
            .ok_or_else(|| anyhow::anyhow!("captured buffer does not match {w}x{h}"))
    }
}
