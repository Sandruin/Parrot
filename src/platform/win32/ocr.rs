use std::borrow::Cow;
use std::cell::Cell;
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow, bail};
use image::imageops::{self, FilterType};
use image::{Rgba, RgbaImage};
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::Win32::Foundation::{RPC_E_CHANGED_MODE, S_FALSE};
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

use crate::model::Rect;
use crate::platform::{Ocr, OcrLine, OcrWord};

/// Shortest side the recognizer handles reliably; smaller images are padded onto a white canvas.
const MIN_SIDE: u32 = 40;

static ENGINE: Mutex<Option<OcrEngine>> = Mutex::new(None);

thread_local! {
    static WINRT_READY: Cell<bool> = const { Cell::new(false) };
}

/// Text recognition through Windows.Media.Ocr; the engine is created on first use and then shared.
#[derive(Default)]
pub struct Win32Ocr;

impl Ocr for Win32Ocr {
    fn recognize(&self, image: &RgbaImage) -> Result<Vec<OcrLine>> {
        if image.width() == 0 || image.height() == 0 {
            return Ok(Vec::new());
        }
        ensure_winrt()?;
        let engine = engine()?;
        let max_dim = OcrEngine::MaxImageDimension().context("OcrEngine::MaxImageDimension failed")?;
        let (prepared, map) = prepare(image, max_dim);
        log::debug!(
            "OCR on {}x{}, analysed as {}x{}, engine limit {max_dim}",
            image.width(),
            image.height(),
            prepared.width(),
            prepared.height()
        );
        let bitmap = software_bitmap(prepared.as_ref())?;
        let result = engine
            .RecognizeAsync(&bitmap)
            .context("RecognizeAsync failed")?
            .join()
            .context("OCR recognition failed")?;

        let mut lines = Vec::new();
        for line in result.Lines()? {
            let mut words = Vec::new();
            for word in line.Words()? {
                let bounds = word.BoundingRect()?;
                words.push(OcrWord {
                    text: word.Text()?.to_string(),
                    rect: map.apply(bounds.X, bounds.Y, bounds.Width, bounds.Height),
                });
            }
            lines.push(OcrLine { text: line.Text()?.to_string(), words });
        }
        Ok(lines)
    }
}

/// Maps word boxes from the analysed image back onto the original pixel grid.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BoxMap {
    scale_x: f32,
    scale_y: f32,
    pad_x: f32,
    pad_y: f32,
    width: i32,
    height: i32,
}

impl BoxMap {
    fn identity(width: u32, height: u32) -> Self {
        Self {
            scale_x: 1.0,
            scale_y: 1.0,
            pad_x: 0.0,
            pad_y: 0.0,
            width: width as i32,
            height: height as i32,
        }
    }

    /// Converts one box from analysed-image pixels to original pixels, clamped to the image.
    fn apply(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        let left = ((x - self.pad_x) / self.scale_x).round() as i32;
        let top = ((y - self.pad_y) / self.scale_y).round() as i32;
        let right = ((x + w - self.pad_x) / self.scale_x).round() as i32;
        let bottom = ((y + h - self.pad_y) / self.scale_y).round() as i32;
        let left = left.clamp(0, self.width);
        let top = top.clamp(0, self.height);
        let right = right.clamp(left, self.width);
        let bottom = bottom.clamp(top, self.height);
        Rect::new(left, top, right - left, bottom - top)
    }
}

/// Fits the image into `max_dim` and pads it up to `MIN_SIDE`, returning the map back to the original.
fn prepare(image: &RgbaImage, max_dim: u32) -> (Cow<'_, RgbaImage>, BoxMap) {
    let (width, height) = image.dimensions();
    let mut map = BoxMap::identity(width, height);
    let mut prepared = Cow::Borrowed(image);

    let longest = width.max(height);
    if max_dim > 0 && longest > max_dim {
        let factor = f64::from(max_dim) / f64::from(longest);
        let target_w = ((f64::from(width) * factor).round() as u32).max(1);
        let target_h = ((f64::from(height) * factor).round() as u32).max(1);
        map.scale_x = target_w as f32 / width as f32;
        map.scale_y = target_h as f32 / height as f32;
        prepared = Cow::Owned(imageops::resize(image, target_w, target_h, FilterType::Triangle));
    }

    let (inner_w, inner_h) = prepared.dimensions();
    if inner_w < MIN_SIDE || inner_h < MIN_SIDE {
        let canvas_w = inner_w.max(MIN_SIDE);
        let canvas_h = inner_h.max(MIN_SIDE);
        map.pad_x = ((canvas_w - inner_w) / 2) as f32;
        map.pad_y = ((canvas_h - inner_h) / 2) as f32;
        let mut canvas = RgbaImage::from_pixel(canvas_w, canvas_h, Rgba([255, 255, 255, 255]));
        imageops::replace(&mut canvas, prepared.as_ref(), map.pad_x as i64, map.pad_y as i64);
        prepared = Cow::Owned(canvas);
    }

    (prepared, map)
}

