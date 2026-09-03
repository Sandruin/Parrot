use std::borrow::Cow;
use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use image::imageops;
use image::{Rgba, RgbaImage};
use tesseract::{OcrEngineMode, PageSegMode, Tesseract};

use crate::model::Rect;
use crate::platform::{Ocr, OcrLine, OcrWord};

/// Shortest side below which recognition degrades; smaller images are padded onto a white canvas.
const MIN_SIDE: u32 = 40;

/// Text recognition through the system's tesseract library; one engine per language is kept warm
/// behind a mutex since building it reloads the language model from disk.
pub struct TesseractOcr {
    engine: Mutex<Option<CachedEngine>>,
}

/// A previously initialised tesseract engine, kept around to skip reloading its language data.
struct CachedEngine {
    tess: Tesseract,
    language: String,
}

impl Default for TesseractOcr {
    fn default() -> Self {
        Self { engine: Mutex::new(None) }
    }
}

impl Ocr for TesseractOcr {
    fn recognize(&self, image: &RgbaImage) -> Result<Vec<OcrLine>> {
        if image.width() == 0 || image.height() == 0 {
            return Ok(Vec::new());
        }
        let (language, datapath) = resolved_language_and_datapath()?;
        let (prepared, map) = prepare(image);
        let started = Instant::now();
        let tsv = self.run_tsv(prepared.as_ref(), &language, &datapath)?;
        log::debug!(
            "tesseract OCR on {}x{} (analysed as {}x{}, lang {language:?}) in {:?}",
            image.width(),
            image.height(),
            prepared.width(),
            prepared.height(),
            started.elapsed()
        );
        Ok(lines_from_tsv(&tsv, &map))
    }
}

impl TesseractOcr {
    /// Recognises `image` and returns its TSV dump, reusing the cached engine when the language matches.
    fn run_tsv(&self, image: &RgbaImage, language: &str, datapath: &Path) -> Result<String> {
        let mut guard = self.engine.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let stale = !matches!(&*guard, Some(cached) if cached.language == language);
        if stale {
            *guard = Some(CachedEngine {
                tess: create_tesseract(datapath, language)?,
                language: language.to_string(),
            });
        }
        let CachedEngine { tess, language } = guard.take().expect("engine ensured present above");

        let width = image.width() as i32;
        let height = image.height() as i32;
        let mut tess = tess
            .set_frame(image.as_raw(), width, height, 4, width * 4)
            .context("tesseract set_frame failed")?;
        tess.set_page_seg_mode(PageSegMode::PsmSparseText);
        let mut tess = tess.recognize().context("tesseract recognize failed")?;
        let tsv = tess.get_tsv_text(0).context("tesseract get_tsv_text failed");
        *guard = Some(CachedEngine { tess, language });
        tsv
    }
}

/// Creates and configures a tesseract engine for `language`, loading data from `datapath`.
fn create_tesseract(datapath: &Path, language: &str) -> Result<Tesseract> {
    let datapath = datapath.to_str().context("tessdata path is not valid UTF-8")?;
    let tess = Tesseract::new_with_oem(Some(datapath), Some(language), OcrEngineMode::LstmOnly)
        .with_context(|| {
            format!("failed to initialise tesseract for language {language:?} using {datapath}")
        })?;
    tess.set_variable("preserve_interword_spaces", "1").context("failed to set preserve_interword_spaces")
}

/// Maps word boxes from the padded analysis canvas back onto the original pixel grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BoxMap {
    pad_x: i32,
    pad_y: i32,
    width: i32,
    height: i32,
}

impl BoxMap {
    fn identity(width: u32, height: u32) -> Self {
        Self { pad_x: 0, pad_y: 0, width: width as i32, height: height as i32 }
    }

    /// Converts one box from canvas pixels to original pixels, clamped to the image.
    fn apply(&self, x: i32, y: i32, w: i32, h: i32) -> Rect {
        let left = (x - self.pad_x).clamp(0, self.width);
        let top = (y - self.pad_y).clamp(0, self.height);
        let right = (x + w - self.pad_x).clamp(left, self.width);
        let bottom = (y + h - self.pad_y).clamp(top, self.height);
        Rect::new(left, top, right - left, bottom - top)
    }
}

