use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use image::RgbaImage;
use macro_recorder::engine::{EngineDeps, EngineHandle, spawn_engine};
use macro_recorder::model::{
    Action, ActionItem, EngineCommand, EngineEvent, Hotkey, HotkeyAction, HotkeyConfig, Key, Macro,
    MacroSettings, PlaybackOutcome, Point, RawInputEvent, RecordOptions, Repeat, TimeUnit, Win32Command,
};
use macro_recorder::platform::mock::{MockCapture, MockInjector, MockWindowManager};
use macro_recorder::platform::sleeper::RealSleeper;

struct Rig {
    engine: EngineHandle,
    raw_tx: Sender<RawInputEvent>,
    win32_rx: Receiver<Win32Command>,
    repaints: Arc<AtomicUsize>,
    injector: Arc<MockInjector>,
}

impl Rig {
    fn new() -> Self {
        let (raw_tx, raw_rx) = crossbeam_channel::unbounded();
        let (win32_tx, win32_rx) = crossbeam_channel::unbounded();
        let repaints = Arc::new(AtomicUsize::new(0));
        let counter = repaints.clone();
        let injector = Arc::new(MockInjector::default());
        let engine = spawn_engine(EngineDeps {
            raw_rx,
            win32_tx,
            repaint: Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
            injector: injector.clone(),
            capture: Arc::new(MockCapture::new(RgbaImage::new(16, 16))),
            windows: Arc::new(MockWindowManager::default()),
            sleeper: Arc::new(RealSleeper::default()),
        })
        .unwrap();
        Self { engine, raw_tx, win32_rx, repaints, injector }
    }

    fn event(&self) -> EngineEvent {
        self.engine.evt_rx.recv_timeout(Duration::from_secs(5)).expect("an engine event")
    }

    fn win32(&self) -> Win32Command {
        self.win32_rx.recv_timeout(Duration::from_secs(5)).expect("a win32 command")
    }

    fn key(&self, vk: u16, down: bool, at: Instant) {
        self.raw_tx
            .send(RawInputEvent::Key {
                key: Key { vk, scancode: 0, extended: false },
                down,
                injected: false,
                own: false,
                at,
            })
            .unwrap();
    }
}

fn macro_of(actions: Vec<Action>) -> Arc<Macro> {
    let items = actions.into_iter().enumerate().map(|(i, a)| ActionItem::new(i as u64 + 1, a)).collect();
    Arc::new(Macro { items, ..Default::default() })
}

