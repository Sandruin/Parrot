use std::ffi::c_void;

use anyhow::{Context, Result, bail};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};
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
use crate::model::{OverlayScene, OverlayShape, Point, Rect};

const CLASS_NAME: PCWSTR = w!("MacroRecorderOverlay");

/// Set this environment variable to keep the overlay visible to screen capture, for manual checks.
const CAPTURABLE_ENV: &str = "MACRO_OVERLAY_CAPTURABLE";

/// Padding around the scene bounds so strokes, arrow heads and anti-aliasing are never clipped.
const MARGIN: i32 = 24;

const GLYPH_W: usize = 5;
const GLYPH_H: usize = 7;
const GLYPH_SCALE: f32 = 2.0;
const LABEL_PAD_X: i32 = 8;
const LABEL_PAD_Y: i32 = 5;

const TEXT_COLOR: [u8; 4] = [12, 12, 16, 255];

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

/// Scene bounds plus [`MARGIN`], widened to fit label pills and clamped to the virtual screen.
fn window_rect(scene: &OverlayScene, screen: Rect) -> Option<Rect> {
    let bounds = scene.bounds()?;
    let mut left = bounds.x - MARGIN;
    let mut top = bounds.y - MARGIN;
    let mut right = bounds.right() + MARGIN;
    let mut bottom = bounds.bottom() + MARGIN;
    for shape in &scene.shapes {
        if let OverlayShape::Label { at, text, .. } = shape {
            let pill = label_rect(*at, text);
            left = left.min(pill.x - MARGIN);
            top = top.min(pill.y - MARGIN);
            right = right.max(pill.right() + MARGIN);
            bottom = bottom.max(pill.bottom() + MARGIN);
        }
    }
    let left = left.max(screen.x);
    let top = top.max(screen.y);
    let right = right.min(screen.right());
    let bottom = bottom.min(screen.bottom());
    (right > left && bottom > top).then(|| Rect::new(left, top, right - left, bottom - top))
}

fn render(scene: &OverlayScene, rect: Rect) -> Result<Pixmap> {
    let mut pixmap = Pixmap::new(rect.w as u32, rect.h as u32)
        .with_context(|| format!("overlay pixmap {}x{} could not be allocated", rect.w, rect.h))?;
    let transform = Transform::from_translate(-rect.x as f32, -rect.y as f32);
    for shape in &scene.shapes {
        draw_shape(&mut pixmap, shape, transform);
    }
    Ok(pixmap)
}

fn draw_shape(pixmap: &mut Pixmap, shape: &OverlayShape, transform: Transform) {
    match shape {
        OverlayShape::Polyline { points, color, width } => {
            if let Some(path) = polyline_path(points) {
                stroke(pixmap, &path, *color, *width, transform);
            }
            if let Some(head) = arrow_head(points, *width) {
                fill(pixmap, &head, *color, transform);
            }
        }
        OverlayShape::Circle { center, radius, color, filled } => {
            if let Some(path) = PathBuilder::from_circle(px(center.x), px(center.y), radius.max(0.5)) {
                if *filled {
                    fill(pixmap, &path, *color, transform);
                } else {
                    stroke(pixmap, &path, *color, 2.0, transform);
                }
            }
        }
        OverlayShape::Crosshair { center, size, color } => {
            draw_crosshair(pixmap, *center, *size, *color, transform);
        }
        OverlayShape::Rect { rect, color, width } => {
            if let Some(path) = rounded_rect(*rect, 4.0) {
                stroke(pixmap, &path, *color, *width, transform);
            }
        }
        OverlayShape::Label { at, text, color } => draw_label(pixmap, *at, text, *color, transform),
    }
}

/// Pixel centre of an integer screen coordinate, so thin strokes stay crisp.
fn px(value: i32) -> f32 {
    value as f32 + 0.5
}

fn paint(color: [u8; 4]) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color[0], color[1], color[2], color[3]);
    paint
}

fn stroke_of(width: f32) -> Stroke {
    Stroke {
        width: width.max(0.5),
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    }
}

