use anyhow::{Result, bail};

use crate::model::{Key, MouseButton, Point};
use crate::platform::InputInjector;

/// Input injection through the virtual pointer and virtual keyboard protocols on its own connection.
pub struct WaylandInjector {}

impl WaylandInjector {
    pub fn new() -> Result<Self> {
        Ok(Self {})
    }
}

impl InputInjector for WaylandInjector {
    fn key(&self, _key: Key, _down: bool) -> Result<()> {
        bail!("key injection is not implemented yet")
    }

    fn unicode(&self, _ch: char, _down: bool) -> Result<()> {
        bail!("unicode injection is not implemented yet")
    }

    fn mouse_move_abs(&self, _pos: Point) -> Result<()> {
        bail!("mouse injection is not implemented yet")
    }

    fn mouse_move_rel(&self, _dx: i32, _dy: i32) -> Result<()> {
        bail!("mouse injection is not implemented yet")
    }

    fn mouse_button(&self, _button: MouseButton, _down: bool) -> Result<()> {
        bail!("mouse injection is not implemented yet")
    }

    fn mouse_wheel(&self, _delta: i32, _horizontal: bool) -> Result<()> {
        bail!("mouse injection is not implemented yet")
    }

    fn cursor_pos(&self) -> Result<Point> {
        bail!("cursor position is not implemented yet")
    }
}