#[test]
fn recording_reports_items_with_running_provisional_ids() {
    let rig = Rig::new();
    rig.engine.send(EngineCommand::StartRecording(RecordOptions::default()));
    assert!(matches!(rig.win32(), Win32Command::EnableHooks(true)));
    assert_eq!(rig.event(), EngineEvent::RecordingStarted);

    let t0 = Instant::now();
    rig.key(0x41, true, t0);
    rig.key(0x41, false, t0 + Duration::from_millis(10));
    rig.key(0x42, true, t0 + Duration::from_millis(500));
    rig.key(0x42, false, t0 + Duration::from_millis(510));

    let mut recorded = Vec::new();
    while recorded.len() < 3 {
        match rig.event() {
            EngineEvent::Recorded(item) => recorded.push(item),
            other => panic!("expected a recorded item, got {other:?}"),
        }
    }
    assert_eq!(recorded.iter().map(|i| i.id).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert_eq!(recorded[0].action, Action::KeyPress { key: Key { vk: 0x41, scancode: 0, extended: false } });
    assert_eq!(recorded[1].action, Action::Wait { duration: 490.0, unit: TimeUnit::Ms });
    assert!(recorded.iter().all(|i| i.enabled && i.comment.is_empty()));

    rig.engine.send(EngineCommand::StopRecording);
    assert!(matches!(rig.win32(), Win32Command::EnableHooks(false)));
    assert_eq!(rig.event(), EngineEvent::RecordingStopped);
    assert!(rig.repaints.load(Ordering::SeqCst) >= 5);
}

#[test]
fn a_held_key_is_flushed_when_recording_stops() {
    let rig = Rig::new();
    rig.engine.send(EngineCommand::StartRecording(RecordOptions::default()));
    assert!(matches!(rig.win32(), Win32Command::EnableHooks(true)));
    assert_eq!(rig.event(), EngineEvent::RecordingStarted);
    rig.key(0x41, true, Instant::now());
    rig.engine.send(EngineCommand::StopRecording);
    match rig.event() {
        EngineEvent::Recorded(item) => assert!(matches!(item.action, Action::KeyDown { .. })),
        other => panic!("expected the held key down, got {other:?}"),
    }
    assert_eq!(rig.event(), EngineEvent::RecordingStopped);
}

#[test]
fn hotkey_chord_keys_are_stripped_from_the_next_recording() {
    let rig = Rig::new();
    let cfg = HotkeyConfig { toggle_record: Some(Hotkey::new(0, 0x74)), ..Default::default() };
    rig.engine.send(EngineCommand::SetHotkeys(cfg));
    assert!(matches!(rig.win32(), Win32Command::SetHotkeys(_)));

    rig.engine.send(EngineCommand::StartRecording(RecordOptions::default()));
    assert!(matches!(rig.win32(), Win32Command::EnableHooks(true)));
    assert_eq!(rig.event(), EngineEvent::RecordingStarted);

    let t0 = Instant::now();
    rig.key(0x74, true, t0);
    rig.key(0x74, false, t0 + Duration::from_millis(10));
    rig.key(0x41, true, t0 + Duration::from_millis(20));
    rig.key(0x41, false, t0 + Duration::from_millis(30));
    match rig.event() {
        EngineEvent::Recorded(item) => {
            assert_eq!(item.action, Action::KeyPress { key: Key { vk: 0x41, scancode: 0, extended: false } });
        }
        other => panic!("expected one key press, got {other:?}"),
    }
}

#[test]
fn playback_reports_start_progress_and_finish() {
    let rig = Rig::new();
    let m = macro_of(vec![
        Action::MouseButton {
            button: macro_recorder::model::MouseButton::Left,
            event: macro_recorder::model::ButtonEvent::Click,
            pos: Some(Point::new(5, 6)),
        },
        Action::Wait { duration: 20.0, unit: TimeUnit::Ms },
    ]);
    rig.engine.send(EngineCommand::Play { macro_: m, start_index: 0 });
    assert!(matches!(rig.win32(), Win32Command::PlaybackStarted(_)));
    assert_eq!(rig.event(), EngineEvent::PlaybackStarted { total: 2 });
    assert_eq!(rig.event(), EngineEvent::PlaybackProgress { index: 0, iteration: 1 });
    assert_eq!(rig.event(), EngineEvent::PlaybackProgress { index: 1, iteration: 1 });
    assert_eq!(rig.event(), EngineEvent::PlaybackFinished(PlaybackOutcome::Completed));
    assert!(matches!(rig.win32(), Win32Command::PlaybackStopped));
    assert_eq!(rig.injector.calls().len(), 3);
}

#[test]
fn stop_playback_and_the_stop_hotkey_both_end_an_endless_run() {
    for stop_via_hotkey in [false, true] {
        let rig = Rig::new();
        let items = vec![ActionItem::new(1, Action::Wait { duration: 30.0, unit: TimeUnit::S })];
        let settings =
            MacroSettings { repeat: Repeat::Infinite, stop_on_user_input: false, ..Default::default() };
        let m = Arc::new(Macro { settings, items, ..Default::default() });
        rig.engine.send(EngineCommand::Play { macro_: m, start_index: 0 });
        assert_eq!(rig.event(), EngineEvent::PlaybackStarted { total: 1 });
        assert_eq!(rig.event(), EngineEvent::PlaybackProgress { index: 0, iteration: 1 });
        if stop_via_hotkey {
            rig.raw_tx.send(RawInputEvent::Hotkey(HotkeyAction::Stop)).unwrap();
            assert_eq!(rig.event(), EngineEvent::HotkeyPressed(HotkeyAction::Stop));
        } else {
            rig.engine.send(EngineCommand::StopPlayback);
        }
        assert_eq!(rig.event(), EngineEvent::PlaybackFinished(PlaybackOutcome::StoppedByUser));
        assert!(matches!(rig.win32(), Win32Command::PlaybackStopped));
    }
}

#[test]
fn playback_is_refused_while_another_one_runs() {
    let rig = Rig::new();
    let items = vec![ActionItem::new(1, Action::Wait { duration: 30.0, unit: TimeUnit::S })];
    let settings = MacroSettings { stop_on_user_input: false, ..Default::default() };
    let m = Arc::new(Macro { settings, items, ..Default::default() });
    rig.engine.send(EngineCommand::Play { macro_: m.clone(), start_index: 0 });
    assert_eq!(rig.event(), EngineEvent::PlaybackStarted { total: 1 });
    assert_eq!(rig.event(), EngineEvent::PlaybackProgress { index: 0, iteration: 1 });

    rig.engine.send(EngineCommand::Play { macro_: m, start_index: 0 });
    match rig.event() {
        EngineEvent::Error(message) => assert!(message.contains("playing"), "{message}"),
        other => panic!("expected an error, got {other:?}"),
    }
    rig.engine.send(EngineCommand::StopPlayback);
    assert_eq!(rig.event(), EngineEvent::PlaybackFinished(PlaybackOutcome::StoppedByUser));
}

#[test]
fn overlay_commands_are_forwarded() {
    let rig = Rig::new();
    let scene = macro_recorder::model::OverlayScene::default();
    rig.engine.send(EngineCommand::ShowOverlay(scene));
    assert!(matches!(rig.win32(), Win32Command::OverlayShow(_)));
    rig.engine.send(EngineCommand::HideOverlay);
    assert!(matches!(rig.win32(), Win32Command::OverlayHide));
}

#[test]
fn hotkeys_are_forwarded_while_idle_without_touching_playback() {
    let rig = Rig::new();
    let mut seen = Vec::new();
    for action in HotkeyAction::ALL {
        rig.raw_tx.send(RawInputEvent::Hotkey(action)).unwrap();
        seen.push(rig.event());
    }
    assert_eq!(seen, HotkeyAction::ALL.map(EngineEvent::HotkeyPressed).to_vec());
}
