use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use image::RgbaImage;

use crate::model::{Key, MouseButton, PlayerControl, Point, Rect};

/// Native window handle wrapped so the engine never touches Win32 types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowRef(pub isize);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowInfo {
    pub handle: WindowRef,
    pub title: String,
    /// Executable file name such as `notepad.exe`.
    pub process_name: String,
}

/// Key plus modifiers that produce a character on the active keyboard layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharKey {
    pub key: Key,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// Sends synthetic input; every call is tagged so the hooks can recognise it as our own.
pub trait InputInjector: Send + Sync {
    fn key(&self, key: Key, down: bool) -> Result<()>;
    /// Sends a character as a Unicode key event, independent of the keyboard layout.
    fn unicode(&self, ch: char, down: bool) -> Result<()>;
    fn mouse_move_abs(&self, pos: Point) -> Result<()>;
    /// Moves the cursor by a raw delta, which is what games reading raw input expect.
    fn mouse_move_rel(&self, dx: i32, dy: i32) -> Result<()>;
    fn mouse_button(&self, button: MouseButton, down: bool) -> Result<()>;
    /// `delta` in multiples of 120, positive is up or right.
    fn mouse_wheel(&self, delta: i32, horizontal: bool) -> Result<()>;
    fn cursor_pos(&self) -> Result<Point>;
    /// Scan-code chord for `ch` on the current layout, `None` when the platform cannot tell.
    fn key_for_char(&self, _ch: char) -> Option<CharKey> {
        None
    }
}

pub trait ScreenCapture: Send + Sync {
    /// Bounding rectangle of all monitors in physical pixels.
    fn virtual_screen(&self) -> Rect;
    /// Bounds of every monitor in physical pixels; defaults to the whole virtual screen.
    fn monitors(&self) -> Vec<Rect> {
        vec![self.virtual_screen()]
    }
    fn capture(&self, region: Rect) -> Result<RgbaImage>;
}

pub trait WindowManager: Send + Sync {
    /// Finds a visible top-level window; empty filters match anything.
    fn find(&self, title_contains: &str, process_name: &str) -> Option<WindowRef>;
    fn activate(&self, window: WindowRef, timeout: Duration) -> Result<()>;
    fn foreground(&self) -> Option<WindowInfo>;
}

/// One recognized word with its bounding box in pixels of the analysed image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrWord {
    pub text: String,
    pub rect: Rect,
}

/// One recognized line; `text` is the words joined by single spaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrLine {
    pub text: String,
    pub words: Vec<OcrWord>,
}

pub trait Ocr: Send + Sync {
    fn recognize(&self, image: &RgbaImage) -> Result<Vec<OcrLine>>;
}

/// The platform implementations the engine and the GUI share.
#[derive(Clone)]
pub struct PlatformServices {
    pub injector: Arc<dyn InputInjector>,
    pub capture: Arc<dyn ScreenCapture>,
    pub windows: Arc<dyn WindowManager>,
    pub ocr: Arc<dyn Ocr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitResult {
    Elapsed,
    Stopped,
}

/// Time source and interruptible sleep so the player can run against a virtual clock in tests.
pub trait Sleeper: Send + Sync {
    fn now(&self) -> Instant;
    /// Sleeps until `deadline` unless `ctl` requests a stop first.
    fn sleep_until(&self, deadline: Instant, ctl: &PlayerControl) -> WaitResult;
}
