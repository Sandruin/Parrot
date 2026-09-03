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

    let capture: Arc<dyn platform::ScreenCapture> = Arc::new(platform::win32::capture::Win32Capture);
    let windows: Arc<dyn platform::WindowManager> = Arc::new(platform::win32::window::Win32Windows);
    let services = ui::UiServices { capture: capture.clone(), windows: windows.clone() };

    let repaint_ctx: Arc<Mutex<Option<egui::Context>>> = Default::default();
    let repaint_slot = repaint_ctx.clone();
    let engine = if std::env::var_os("MACRO_FAKE_ENGINE").is_some() {
        log::warn!("MACRO_FAKE_ENGINE is set: using the scripted fake engine");
        drop(raw_rx);
        ui::fake_engine::spawn_fake()
    } else {
        engine::spawn_engine(engine::EngineDeps {
            raw_rx,
            win32_tx: win32.cmd_sender(),
            repaint: Box::new(move || {
                if let Some(ctx) = repaint_slot.lock().unwrap().as_ref() {
                    ctx.request_repaint();
                }
            }),
            injector: Arc::new(platform::win32::injector::Win32Injector),
            capture,
            windows,
            sleeper: Arc::new(platform::sleeper::RealSleeper::default()),
            ocr: Arc::new(platform::win32::ocr::Win32Ocr),
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

    drop(win32);
    Ok(())
}
