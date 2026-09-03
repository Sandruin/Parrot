use anyhow::{Result, bail};
use image::RgbaImage;

use crate::model::Rect;
use crate::platform::ScreenCapture;

/// Screen capture backed by xcap. Placeholder until the platform implementation lands.
#[derive(Default)]
pub struct Win32Capture;

impl ScreenCapture for Win32Capture {
    fn virtual_screen(&self) -> Rect {
        Rect::default()
    }

    fn capture(&self, _region: Rect) -> Result<RgbaImage> {
        bail!("screen capture not implemented yet")
    }
}
