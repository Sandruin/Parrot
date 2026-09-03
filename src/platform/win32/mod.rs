pub mod dpi;

use std::thread::JoinHandle;

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender};

use crate::model::{RawInputEvent, Win32Command};

/// Handle to the Win32 service thread that hosts hooks, hotkeys and the overlay.
pub struct Win32Handle {
    cmd_tx: Sender<Win32Command>,
    thread: Option<JoinHandle<()>>,
}

impl Win32Handle {
    /// Clone of the command sender for threads that must not own the handle.
    pub fn cmd_sender(&self) -> Sender<Win32Command> {
        self.cmd_tx.clone()
    }

    pub fn send(&self, cmd: Win32Command) {
        if self.cmd_tx.send(cmd).is_err() {
            log::warn!("win32 service thread is gone");
        }
    }
}

impl Drop for Win32Handle {
    fn drop(&mut self) {
        self.send(Win32Command::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Spawns the service thread. Placeholder until the hook implementation lands: drains commands only.
pub fn spawn_win32_service(_raw_tx: Sender<RawInputEvent>) -> Result<Win32Handle> {
    let (cmd_tx, cmd_rx): (Sender<Win32Command>, Receiver<Win32Command>) = crossbeam_channel::unbounded();
    let thread = std::thread::Builder::new().name("win32-service".into()).spawn(move || {
        while let Ok(cmd) = cmd_rx.recv() {
            if matches!(cmd, Win32Command::Shutdown) {
                break;
            }
            log::debug!("win32 command ignored by placeholder: {cmd:?}");
        }
    })?;
    Ok(Win32Handle { cmd_tx, thread: Some(thread) })
}
