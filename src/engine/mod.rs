pub mod matcher;
pub mod player;
pub mod recorder;
pub mod scheduler;
pub mod text_match;

use std::sync::Arc;
use std::thread::JoinHandle;

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender};

use crate::model::{
    ActionId, ActionItem, EngineCommand, EngineEvent, HotkeyAction, HotkeyConfig, PlaybackOutcome,
    PlayerControl, RawInputEvent, Win32Command,
};
use crate::platform::{InputInjector, Ocr, ScreenCapture, Sleeper, WindowManager};
use player::{Player, PlayerDeps};
use recorder::Recorder;

/// GUI-side handle: send commands, drain events once per frame.
pub struct EngineHandle {
    pub cmd_tx: Sender<EngineCommand>,
    pub evt_rx: Receiver<EngineEvent>,
    thread: Option<JoinHandle<()>>,
}

impl EngineHandle {
    /// Handle over externally owned channels, used by the GUI's fake engine in development.
    pub fn from_channels(cmd_tx: Sender<EngineCommand>, evt_rx: Receiver<EngineEvent>) -> Self {
        Self { cmd_tx, evt_rx, thread: None }
    }

    pub fn send(&self, cmd: EngineCommand) {
        if self.cmd_tx.send(cmd).is_err() {
            log::warn!("engine thread is gone");
        }
    }