/// Dark halo colour that keeps a shape readable on light and dark backgrounds.
fn shadow(color: [u8; 4]) -> [u8; 4] {
    [0, 0, 0, (color[3] as u16 * 200 / 255) as u8]
}

fn stroke(pixmap: &mut Pixmap, path: &Path, color: [u8; 4], width: f32, transform: Transform) {
    pixmap.stroke_path(path, &paint(shadow(color)), &stroke_of(width + 2.0), transform, None);
    pixmap.stroke_path(path, &paint(color), &stroke_of(width), transform, None);
}

fn fill(pixmap: &mut Pixmap, path: &Path, color: [u8; 4], transform: Transform) {
    pixmap.stroke_path(path, &paint(shadow(color)), &stroke_of(2.0), transform, None);
    pixmap.fill_path(path, &paint(color), FillRule::Winding, transform, None);
}

fn polyline_path(points: &[Point]) -> Option<Path> {
    let (first, rest) = points.split_first()?;
    let mut builder = PathBuilder::with_capacity(points.len(), points.len());
    builder.move_to(px(first.x), px(first.y));
    for point in rest {
        builder.line_to(px(point.x), px(point.y));
    }
    builder.finish()
}

/// Filled triangle pointing along the last segment that is long enough to give a direction.
fn arrow_head(points: &[Point], width: f32) -> Option<Path> {
    if points.len() < 2 {
        return None;
    }
    let tip = *points.last()?;
    let (dx, dy) = points.iter().rev().skip(1).find_map(|p| {
        let (dx, dy) = ((tip.x - p.x) as f32, (tip.y - p.y) as f32);
        let len = (dx * dx + dy * dy).sqrt();
        (len >= 1.0).then_some((dx / len, dy / len))
    })?;
    let length = (width * 4.0).max(12.0);
    let half = length * 0.45;
    let (bx, by) = (px(tip.x) - dx * length, px(tip.y) - dy * length);
    let mut builder = PathBuilder::with_capacity(4, 3);
    builder.move_to(px(tip.x), px(tip.y));
    builder.line_to(bx - dy * half, by + dx * half);
    builder.line_to(bx + dy * half, by - dx * half);
    builder.close();
    builder.finish()
}

fn draw_crosshair(pixmap: &mut Pixmap, center: Point, size: i32, color: [u8; 4], transform: Transform) {
    let size = size.max(6) as f32;
    let ring = size * 0.42;
    let gap = ring + 2.0;
    let (cx, cy) = (px(center.x), px(center.y));
    let mut builder = PathBuilder::with_capacity(8, 8);
    builder.move_to(cx - size, cy);
    builder.line_to(cx - gap, cy);
    builder.move_to(cx + gap, cy);
    builder.line_to(cx + size, cy);
    builder.move_to(cx, cy - size);
    builder.line_to(cx, cy - gap);
    builder.move_to(cx, cy + gap);
    builder.line_to(cx, cy + size);
    if let Some(path) = builder.finish() {
        stroke(pixmap, &path, color, 2.0, transform);
    }
    if let Some(path) = PathBuilder::from_circle(cx, cy, ring) {
        stroke(pixmap, &path, color, 1.5, transform);
    }
}

fn rounded_rect(rect: Rect, radius: f32) -> Option<Path> {
    let left = px(rect.x);
    let top = px(rect.y);
    let right = (rect.right() as f32 - 0.5).max(left);
    let bottom = (rect.bottom() as f32 - 0.5).max(top);
    let r = radius.min((right - left) / 3.0).min((bottom - top) / 3.0).max(0.0);
    let mut builder = PathBuilder::with_capacity(10, 12);
    builder.move_to(left + r, top);
    builder.line_to(right - r, top);
    builder.quad_to(right, top, right, top + r);
    builder.line_to(right, bottom - r);
    builder.quad_to(right, bottom, right - r, bottom);
    builder.line_to(left + r, bottom);
    builder.quad_to(left, bottom, left, bottom - r);
    builder.line_to(left, top + r);
    builder.quad_to(left, top, left + r, top);
    builder.close();
    builder.finish()
}

