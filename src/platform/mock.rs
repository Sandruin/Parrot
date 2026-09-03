use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use image::RgbaImage;

use super::{
    InputInjector, Ocr, OcrLine, ScreenCapture, Sleeper, WaitResult, WindowInfo, WindowManager, WindowRef,
};
use crate::model::{Key, MouseButton, PlayerControl, Point, Rect};

/// One recorded call on `MockInjector`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InjectedCall {
    Key { key: Key, down: bool },
    Unicode { ch: char, down: bool },
    MoveAbs(Point),
    MoveRel { dx: i32, dy: i32 },
    Button { button: MouseButton, down: bool },
    Wheel { delta: i32, horizontal: bool },
}

/// Records every injected call and tracks a fake cursor position.
#[derive(Default)]
pub struct MockInjector {
    pub calls: Mutex<Vec<InjectedCall>>,
    pub cursor: Mutex<Point>,
}

impl MockInjector {
    pub fn calls(&self) -> Vec<InjectedCall> {
        self.calls.lock().unwrap().clone()
    }

    fn push(&self, call: InjectedCall) -> Result<()> {
        self.calls.lock().unwrap().push(call);
        Ok(())
    }
}

impl InputInjector for MockInjector {
    fn key(&self, key: Key, down: bool) -> Result<()> {
        self.push(InjectedCall::Key { key, down })
    }

    fn unicode(&self, ch: char, down: bool) -> Result<()> {
        self.push(InjectedCall::Unicode { ch, down })
    }

    fn mouse_move_abs(&self, pos: Point) -> Result<()> {
        *self.cursor.lock().unwrap() = pos;
        self.push(InjectedCall::MoveAbs(pos))
    }

    fn mouse_move_rel(&self, dx: i32, dy: i32) -> Result<()> {
        {
            let mut cursor = self.cursor.lock().unwrap();
            cursor.x += dx;
            cursor.y += dy;
        }
        self.push(InjectedCall::MoveRel { dx, dy })
    }

    fn mouse_button(&self, button: MouseButton, down: bool) -> Result<()> {
        self.push(InjectedCall::Button { button, down })
    }

    fn mouse_wheel(&self, delta: i32, horizontal: bool) -> Result<()> {
        self.push(InjectedCall::Wheel { delta, horizontal })
    }

    fn cursor_pos(&self) -> Result<Point> {
        Ok(*self.cursor.lock().unwrap())
    }
}

/// Returns a fixed set of lines for every image and counts the calls.
#[derive(Default)]
pub struct MockOcr {
    pub lines: Mutex<Vec<OcrLine>>,
    pub calls: Mutex<usize>,
}

impl Ocr for MockOcr {
    fn recognize(&self, _image: &RgbaImage) -> Result<Vec<OcrLine>> {
        *self.calls.lock().unwrap() += 1;
        Ok(self.lines.lock().unwrap().clone())
    }
}

/// Serves a fixed screen image; captures crop out of it.
pub struct MockCapture {
    pub screen: Mutex<RgbaImage>,
    pub origin: Point,
}

impl MockCapture {
    pub fn new(screen: RgbaImage) -> Self {
        Self { screen: Mutex::new(screen), origin: Point::default() }
    }
}

impl ScreenCapture for MockCapture {
    fn virtual_screen(&self) -> Rect {
        let img = self.screen.lock().unwrap();
        Rect::new(self.origin.x, self.origin.y, img.width() as i32, img.height() as i32)
    }

    fn capture(&self, region: Rect) -> Result<RgbaImage> {
        let img = self.screen.lock().unwrap();
        let x = region.x - self.origin.x;
        let y = region.y - self.origin.y;
        if x < 0
            || y < 0
            || region.right() - self.origin.x > img.width() as i32
            || region.bottom() - self.origin.y > img.height() as i32
        {
            bail!("capture region {:?} outside mock screen", region);
        }
        Ok(image::imageops::crop_imm(&*img, x as u32, y as u32, region.w as u32, region.h as u32).to_image())
    }
}

/// Fixed list of windows; `activate` succeeds for known handles and records the call.
#[derive(Default)]
pub struct MockWindowManager {
    pub windows: Mutex<Vec<WindowInfo>>,
    pub foreground: Mutex<Option<WindowRef>>,
    pub activated: Mutex<Vec<WindowRef>>,
}

impl WindowManager for MockWindowManager {
    fn find(&self, title_contains: &str, process_name: &str) -> Option<WindowRef> {
        let title = title_contains.to_lowercase();
        let process = process_name.to_lowercase();
        self.windows
            .lock()
            .unwrap()
            .iter()
            .find(|w| {
                (title.is_empty() || w.title.to_lowercase().contains(&title))
                    && (process.is_empty() || w.process_name.to_lowercase() == process)
            })
            .map(|w| w.handle)
    }

    fn activate(&self, window: WindowRef, _timeout: Duration) -> Result<()> {
        if !self.windows.lock().unwrap().iter().any(|w| w.handle == window) {
            bail!("window {:?} not found", window);
        }
        self.activated.lock().unwrap().push(window);
        *self.foreground.lock().unwrap() = Some(window);
        Ok(())
    }

    fn foreground(&self) -> Option<WindowInfo> {
        let fg = (*self.foreground.lock().unwrap())?;
        self.windows.lock().unwrap().iter().find(|w| w.handle == fg).cloned()
    }
}

/// Virtual clock: `sleep_until` advances time instantly instead of blocking.
pub struct MockSleeper {
    now: Mutex<Instant>,
    pub sleeps: Mutex<Vec<Duration>>,
}

impl Default for MockSleeper {
    fn default() -> Self {
        Self { now: Mutex::new(Instant::now()), sleeps: Mutex::new(Vec::new()) }
    }
}

impl MockSleeper {
    /// Total virtual time slept so far.
    pub fn total_slept(&self) -> Duration {
        self.sleeps.lock().unwrap().iter().sum()
    }
}

impl Sleeper for MockSleeper {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap()
    }

    fn sleep_until(&self, deadline: Instant, ctl: &PlayerControl) -> WaitResult {
        if ctl.is_stopped() {
            return WaitResult::Stopped;
        }
        let mut now = self.now.lock().unwrap();
        if deadline > *now {
            self.sleeps.lock().unwrap().push(deadline - *now);
            *now = deadline;
        }
        WaitResult::Elapsed
    }
}
