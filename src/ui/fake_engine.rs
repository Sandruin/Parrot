use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};

use crate::engine::EngineHandle;
use crate::model::{
    Action, ActionId, ActionItem, ButtonEvent, EngineCommand, EngineEvent, ImageMatchMode, Key, Macro,
    MouseButton, PathPoint, PlaybackOutcome, Point, Rect, TextMode, TimeUnit, vk,
};

const RECORD_INTERVAL: Duration = Duration::from_millis(300);
const PLAY_INTERVAL: Duration = Duration::from_millis(200);

/// Scripted engine for GUI development: answers commands with plausible events.
pub fn spawn_fake() -> EngineHandle {
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<EngineCommand>();
    let (evt_tx, evt_rx) = crossbeam_channel::unbounded::<EngineEvent>();
    std::thread::Builder::new()
        .name("fake-engine".into())
        .spawn(move || run(&cmd_rx, &evt_tx))
        .expect("spawning the fake engine thread");
    EngineHandle::from_channels(cmd_tx, evt_rx)
}

fn run(cmd_rx: &Receiver<EngineCommand>, evt_tx: &Sender<EngineEvent>) {
    let mut next_id: ActionId = 1;
    while let Ok(cmd) = cmd_rx.recv() {
        let keep_running = match cmd {
            EngineCommand::StartRecording(_) => record(cmd_rx, evt_tx, &mut next_id),
            EngineCommand::Play { macro_, start_index } => play(cmd_rx, evt_tx, &macro_, start_index),
            EngineCommand::Shutdown => false,
            _ => true,
        };
        if !keep_running {
            break;
        }
    }
}

fn record(cmd_rx: &Receiver<EngineCommand>, evt_tx: &Sender<EngineEvent>, next_id: &mut ActionId) -> bool {
    let _ = evt_tx.send(EngineEvent::RecordingStarted);
    let script = script();
    let mut step = 0;
    loop {
        match cmd_rx.recv_timeout(RECORD_INTERVAL) {
            Ok(EngineCommand::StopRecording) => break,
            Ok(EngineCommand::Shutdown) => return false,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {
                let action = script[step % script.len()].clone();
                step += 1;
                let item = ActionItem::new(*next_id, action);
                *next_id += 1;
                if evt_tx.send(EngineEvent::Recorded(item)).is_err() {
                    return false;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return false,
        }
    }
    let _ = evt_tx.send(EngineEvent::RecordingStopped);
    true
}

fn play(
    cmd_rx: &Receiver<EngineCommand>,
    evt_tx: &Sender<EngineEvent>,
    macro_: &Arc<Macro>,
    start_index: usize,
) -> bool {
    let total = macro_.items.len();
    let _ = evt_tx.send(EngineEvent::PlaybackStarted { total });
    for index in start_index..total {
        match cmd_rx.recv_timeout(PLAY_INTERVAL) {
            Ok(EngineCommand::StopPlayback) => {
                let _ = evt_tx.send(EngineEvent::PlaybackFinished(PlaybackOutcome::StoppedByUser));
                return true;
            }
            Ok(EngineCommand::Shutdown) => return false,
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return false,
        }
        if evt_tx.send(EngineEvent::PlaybackProgress { index, iteration: 1 }).is_err() {
            return false;
        }
    }
    let _ = evt_tx.send(EngineEvent::PlaybackFinished(PlaybackOutcome::Completed));
    true
}

/// The action stream the fake recorder repeats while recording.
fn script() -> Vec<Action> {
    vec![
        Action::Wait { duration: 120.0, unit: TimeUnit::Ms },
        Action::KeyPress { key: Key { vk: 0x48, scancode: 0x23, extended: false } },
        Action::KeyPress { key: Key { vk: 0x49, scancode: 0x17, extended: false } },
        Action::Wait { duration: 340.0, unit: TimeUnit::Ms },
        Action::MouseMove { path: arc_path() },
        Action::MouseButton {
            button: MouseButton::Left,
            event: ButtonEvent::Click,
            pos: Some(Point::new(1210, 744)),
        },
        Action::Wait { duration: 1.2, unit: TimeUnit::S },
        Action::MouseWheel { delta: -240, horizontal: false, pos: Some(Point::new(1210, 744)) },
        Action::KeyDown { key: Key { vk: vk::LSHIFT, scancode: 0x2A, extended: false } },
        Action::KeyUp { key: Key { vk: vk::LSHIFT, scancode: 0x2A, extended: false } },
    ]
}

fn arc_path() -> Vec<PathPoint> {
    (0..24)
        .map(|i| PathPoint { x: 640 + i * 24, y: 480 + (i * i) / 6, dt_ms: if i == 0 { 0 } else { 12 } })
        .collect()
}

/// Macro with one item of every kind, loaded when `MACRO_DEMO_DOC` is set.
pub fn demo_doc() -> Macro {
    let key = Key { vk: 0x41, scancode: 0x1E, extended: false };
    let mut doc = Macro { name: "demo".into(), ..Default::default() };
    let items = [
        (
            Action::WindowActivate {
                title_contains: "Untitled - Notepad".into(),
                process_name: "notepad.exe".into(),
                timeout_ms: 5_000,
            },
            "bring the editor up",
        ),
        (Action::Label { name: "start".into() }, ""),
        (Action::Wait { duration: 250.0, unit: TimeUnit::Ms }, ""),
        (Action::KeyDown { key }, ""),
        (Action::KeyUp { key }, ""),
        (Action::KeyPress { key: Key { vk: vk::RETURN, scancode: 0x1C, extended: false } }, ""),
        (
            Action::TypeText {
                text: "hello from the macro recorder".into(),
                mode: TextMode::Unicode,
                char_delay_ms: 12,
            },
            "typed with unicode events",
        ),
        (Action::MouseMove { path: arc_path() }, ""),
        (
            Action::MouseButton {
                button: MouseButton::Left,
                event: ButtonEvent::Click,
                pos: Some(Point::new(1210, 744)),
            },
            "click the target",
        ),
        (Action::MouseWheel { delta: -360, horizontal: false, pos: Some(Point::new(960, 540)) }, ""),
        (
            Action::WaitForImage {
                region: Rect::new(820, 460, 320, 200),
                template_png: sample_png(),
                similarity: 0.92,
                poll_ms: 250,
                timeout_ms: 10_000,
                mode: ImageMatchMode::Search,
            },
            "wait for the dialog",
        ),
        (
            Action::WaitForText {
                region: Rect::new(100, 100, 400, 40),
                text: "Ready".into(),
                case_sensitive: false,
                poll_ms: 500,
                timeout_ms: 8_000,
            },
            "",
        ),
        (Action::WaitForFile { path: "C:/out/render.png".into(), timeout_ms: 60_000 }, ""),
        (Action::Comment { text: "loop ends here".into() }, ""),
    ];
    for (action, comment) in items {
        let id = doc.push(action);
        if let Some(item) = doc.item_mut(id) {
            item.comment = comment.to_string();
        }
    }
    if let Some(item) = doc.items.get_mut(4) {
        item.enabled = false;
    }
    doc
}

fn sample_png() -> Vec<u8> {
    use image::ImageEncoder as _;

    let mut image = image::RgbaImage::new(64, 40);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let checker = if (x / 8 + y / 8) % 2 == 0 { 210 } else { 60 };
        *pixel = image::Rgba([checker, (x * 4) as u8, (y * 6) as u8, 255]);
    }
    let mut png = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png);
    if encoder
        .write_image(image.as_raw(), image.width(), image.height(), image::ExtendedColorType::Rgba8)
        .is_err()
    {
        return Vec::new();
    }
    png
}
