use std::thread::JoinHandle;

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender};

use crate::model::{EngineCommand, EngineEvent, RawInputEvent, Win32Command};

/// GUI-side handle: send commands, drain events once per frame.
pub struct EngineHandle {
    pub cmd_tx: Sender<EngineCommand>,
    pub evt_rx: Receiver<EngineEvent>,
    thread: Option<JoinHandle<()>>,
}

impl EngineHandle {
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
    pub repaint: Box<dyn Fn() + Send + Sync>,
}

/// Spawns the engine thread. Placeholder until recorder and player land: answers every command with an error.
pub fn spawn_engine(deps: EngineDeps) -> Result<EngineHandle> {
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<EngineCommand>();
    let (evt_tx, evt_rx) = crossbeam_channel::unbounded::<EngineEvent>();
    let thread = std::thread::Builder::new().name("engine".into()).spawn(move || {
        let EngineDeps { raw_rx, win32_tx, repaint } = deps;
        loop {
            crossbeam_channel::select! {
                recv(cmd_rx) -> cmd => match cmd {
                    Ok(EngineCommand::Shutdown) | Err(_) => break,
                    Ok(EngineCommand::ShowOverlay(scene)) => {
                        let _ = win32_tx.send(Win32Command::OverlayShow(scene));
                    }
                    Ok(EngineCommand::HideOverlay) => {
                        let _ = win32_tx.send(Win32Command::OverlayHide);
                    }
                    Ok(EngineCommand::SetHotkeys(cfg)) => {
                        let _ = win32_tx.send(Win32Command::SetHotkeys(cfg));
                    }
                    Ok(other) => {
                        let _ = evt_tx.send(EngineEvent::Error(format!("not implemented: {other:?}")));
                        repaint();
                    }
                },
                recv(raw_rx) -> raw => match raw {
                    Ok(RawInputEvent::Hotkey(action)) => {
                        let _ = evt_tx.send(EngineEvent::HotkeyPressed(action));
                        repaint();
                    }
                    Ok(_) => {}
                    Err(_) => break,
                },
            }
        }
    })?;
    Ok(EngineHandle { cmd_tx, evt_rx, thread: Some(thread) })
}