/// Pads images smaller than [`MIN_SIDE`] on a side onto a centred white canvas.
fn prepare(image: &RgbaImage) -> (Cow<'_, RgbaImage>, BoxMap) {
    let (width, height) = image.dimensions();
    if width >= MIN_SIDE && height >= MIN_SIDE {
        return (Cow::Borrowed(image), BoxMap::identity(width, height));
    }
    let canvas_w = width.max(MIN_SIDE);
    let canvas_h = height.max(MIN_SIDE);
    let pad_x = ((canvas_w - width) / 2) as i32;
    let pad_y = ((canvas_h - height) / 2) as i32;
    let mut canvas = RgbaImage::from_pixel(canvas_w, canvas_h, Rgba([255, 255, 255, 255]));
    imageops::replace(&mut canvas, image, pad_x as i64, pad_y as i64);
    let map = BoxMap { pad_x, pad_y, width: width as i32, height: height as i32 };
    (Cow::Owned(canvas), map)
}

/// One `level` 5 (word) row from tesseract's TSV output.
struct TsvWord<'a> {
    block: i32,
    par: i32,
    line: i32,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    text: &'a str,
}

/// Parses one TSV row into a word, skipping every non-word row and every row with empty text.
fn parse_word_row(row: &str) -> Option<TsvWord<'_>> {
    let mut fields = row.split('\t');
    if fields.next()? != "5" {
        return None;
    }
    let _page_num = fields.next()?;
    let block = fields.next()?.parse().ok()?;
    let par = fields.next()?.parse().ok()?;
    let line = fields.next()?.parse().ok()?;
    let _word_num = fields.next()?;
    let left = fields.next()?.parse().ok()?;
    let top = fields.next()?.parse().ok()?;
    let width = fields.next()?.parse().ok()?;
    let height = fields.next()?.parse().ok()?;
    let _conf = fields.next()?;
    let text = fields.next()?;
    if text.trim().is_empty() {
        return None;
    }
    Some(TsvWord { block, par, line, left, top, width, height, text })
}

/// Groups consecutive words sharing a block/paragraph/line into one [`OcrLine`] each.
fn lines_from_tsv(tsv: &str, map: &BoxMap) -> Vec<OcrLine> {
    let mut lines: Vec<OcrLine> = Vec::new();
    let mut current_key = None;
    for row in tsv.lines().filter_map(parse_word_row) {
        let key = (row.block, row.par, row.line);
        let word =
            OcrWord { text: row.text.to_string(), rect: map.apply(row.left, row.top, row.width, row.height) };
        if current_key == Some(key) {
            let line = lines.last_mut().expect("current_key is only set once a line exists");
            line.text.push(' ');
            line.text.push_str(&word.text);
            line.words.push(word);
        } else {
            lines.push(OcrLine { text: word.text.clone(), words: vec![word] });
            current_key = Some(key);
        }
    }
    lines
}

/// Resolves the tesseract language string and tessdata directory once and caches the result.
fn resolved_language_and_datapath() -> Result<(String, PathBuf)> {
    static RESOLVED: OnceLock<std::result::Result<(String, PathBuf), String>> = OnceLock::new();
    RESOLVED
        .get_or_init(|| resolve_language_and_datapath().map_err(|err| format!("{err:#}")))
        .clone()
        .map_err(|err| anyhow!(err))
}

fn resolve_language_and_datapath() -> Result<(String, PathBuf)> {
    let (datapath, installed) = locate_tessdata().ok_or_else(|| {
        anyhow!(
            "no tesseract language data found under TESSDATA_PREFIX, /usr/share/tessdata or \
             /usr/share/tesseract-ocr/*/tessdata; install tesseract-data-eng (Arch) or \
             tesseract-ocr-eng (Debian/Ubuntu)"
        )
    })?;
    let override_lang = env::var("MACRO_OCR_LANG").ok();
    let locale = env::var("LC_ALL").ok().filter(|v| !v.is_empty()).or_else(|| env::var("LANG").ok());
    let language = select_language(override_lang.as_deref(), locale.as_deref(), &installed)?;
    log::debug!("tesseract using language {language:?} from {}", datapath.display());
    Ok((language, datapath))
}

