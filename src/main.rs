#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{Arc, Mutex};

use anyhow::Result;
use macro_recorder::platform::{PlatformServices, native};
use macro_recorder::{engine, model, ui};

fn main() -> Result<()> {
    native::init();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let settings = model::AppSettings::load_or_default();
    let (raw_tx, raw_rx) = crossbeam_channel::unbounded();

    let service = native::spawn_service(raw_tx)?;
    service.send(model::PlatformCommand::SetHotkeys(settings.hotkeys.clone()));

    let PlatformServices { injector, capture, windows, ocr } = native::services()?;
    let services =
        ui::UiServices { injector: injector.clone(), capture: capture.clone(), windows: windows.clone() };

    let repaint_ctx: Arc<Mutex<Option<egui::Context>>> = Default::default();
    let repaint_slot = repaint_ctx.clone();
    let engine = if std::env::var_os("MACRO_FAKE_ENGINE").is_some() {
        log::warn!("MACRO_FAKE_ENGINE is set: using the scripted fake engine");
        drop(raw_rx);
        ui::fake_engine::spawn_fake()
    } else {
        engine::spawn_engine(engine::EngineDeps {
            raw_rx,
            platform_tx: service.cmd_sender(),
            repaint: Box::new(move || {
                if let Some(ctx) = repaint_slot.lock().unwrap().as_ref() {
                    ctx.request_repaint();
                }
            }),
            injector,
            capture,
            windows,
            sleeper: Arc::new(macro_recorder::platform::sleeper::RealSleeper::default()),
            ocr,
        })?
    };

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
            let mut app = ui::App::new(cc, engine, settings, services);
            if let Some(path) = std::env::args_os().nth(1) {
                ui::files::open_path(&mut app, path.into());
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))?;

    drop(service);
    Ok(())
}