    /// Events that arrived since the last call.
    pub fn drain(&self) -> Vec<EngineEvent> {
        self.evt_rx.try_iter().collect()
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        self.send(EngineCommand::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Everything the engine thread needs from the outside world.
pub struct EngineDeps {
    pub raw_rx: Receiver<RawInputEvent>,
    pub win32_tx: Sender<Win32Command>,
    /// Wakes the GUI after an event was sent.
    pub repaint: Box<dyn Fn() + Send + Sync>,
    pub injector: Arc<dyn InputInjector>,
    pub capture: Arc<dyn ScreenCapture>,
    pub windows: Arc<dyn WindowManager>,
    pub sleeper: Arc<dyn Sleeper>,
    pub ocr: Arc<dyn Ocr>,
}

/// Spawns the engine thread: one recorder, one playback at a time.
pub fn spawn_engine(deps: EngineDeps) -> Result<EngineHandle> {
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<EngineCommand>();
    let (evt_tx, evt_rx) = crossbeam_channel::unbounded::<EngineEvent>();
    let thread = std::thread::Builder::new().name("engine".into()).spawn(move || {
        let EngineDeps { raw_rx, win32_tx, repaint, injector, capture, windows, sleeper, ocr } = deps;
        let (done_tx, done_rx) = crossbeam_channel::unbounded::<PlaybackOutcome>();
        let mut engine = Engine {
            evt_tx,
            win32_tx,
            repaint: Arc::from(repaint),
            player_deps: PlayerDeps { injector, capture, windows, sleeper, ocr },
            state: State::Idle,
            chord_vks: HotkeyConfig::default().chord_vks(),
            next_id: 1,
            done_tx,
        };
        engine.run(&cmd_rx, &raw_rx, &done_rx);
    })?;
    Ok(EngineHandle { cmd_tx, evt_rx, thread: Some(thread) })
}

enum State {
    Idle,
    Recording(Recorder),
    Playing { ctl: Arc<PlayerControl>, thread: JoinHandle<()> },
}

struct Engine {
    evt_tx: Sender<EngineEvent>,
    win32_tx: Sender<Win32Command>,
    repaint: Arc<dyn Fn() + Send + Sync>,
    player_deps: PlayerDeps,
    state: State,
    chord_vks: Vec<u16>,
    next_id: ActionId,
    done_tx: Sender<PlaybackOutcome>,
}

impl Engine {
    fn run(
        &mut self,
        cmd_rx: &Receiver<EngineCommand>,
        raw_rx: &Receiver<RawInputEvent>,
        done_rx: &Receiver<PlaybackOutcome>,
    ) {
        loop {
            crossbeam_channel::select! {
                recv(cmd_rx) -> cmd => match cmd {
                    Ok(cmd) => {
                        if !self.command(cmd) {
                            break;
                        }
                    }
                    Err(_) => break,
                },
                recv(raw_rx) -> raw => match raw {
                    Ok(event) => self.raw(event),
                    Err(_) => break,
                },
                recv(done_rx) -> outcome => match outcome {
                    Ok(outcome) => self.playback_finished(outcome),
                    Err(_) => break,
                },
            }
        }
        self.shutdown();
    }

    /// Handles one command; `false` ends the engine thread.
    fn command(&mut self, cmd: EngineCommand) -> bool {
        match cmd {
            EngineCommand::StartRecording(opts) => {
                if !matches!(self.state, State::Idle) {
                    self.emit(EngineEvent::Error("cannot record while busy".into()));
                    return true;
                }
                self.to_win32(Win32Command::EnableHooks(true));
                self.state = State::Recording(Recorder::new(opts, self.chord_vks.clone()));
                self.emit(EngineEvent::RecordingStarted);
            }
            EngineCommand::StopRecording => {
                let tail = match &mut self.state {
                    State::Recording(recorder) => recorder.finish(),
                    _ => return true,
                };
                self.state = State::Idle;
                for action in tail {
                    let item = ActionItem::new(self.take_id(), action);
                    self.emit(EngineEvent::Recorded(item));
                }
                self.to_win32(Win32Command::EnableHooks(false));
                self.emit(EngineEvent::RecordingStopped);
            }
            EngineCommand::Play { macro_, start_index } => {
                if !matches!(self.state, State::Idle) {
                    self.emit(EngineEvent::Error("already recording or playing".into()));
                    return true;
                }
                let ctl = PlayerControl::new();
                if macro_.settings.stop_on_user_input {
                    self.to_win32(Win32Command::PlaybackStarted(ctl.clone()));
                }
                self.emit(EngineEvent::PlaybackStarted { total: macro_.items.len() });
                let evt_tx = self.evt_tx.clone();
                let repaint = self.repaint.clone();
                let done_tx = self.done_tx.clone();
                let spawned = Player::spawn(
                    self.player_deps.clone(),
                    ctl.clone(),
                    macro_,
                    start_index,
                    Box::new(move |index, iteration| {
                        let _ = evt_tx.send(EngineEvent::PlaybackProgress { index, iteration });
                        repaint();
                    }),
                    Box::new(move |outcome| {
                        let _ = done_tx.send(outcome);
                    }),
                );
                match spawned {
                    Ok(thread) => self.state = State::Playing { ctl, thread },
                    Err(e) => {
                        self.to_win32(Win32Command::PlaybackStopped);
                        self.emit(EngineEvent::PlaybackFinished(PlaybackOutcome::Failed {
                            index: start_index,
                            error: format!("cannot start the playback thread: {e}"),
                        }));
                    }
                }
            }
            EngineCommand::StopPlayback => {
                if let State::Playing { ctl, .. } = &self.state {
                    ctl.request_stop();
                }
            }
            EngineCommand::SetHotkeys(cfg) => {
                self.chord_vks = cfg.chord_vks();
                self.to_win32(Win32Command::SetHotkeys(cfg));
            }
            EngineCommand::ShowOverlay(scene) => self.to_win32(Win32Command::OverlayShow(scene)),
            EngineCommand::HideOverlay => self.to_win32(Win32Command::OverlayHide),
            EngineCommand::Shutdown => return false,
        }
        true
    }

    fn raw(&mut self, event: RawInputEvent) {
        if let RawInputEvent::Hotkey(action) = event {
            self.emit(EngineEvent::HotkeyPressed(action));
            if let State::Playing { ctl, .. } = &self.state
                && matches!(action, HotkeyAction::Stop | HotkeyAction::TogglePlay)
            {
                ctl.request_stop();
            }
            return;
        }
        let actions = match &mut self.state {
            State::Recording(recorder) => recorder.feed(event),
            _ => return,
        };
        for action in actions {
            let item = ActionItem::new(self.take_id(), action);
            self.emit(EngineEvent::Recorded(item));
        }
    }

    fn playback_finished(&mut self, outcome: PlaybackOutcome) {
        if let State::Playing { thread, .. } = std::mem::replace(&mut self.state, State::Idle) {
            let _ = thread.join();
        }
        self.to_win32(Win32Command::PlaybackStopped);
        self.emit(EngineEvent::PlaybackFinished(outcome));
    }

    fn shutdown(&mut self) {
        match std::mem::replace(&mut self.state, State::Idle) {
            State::Playing { ctl, thread } => {
                ctl.request_stop();
                let _ = thread.join();
                self.to_win32(Win32Command::PlaybackStopped);
            }
            State::Recording(_) => self.to_win32(Win32Command::EnableHooks(false)),
            State::Idle => {}
        }
    }

    /// Provisional id for a recorded item; the GUI re-ids items when it appends them.
    fn take_id(&mut self) -> ActionId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn emit(&self, event: EngineEvent) {
        let _ = self.evt_tx.send(event);
        (self.repaint)();
    }

    fn to_win32(&self, cmd: Win32Command) {
        if self.win32_tx.send(cmd).is_err() {
            log::warn!("win32 service thread is gone");
        }
    }
}
