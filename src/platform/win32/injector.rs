use anyhow::{Result, bail};

use crate::model::{Key, MouseButton, Point};
use crate::platform::InputInjector;

/// SendInput based injector. Placeholder until the platform implementation lands.
#[derive(Default)]
pub struct Win32Injector;

impl InputInjector for Win32Injector {
    fn key(&self, _key: Key, _down: bool) -> Result<()> {
        bail!("input injection not implemented yet")
    }

    fn unicode(&self, _utf16: u16, _down: bool) -> Result<()> {
        bail!("input injection not implemented yet")
    }

    fn mouse_move_abs(&self, _pos: Point) -> Result<()> {
        bail!("input injection not implemented yet")
    }

    fn mouse_button(&self, _button: MouseButton, _down: bool) -> Result<()> {
        bail!("input injection not implemented yet")
    }

    fn mouse_wheel(&self, _delta: i32, _horizontal: bool) -> Result<()> {
        bail!("input injection not implemented yet")
    }

    fn cursor_pos(&self) -> Result<Point> {
        bail!("input injection not implemented yet")
    }
}
