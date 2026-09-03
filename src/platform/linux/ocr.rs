use anyhow::{Result, bail};
use image::RgbaImage;

use crate::platform::{Ocr, OcrLine};

/// Text recognition through the system's tesseract library.
#[derive(Default)]
pub struct TesseractOcr;

impl Ocr for TesseractOcr {
    fn recognize(&self, _image: &RgbaImage) -> Result<Vec<OcrLine>> {
        bail!("OCR is not implemented yet")
    }
}
