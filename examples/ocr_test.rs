use std::time::Instant;

use anyhow::{Result, anyhow};
use parrot::model::Rect;
use parrot::platform::win32::capture::Win32Capture;
use parrot::platform::win32::dpi;
use parrot::platform::win32::ocr::Win32Ocr;
use parrot::platform::{Ocr, ScreenCapture};

const DEFAULT_W: i32 = 800;
const DEFAULT_H: i32 = 300;

/// Captures a screen region given as `x y w h` and prints what Windows.Media.Ocr reads in it.
/// The recognition runs on a worker thread, which is where the engine normally sees it.
fn main() -> Result<()> {
    dpi::ensure_per_monitor_v2();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();

    let capture = Win32Capture;
    let screen = capture.virtual_screen();
    let region = region_from_args(screen);
    println!("virtual screen: {screen:?}");
    println!("region: {region:?}");

    let image = capture.capture(region)?;
    let worker = std::thread::spawn(move || {
        let started = Instant::now();
        (Win32Ocr.recognize(&image), started.elapsed())
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
