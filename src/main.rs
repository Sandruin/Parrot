#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(dead_code, unused_imports)] // scaffolding contracts are consumed by later phases

mod engine;
mod model;
mod platform;
mod ui;

use anyhow::Result;

fn main() -> Result<()> {
    #[cfg(windows)]
    platform::win32::dpi::ensure_per_monitor_v2();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let settings = model::AppSettings::load_or_default();
    let (raw_tx, raw_rx) = crossbeam_channel::unbounded();

    #[cfg(windows)]
    let win32 = platform::win32::spawn_win32_service(raw_tx)?;
    #[cfg(not(windows))]
    compile_error!("only Windows is supported right now");

    let win32_tx = win32_cmd_sender(&win32);
    win32.send(model::Win32Command::SetHotkeys(settings.hotkeys.clone()));

    let repaint_ctx: std::sync::Arc<std::sync::Mutex<Option<egui::Context>>> = Default::default();
    let repaint_slot = repaint_ctx.clone();
    let engine = engine::spawn_engine(engine::EngineDeps {
        raw_rx,
        win32_tx,
        repaint: Box::new(move || {
            if let Some(ctx) = repaint_slot.lock().unwrap().as_ref() {
                ctx.request_repaint();
            }
        }),
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

/// The service thread's command sender, cloned out of the handle for the engine.
fn win32_cmd_sender(handle: &platform::win32::Win32Handle) -> crossbeam_channel::Sender<model::Win32Command> {
    handle.cmd_sender()
}