/// Picks the first candidate tessdata directory that has any installed language, plus that set.
fn locate_tessdata() -> Option<(PathBuf, HashSet<String>)> {
    tessdata_dir_candidates().into_iter().find_map(|dir| {
        let installed = installed_languages(&dir);
        (!installed.is_empty()).then_some((dir, installed))
    })
}

fn tessdata_dir_candidates() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(prefix) = env::var("TESSDATA_PREFIX")
        && !prefix.trim().is_empty()
    {
        dirs.push(PathBuf::from(prefix));
    }
    dirs.push(PathBuf::from("/usr/share/tessdata"));
    if let Ok(matches) = glob::glob("/usr/share/tesseract-ocr/*/tessdata") {
        dirs.extend(matches.flatten());
    }
    dirs
}

/// Language codes with installed `.traineddata` in `dir`, `osd` excluded; empty if `dir` is missing.
fn installed_languages(dir: &Path) -> HashSet<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return HashSet::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("traineddata") {
                return None;
            }
            let stem = path.file_stem()?.to_str()?;
            (stem != "osd").then(|| stem.to_string())
        })
        .collect()
}

/// Builds the `+`-joined tesseract language string from an env override, the locale and what is installed.
fn select_language(
    env_override: Option<&str>,
    locale: Option<&str>,
    installed: &HashSet<String>,
) -> Result<String> {
    if let Some(lang) = env_override {
        let lang = lang.trim();
        if !lang.is_empty() {
            return Ok(lang.to_string());
        }
    }

    let mut chosen = Vec::new();
    if let Some(code) = locale.and_then(locale_language_code)
        && installed.contains(code)
    {
        chosen.push(code.to_string());
    }
    if installed.contains("eng") && !chosen.iter().any(|lang| lang == "eng") {
        chosen.push("eng".to_string());
    }

    if chosen.is_empty() {
        bail!(
            "no tesseract language data installed; install tesseract-data-eng (Arch) or \
             tesseract-ocr-eng (Debian/Ubuntu), or point TESSDATA_PREFIX at a tessdata directory"
        );
    }
    Ok(chosen.join("+"))
}