fn text_size(text: &str) -> (i32, i32) {
    let scale = GLYPH_SCALE as i32;
    let glyphs = text.chars().count() as i32;
    let width = if glyphs == 0 { 0 } else { glyphs * (GLYPH_W as i32 + 1) * scale - scale };
    (width, GLYPH_H as i32 * scale)
}

/// Pill that carries the label text, anchored at the scene position.
fn label_rect(at: Point, text: &str) -> Rect {
    let (width, height) = text_size(text);
    Rect::new(at.x, at.y, width + 2 * LABEL_PAD_X, height + 2 * LABEL_PAD_Y)
}

fn draw_label(pixmap: &mut Pixmap, at: Point, text: &str, color: [u8; 4], transform: Transform) {
    let pill = label_rect(at, text);
    if let Some(path) = rounded_rect(pill, pill.h as f32 * 0.45) {
        fill(pixmap, &path, color, transform);
    }
    let (width, height) = text_size(text);
    let x = pill.x + (pill.w - width) / 2;
    let y = pill.y + (pill.h - height) / 2;
    if let Some(path) = text_path(text, x, y) {
        pixmap.fill_path(&path, &paint(TEXT_COLOR), FillRule::Winding, transform, None);
    }
}

/// Turns text into one path of scaled pixel rectangles, merging each glyph row into runs.
fn text_path(text: &str, x: i32, y: i32) -> Option<Path> {
    let advance = (GLYPH_W as f32 + 1.0) * GLYPH_SCALE;
    let mut builder = PathBuilder::new();
    for (index, ch) in text.chars().enumerate() {
        let origin = x as f32 + index as f32 * advance;
        for (row, bits) in glyph(ch).iter().enumerate() {
            let mut col = 0;
            while col < GLYPH_W {
                if bits & bit(col) == 0 {
                    col += 1;
                    continue;
                }
                let start = col;
                while col < GLYPH_W && bits & bit(col) != 0 {
                    col += 1;
                }
                let cell = tiny_skia::Rect::from_xywh(
                    origin + start as f32 * GLYPH_SCALE,
                    y as f32 + row as f32 * GLYPH_SCALE,
                    (col - start) as f32 * GLYPH_SCALE,
                    GLYPH_SCALE,
                );
                if let Some(cell) = cell {
                    builder.push_rect(cell);
                }
            }
        }
    }
    builder.finish()
}

fn bit(col: usize) -> u8 {
    1 << (GLYPH_W - 1 - col)
}

fn glyph(ch: char) -> [u8; GLYPH_H] {
    let upper = ch.to_ascii_uppercase();
    FONT.iter().find(|(c, _)| *c == upper).map(|(_, rows)| *rows).unwrap_or(UNKNOWN_GLYPH)
}

const UNKNOWN_GLYPH: [u8; GLYPH_H] = [0b11111, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11111];

