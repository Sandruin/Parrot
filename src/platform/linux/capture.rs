use anyhow::{Result, bail};
use image::RgbaImage;

use crate::model::Rect;
use crate::platform::ScreenCapture;

/// Screencopy bookkeeping the Wayland dispatcher fills in while a capture is pending.
#[derive(Default)]
pub struct WlState {}

/// Screen capture through wlr-screencopy on its own Wayland connection.
pub struct WaylandCapture {}

impl WaylandCapture {
    pub fn new() -> Result<Self> {
        Ok(Self {})
    }
}

impl ScreenCapture for WaylandCapture {
    fn virtual_screen(&self) -> Rect {
        Rect::default()
    }

    fn capture(&self, _region: Rect) -> Result<RgbaImage> {
        bail!("screen capture is not implemented yet")
    }
}
