use anyhow::{Result, bail};
use image::RgbaImage;

use crate::platform::{Ocr, OcrLine};

/// Text recognition through Windows.Media.Ocr. Placeholder until the platform implementation lands.
#[derive(Default)]
pub struct Win32Ocr;

impl Ocr for Win32Ocr {
    fn recognize(&self, _image: &RgbaImage) -> Result<Vec<OcrLine>> {
        bail!("OCR not implemented yet")
    }
}
