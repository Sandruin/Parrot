#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{Arc, Mutex};

use anyhow::Result;
use macro_recorder::{engine, model, platform, ui};

fn main() -> Result<()> {
    #[cfg(windows)]
    platform::win32::dpi::ensure_per_monitor_v2();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let settings = model::AppSettings::load_or_default();
    let (raw_tx, raw_rx) = crossbeam_channel::unbounded();

    #[cfg(not(windows))]
    compile_error!("only Windows is supported right now");
    let win32 = platform::win32::spawn_win32_service(raw_tx)?;
    win32.send(model::Win32Command::SetHotkeys(settings.hotkeys.clone()));

    let repaint_ctx: Arc<Mutex<Option<egui::Context>>> = Default::default();
    let repaint_slot = repaint_ctx.clone();
    let engine = engine::spawn_engine(engine::EngineDeps {
        raw_rx,
        win32_tx: win32.cmd_sender(),
        repaint: Box::new(move || {
            if let Some(ctx) = repaint_slot.lock().unwrap().as_ref() {
                ctx.request_repaint();
            }
        }),
        injector: Arc::new(platform::win32::injector::Win32Injector),
        capture: Arc::new(platform::win32::capture::Win32Capture),
        windows: Arc::new(platform::win32::window::Win32Windows),
        sleeper: Arc::new(platform::sleeper::RealSleeper::default()),
    })?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Macro Recorder")
            .with_inner_size([960.0, 680.0])
            .with_min_inner_size([720.0, 480.0])
            .with_app_id("macro-recorder"),
        ..Default::default()
    };

    eframe::run_native(
        "Macro Recorder",
        options,
        Box::new(move |cc| {
            *repaint_ctx.lock().unwrap() = Some(cc.egui_ctx.clone());
            Ok(Box::new(ui::App::new(cc, engine, settings)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))?;

    drop(win32);
    Ok(())
}
