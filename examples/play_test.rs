use std::sync::Arc;
use std::time::Duration;

use parrot::engine::player::{Player, PlayerDeps};
use parrot::engine::{EngineDeps, spawn_engine};
use parrot::model::{Action, EngineCommand, EngineEvent, Macro, PlayerControl};
use parrot::platform::sleeper::RealSleeper;
use parrot::platform::win32::{
    capture::Win32Capture, injector::Win32Injector, keys, ocr::Win32Ocr, window::Win32Windows,
};

/// Plays "hi " into the focused window after 3 seconds, directly or through the engine thread.
fn main() -> anyhow::Result<()> {
    env_logger::init();
    let through_engine = std::env::args().any(|a| a == "--engine");
    let (raw_tx, raw_rx) = crossbeam_channel::unbounded();
    let win32 = parrot::platform::win32::spawn_win32_service(raw_tx)?;

    let mut doc = Macro::default();
    for vk in [0x48u16, 0x49, 0x20] {
        doc.push(Action::KeyPress { key: keys::key_from_vk(vk) });
    }
    println!("items: {:?}", doc.items.iter().map(|i| i.action.value_text()).collect::<Vec<_>>());
    println!(
        "focus the target window, playing in 3 seconds ({})",
        if through_engine { "engine" } else { "direct" }
    );
    std::thread::sleep(Duration::from_secs(3));

    let deps = PlayerDeps {
        injector: Arc::new(Win32Injector),
        capture: Arc::new(Win32Capture),
        windows: Arc::new(Win32Windows),
        sleeper: Arc::new(RealSleeper::default()),
        ocr: Arc::new(Win32Ocr),
    };
    if through_engine {
        let engine = spawn_engine(EngineDeps {
            raw_rx,
            win32_tx: win32.cmd_sender(),
            repaint: Box::new(|| {}),
            injector: deps.injector.clone(),
            capture: deps.capture.clone(),
            windows: deps.windows.clone(),
            sleeper: deps.sleeper.clone(),
            ocr: deps.ocr.clone(),
        })?;
        engine.send(EngineCommand::Play { macro_: Arc::new(doc), start_index: 0 });
        loop {
            match engine.evt_rx.recv_timeout(Duration::from_secs(5))? {
                EngineEvent::PlaybackFinished(outcome) => {
                    println!("finished: {outcome:?}");
                    break;
                }
                other => println!("event: {other:?}"),
            }
        }
    } else {
        let ctl = PlayerControl::new();
        let outcome =
            Player::new(deps, ctl, Box::new(|i, it| println!("progress {i} iteration {it}"))).run(&doc, 0);
        println!("finished: {outcome:?}");
    }
    Ok(())
}