/// Uppercase letters, digits and a little punctuation in a 5x7 cell, one byte per row.
const FONT: &[(char, [u8; GLYPH_H])] = &[
    (' ', [0, 0, 0, 0, 0, 0, 0]),
    ('A', [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001]),
    ('B', [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110]),
    ('C', [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110]),
    ('D', [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110]),
    ('E', [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111]),
    ('F', [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000]),
    ('G', [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110]),
    ('H', [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001]),
    ('I', [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111]),
    ('J', [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100]),
    ('K', [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001]),
    ('L', [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111]),
    ('M', [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001]),
    ('N', [0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001]),
    ('O', [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110]),
    ('P', [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000]),
    ('Q', [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10011, 0b01101]),
    ('R', [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001]),
    ('S', [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110]),
    ('T', [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100]),
    ('U', [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110]),
    ('V', [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100]),
    ('W', [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001]),
    ('X', [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001]),
    ('Y', [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100]),
    ('Z', [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111]),
    ('0', [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110]),
    ('1', [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110]),
    ('2', [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111]),
    ('3', [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110]),
    ('4', [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010]),
    ('5', [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110]),
    ('6', [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110]),
    ('7', [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000]),
    ('8', [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110]),
    ('9', [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100]),
    ('.', [0, 0, 0, 0, 0, 0b01100, 0b01100]),
    (',', [0, 0, 0, 0, 0b01100, 0b00100, 0b01000]),
    (':', [0, 0b01100, 0b01100, 0, 0b01100, 0b01100, 0]),
    ('-', [0, 0, 0, 0b01110, 0, 0, 0]),
    ('+', [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0]),
    ('=', [0, 0, 0b11111, 0, 0b11111, 0, 0]),
    ('/', [0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000]),
    ('(', [0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010]),
    (')', [0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000]),
    ('!', [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0, 0b00100]),
    ('?', [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0, 0b00100]),
    ('%', [0b11000, 0b11001, 0b00010, 0b00100, 0b01000, 0b10011, 0b00011]),
];

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

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect::new(0, 0, 1920, 1080);

    fn scene(shapes: Vec<OverlayShape>) -> OverlayScene {
        OverlayScene { shapes }
    }

    #[test]
    fn window_rect_adds_the_margin_and_clamps_to_the_screen() {
        let far = scene(vec![OverlayShape::Crosshair {
            center: Point::new(500, 400),
            size: 10,
            color: [255, 0, 0, 255],
        }]);
        assert_eq!(window_rect(&far, SCREEN), Some(Rect::new(466, 366, 68, 68)));

        let corner = scene(vec![OverlayShape::Circle {
            center: Point::new(2, 2),
            radius: 4.0,
            color: [255, 0, 0, 255],
            filled: true,
        }]);
        assert_eq!(window_rect(&corner, SCREEN), Some(Rect::new(0, 0, 30, 30)));

        assert_eq!(window_rect(&OverlayScene::default(), SCREEN), None);
    }

    #[test]
    fn window_rect_covers_the_whole_label_pill() {
        let text = "right click";
        let label = scene(vec![OverlayShape::Label {
            at: Point::new(400, 300),
            text: text.into(),
            color: [255, 0, 0, 220],
        }]);
        let rect = window_rect(&label, SCREEN).expect("a label has bounds");
        let pill = label_rect(Point::new(400, 300), text);
        assert!(rect.contains(Point::new(pill.right(), pill.bottom())), "{rect:?} vs {pill:?}");
        assert!(rect.right() >= pill.right() + MARGIN);
    }

    #[test]
    fn rendering_paints_inside_the_window() {
        let center = Point::new(600, 500);
        let scene = scene(vec![
            OverlayShape::Polyline {
                points: vec![Point::new(560, 460), Point::new(580, 480), center],
                color: [96, 165, 250, 220],
                width: 2.0,
            },
            OverlayShape::Crosshair { center, size: 16, color: [239, 68, 68, 220] },
            OverlayShape::Label {
                at: Point::new(center.x + 20, center.y + 8),
                text: "left click".into(),
                color: [239, 68, 68, 220],
            },
        ]);
        let rect = window_rect(&scene, SCREEN).expect("bounds");
        let pixmap = render(&scene, rect).expect("render");
        let alpha = |p: Point| {
            let index = ((p.y - rect.y) as usize * rect.w as usize + (p.x - rect.x) as usize) * 4 + 3;
            pixmap.data()[index]
        };
        assert!(alpha(Point::new(center.x + 12, center.y)) > 0, "crosshair arm is missing");
        assert_eq!(alpha(Point::new(rect.x + 1, rect.y + 1)), 0, "the margin must stay transparent");
        assert!(pixmap.data().iter().skip(3).step_by(4).filter(|a| **a > 0).count() > 500);
    }

    #[test]
    fn the_font_covers_every_label_the_gui_produces() {
        for ch in "left right middle x1 x2 down up click 0123456789".chars() {
            assert_ne!(glyph(ch), UNKNOWN_GLYPH, "no glyph for {ch:?}");
        }
        assert_eq!(glyph('~'), UNKNOWN_GLYPH);
        assert!(text_path("AB", 0, 0).is_some());
        assert!(text_path("", 0, 0).is_none());
    }
}