/// Maps a POSIX locale such as `de_DE.UTF-8` to the tesseract language code it corresponds to.
fn locale_language_code(locale: &str) -> Option<&'static str> {
    let lang = locale.split(['.', '@']).next().unwrap_or(locale);
    let lang = lang.split('_').next().unwrap_or(lang);
    Some(match lang.to_ascii_lowercase().as_str() {
        "en" => "eng",
        "de" => "deu",
        "fr" => "fra",
        "es" => "spa",
        "it" => "ita",
        "pt" => "por",
        "nl" => "nld",
        "pl" => "pol",
        "ru" => "rus",
        "uk" => "ukr",
        "cs" => "ces",
        "sv" => "swe",
        "da" => "dan",
        "fi" => "fin",
        "nb" | "no" | "nn" => "nor",
        "tr" => "tur",
        "el" => "ell",
        "hu" => "hun",
        "ro" => "ron",
        "bg" => "bul",
        "hr" => "hrv",
        "sk" => "slk",
        "sl" => "slv",
        "et" => "est",
        "lv" => "lav",
        "lt" => "lit",
        "ja" => "jpn",
        "ko" => "kor",
        "zh" => "chi_sim",
        "ar" => "ara",
        "he" | "iw" => "heb",
        "hi" => "hin",
        "th" => "tha",
        "vi" => "vie",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_map_passes_boxes_through() {
        let map = BoxMap::identity(100, 50);
        assert_eq!(map.apply(10, 20, 30, 8), Rect::new(10, 20, 30, 8));
    }

    #[test]
    fn small_images_are_padded_and_centred() {
        let source = RgbaImage::new(10, 9);
        let (prepared, map) = prepare(&source);
        assert_eq!(prepared.dimensions(), (40, 40));
        assert_eq!((map.pad_x, map.pad_y), (15, 15));
        assert_eq!(map.apply(15, 15, 10, 9), Rect::new(0, 0, 10, 9));
    }

    #[test]
    fn only_the_narrow_side_is_padded() {
        let source = RgbaImage::new(400, 20);
        let (prepared, map) = prepare(&source);
        assert_eq!(prepared.dimensions(), (400, 40));
        assert_eq!((map.pad_x, map.pad_y), (0, 10));
        assert_eq!(map.apply(0, 10, 50, 20), Rect::new(0, 0, 50, 20));
    }

    #[test]
    fn images_at_or_above_the_minimum_are_untouched() {
        let source = RgbaImage::new(800, 300);
        let (prepared, map) = prepare(&source);
        assert_eq!(prepared.dimensions(), (800, 300));
        assert_eq!(map, BoxMap::identity(800, 300));
    }

    #[test]
    fn boxes_are_clamped_to_the_original_image() {
        let map = BoxMap::identity(20, 10);
        assert_eq!(map.apply(-5, -5, 100, 100), Rect::new(0, 0, 20, 10));
        assert_eq!(map.apply(30, 30, 5, 5), Rect::new(20, 10, 0, 0));
    }

    #[test]
    fn padded_boxes_map_back_past_the_canvas_origin() {
        let map = BoxMap { pad_x: 10, pad_y: 5, width: 20, height: 20 };
        assert_eq!(map.apply(0, 0, 8, 3), Rect::new(0, 0, 0, 0));
        assert_eq!(map.apply(12, 8, 6, 4), Rect::new(2, 3, 6, 4));
    }

    fn sample_tsv() -> &'static str {
        "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
         1\t1\t0\t0\t0\t0\t0\t0\t100\t50\t-1\t\n\
         2\t1\t1\t0\t0\t0\t10\t10\t80\t20\t-1\t\n\
         5\t1\t1\t1\t1\t1\t10\t10\t30\t20\t95.5\tHello\n\
         5\t1\t1\t1\t1\t2\t45\t10\t45\t20\t90.0\tworld\n\
         5\t1\t1\t1\t2\t1\t10\t35\t20\t15\t40.0\t\n\
         5\t1\t1\t1\t2\t2\t35\t35\t20\t15\t80.0\tfoo\n"
    }

    #[test]
    fn tsv_words_are_grouped_into_lines_by_block_paragraph_and_line() {
        let map = BoxMap::identity(100, 50);
        let lines = lines_from_tsv(sample_tsv(), &map);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!(lines[0].words.len(), 2);
        assert_eq!(lines[0].words[0].rect, Rect::new(10, 10, 30, 20));
        assert_eq!(lines[0].words[1].rect, Rect::new(45, 10, 45, 20));
        assert_eq!(lines[1].text, "foo");
        assert_eq!(lines[1].words, vec![OcrWord { text: "foo".into(), rect: Rect::new(35, 35, 20, 15) }]);
    }

    #[test]
    fn empty_words_are_skipped() {
        let map = BoxMap::identity(100, 50);
        let lines = lines_from_tsv(sample_tsv(), &map);
        assert!(lines.iter().all(|line| !line.text.is_empty()));
        assert!(lines.iter().flat_map(|line| &line.words).all(|word| !word.text.is_empty()));
    }

    #[test]
    fn tsv_word_boxes_are_mapped_back_through_padding() {
        let map = BoxMap { pad_x: 5, pad_y: 5, width: 90, height: 40 };
        let lines = lines_from_tsv(sample_tsv(), &map);
        assert_eq!(lines[0].words[0].rect, Rect::new(5, 5, 30, 20));
    }

    #[test]
    fn non_word_rows_are_ignored() {
        assert!(parse_word_row("1\t1\t0\t0\t0\t0\t0\t0\t100\t50\t-1\t").is_none());
        assert!(parse_word_row("4\t1\t1\t1\t1\t0\t10\t10\t80\t20\t-1\t").is_none());
    }

    #[test]
    fn malformed_rows_are_skipped_without_panicking() {
        assert!(parse_word_row("5\tnot enough fields").is_none());
        assert!(parse_word_row("5\t1\t1\t1\t1\t1\tabc\t10\t30\t20\t95.5\tHello").is_none());
    }

    #[test]
    fn env_override_wins_regardless_of_installed_languages() {
        let installed = HashSet::new();
        assert_eq!(select_language(Some("fra"), None, &installed).unwrap(), "fra");
    }

    #[test]
    fn blank_env_override_is_ignored() {
        let installed = HashSet::from(["eng".to_string()]);
        assert_eq!(select_language(Some("  "), None, &installed).unwrap(), "eng");
    }

    #[test]
    fn locale_language_is_added_ahead_of_english_when_installed() {
        let installed = HashSet::from(["eng".to_string(), "deu".to_string()]);
        assert_eq!(select_language(None, Some("de_DE.UTF-8"), &installed).unwrap(), "deu+eng");
    }

    #[test]
    fn locale_language_is_skipped_when_not_installed() {
        let installed = HashSet::from(["eng".to_string()]);
        assert_eq!(select_language(None, Some("fr_FR.UTF-8"), &installed).unwrap(), "eng");
    }

    #[test]
    fn english_is_not_duplicated_when_the_locale_is_english() {
        let installed = HashSet::from(["eng".to_string()]);
        assert_eq!(select_language(None, Some("en_US.UTF-8"), &installed).unwrap(), "eng");
    }

    #[test]
    fn no_installed_languages_is_an_error() {
        let installed = HashSet::new();
        let err = select_language(None, Some("de_DE.UTF-8"), &installed).unwrap_err();
        assert!(err.to_string().contains("tesseract-data-eng"));
    }

    #[test]
    fn locale_codes_map_common_locales() {
        assert_eq!(locale_language_code("de_DE.UTF-8"), Some("deu"));
        assert_eq!(locale_language_code("fr_FR"), Some("fra"));
        assert_eq!(locale_language_code("en_US.UTF-8"), Some("eng"));
        assert_eq!(locale_language_code("C"), None);
        assert_eq!(locale_language_code("xx_YY.UTF-8"), None);
    }

    #[test]
    fn installed_languages_lists_traineddata_and_excludes_osd() {
        let dir = std::env::temp_dir().join(format!("macro-ocr-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["eng.traineddata", "deu.traineddata", "osd.traineddata", "readme.txt"] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        let installed = installed_languages(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(installed, HashSet::from(["eng".to_string(), "deu".to_string()]));
    }

    #[test]
    fn installed_languages_on_a_missing_directory_is_empty() {
        assert!(installed_languages(Path::new("/does/not/exist")).is_empty());
    }

    /// Runs real recognition against a screenshot, either `MACRO_OCR_TEST_IMAGE` or a fresh `grim`
    /// capture cropped to the top-left corner; prints the recognized lines, words and timing.
    #[test]
    #[ignore = "needs tessdata and, without MACRO_OCR_TEST_IMAGE, a live Wayland session with grim"]
    fn recognizes_a_live_screenshot() {
        let path = match env::var("MACRO_OCR_TEST_IMAGE") {
            Ok(path) => PathBuf::from(path),
            Err(_) => {
                let path = std::env::temp_dir().join("macro-ocr-live-test.png");
                let status = std::process::Command::new("grim")
                    .arg(&path)
                    .status()
                    .expect("failed to run grim; is it installed and is this a Wayland session?");
                assert!(status.success(), "grim exited with {status}");
                path
            }
        };

        let full =
            image::open(&path).unwrap_or_else(|err| panic!("failed to load {path:?}: {err}")).to_rgba8();
        let crop_w = full.width().min(1200);
        let crop_h = full.height().min(400);
        let image = imageops::crop_imm(&full, 0, 0, crop_w, crop_h).to_image();

        let ocr = TesseractOcr::default();
        let started = Instant::now();
        let lines = ocr.recognize(&image).expect("recognition failed");
        let cold = started.elapsed();

        let started = Instant::now();
        let warm_lines = ocr.recognize(&image).expect("recognition failed");
        let warm = started.elapsed();

        println!(
            "recognized {} line(s) from {path:?}: {cold:?} cold (engine created), {warm:?} warm (engine reused)",
            lines.len()
        );
        for line in &lines {
            println!("  line {:?} ({} words)", line.text, line.words.len());
            for word in &line.words {
                println!("    {:?} @ {:?}", word.text, word.rect);
            }
        }
        assert!(!lines.is_empty(), "expected at least one recognized line in {path:?}");
        assert_eq!(lines, warm_lines, "a warm engine should recognize the same image identically");
    }
}
