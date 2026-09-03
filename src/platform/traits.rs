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

/// Sends synthetic input; every call is tagged so the hooks can recognise it as our own.
pub trait InputInjector: Send + Sync {
    fn key(&self, key: Key, down: bool) -> Result<()>;
    /// Sends one UTF-16 code unit as a Unicode key event.
    fn unicode(&self, utf16: u16, down: bool) -> Result<()>;
    fn mouse_move_abs(&self, pos: Point) -> Result<()>;
    fn mouse_button(&self, button: MouseButton, down: bool) -> Result<()>;
    /// `delta` in multiples of 120, positive is up or right.
    fn mouse_wheel(&self, delta: i32, horizontal: bool) -> Result<()>;
    fn cursor_pos(&self) -> Result<Point>;
}

pub trait ScreenCapture: Send + Sync {
    /// Bounding rectangle of all monitors in physical pixels.
    fn virtual_screen(&self) -> Rect;
    fn capture(&self, region: Rect) -> Result<RgbaImage>;
}

pub trait WindowManager: Send + Sync {
    /// Finds a visible top-level window; empty filters match anything.
    fn find(&self, title_contains: &str, process_name: &str) -> Option<WindowRef>;
    fn activate(&self, window: WindowRef, timeout: Duration) -> Result<()>;
    fn foreground(&self) -> Option<WindowInfo>;
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