fn software_bitmap(image: &RgbaImage) -> Result<SoftwareBitmap> {
    let mut bgra = image.as_raw().clone();
    for chunk in bgra.as_chunks_mut::<4>().0 {
        chunk.swap(0, 2);
    }
    let writer = DataWriter::new()?;
    writer.WriteBytes(&bgra)?;
    let buffer = writer.DetachBuffer()?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        image.width() as i32,
        image.height() as i32,
    )
    .context("SoftwareBitmap::CreateCopyFromBuffer failed")?;
    Ok(bitmap)
}

fn engine() -> Result<OcrEngine> {
    let mut slot = ENGINE.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(engine) = slot.as_ref() {
        return Ok(engine.clone());
    }
    let engine = create_engine()?;
    *slot = Some(engine.clone());
    Ok(engine)
}

/// Prefers the user profile languages and falls back to the first installed recognizer language.
fn create_engine() -> Result<OcrEngine> {
    match OcrEngine::TryCreateFromUserProfileLanguages() {
        Ok(engine) => {
            log::debug!("OCR engine for user profile languages: {}", language_tag(&engine));
            return Ok(engine);
        }
        Err(err) => log::debug!("no OCR engine for the user profile languages: {err}"),
    }

    let languages = OcrEngine::AvailableRecognizerLanguages()
        .context("OcrEngine::AvailableRecognizerLanguages failed")?;
    if languages.Size()? == 0 {
        bail!(
            "no OCR language pack installed; add one under Settings > Time & language > \
             Language & region > Add a language, including its optional \"Basic typing\" feature"
        );
    }
    let language = languages.GetAt(0)?;
    let tag = language.LanguageTag().unwrap_or_default();
    let engine = OcrEngine::TryCreateFromLanguage(&language)
        .with_context(|| format!("no OCR engine for the installed language {tag}"))?;
    log::debug!("OCR engine for fallback language {tag}");
    Ok(engine)
}

fn language_tag(engine: &OcrEngine) -> String {
    engine
        .RecognizerLanguage()
        .and_then(|language| language.LanguageTag())
        .map(|tag| tag.to_string())
        .unwrap_or_else(|_| "unknown".into())
}

/// Puts the calling thread into the multithreaded apartment, since `recognize` may run anywhere.
fn ensure_winrt() -> Result<()> {
    WINRT_READY.with(|ready| {
        if ready.get() {
            return Ok(());
        }
        // SAFETY: plain FFI call; an apartment set up by someone else is tolerated below.
        if let Err(err) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
            let code = err.code();
            if code != RPC_E_CHANGED_MODE && code != S_FALSE {
                return Err(anyhow!("RoInitialize failed: {err}"));
            }
        }
        ready.set(true);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_map_rounds_box_edges() {
        let map = BoxMap::identity(100, 50);
        assert_eq!(map.apply(10.2, 20.8, 30.4, 8.4), Rect::new(10, 21, 31, 8));
    }

    #[test]
    fn downscaled_boxes_map_back_to_the_original_grid() {
        let source = RgbaImage::new(1000, 400);
        let (prepared, map) = prepare(&source, 200);
        assert_eq!(prepared.dimensions(), (200, 80));
        assert_eq!(map.apply(20.0, 8.0, 40.0, 4.0), Rect::new(100, 40, 200, 20));
    }

    #[test]
    fn small_images_are_padded_and_centred() {
        let source = RgbaImage::new(10, 9);
        let (prepared, map) = prepare(&source, 4096);
        assert_eq!(prepared.dimensions(), (40, 40));
        assert_eq!((map.pad_x, map.pad_y), (15.0, 15.0));
        assert_eq!(map.apply(15.0, 15.0, 10.0, 9.0), Rect::new(0, 0, 10, 9));
    }

    #[test]
    fn padding_and_downscaling_combine() {
        let source = RgbaImage::new(400, 40);
        let (prepared, map) = prepare(&source, 100);
        assert_eq!(prepared.dimensions(), (100, 40));
        assert_eq!((map.pad_x, map.pad_y), (0.0, 15.0));
        assert_eq!(map.apply(0.0, 15.0, 50.0, 10.0), Rect::new(0, 0, 200, 40));
    }

    #[test]
    fn images_within_the_limits_are_untouched() {
        let source = RgbaImage::new(800, 300);
        let (prepared, map) = prepare(&source, 4096);
        assert_eq!(prepared.dimensions(), (800, 300));
        assert_eq!(map, BoxMap::identity(800, 300));
    }

    #[test]
    fn boxes_are_clamped_to_the_original_image() {
        let map = BoxMap::identity(20, 10);
        assert_eq!(map.apply(-5.0, -5.0, 100.0, 100.0), Rect::new(0, 0, 20, 10));
        assert_eq!(map.apply(30.0, 30.0, 5.0, 5.0), Rect::new(20, 10, 0, 0));
    }
}
