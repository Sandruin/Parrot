use std::time::Instant;

use anyhow::{Result, anyhow};
use macro_recorder::model::Rect;
use macro_recorder::platform::native;

const DEFAULT_W: i32 = 800;
const DEFAULT_H: i32 = 300;

/// Captures a screen region given as `x y w h` and prints what the platform OCR reads in it.
/// The recognition runs on a worker thread, which is where the engine normally sees it.
fn main() -> Result<()> {
    native::init();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();

    let services = native::services()?;
    let capture = services.capture;
    let screen = capture.virtual_screen();
    let region = region_from_args(screen);
    println!("virtual screen: {screen:?}");
    println!("region: {region:?}");

    let image = capture.capture(region)?;
    let ocr = services.ocr;
    let worker = std::thread::spawn(move || {
        let started = Instant::now();
        (ocr.recognize(&image), started.elapsed())
    });
    let (lines, elapsed) = worker.join().map_err(|_| anyhow!("OCR thread panicked"))?;
    let lines = lines?;

    let mut words = 0;
    for (index, line) in lines.iter().enumerate() {
        println!("line {index}: {:?}", line.text);
        for word in &line.words {
            let r = word.rect;
            println!("    {:?} at {},{} {}x{}", word.text, r.x, r.y, r.w, r.h);
            words += 1;
        }
    }
    println!("{} lines, {words} words in {:.1} ms", lines.len(), elapsed.as_secs_f64() * 1000.0);
    Ok(())
}

/// `x y w h` from the command line, or the top-left corner of the primary monitor.
fn region_from_args(screen: Rect) -> Rect {
    let numbers: Vec<i32> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    match numbers.as_slice() {
        [x, y, w, h] => Rect::new(*x, *y, *w, *h),
        _ => Rect::new(0, 0, DEFAULT_W.min(screen.w), DEFAULT_H.min(screen.h)),
    }
}
