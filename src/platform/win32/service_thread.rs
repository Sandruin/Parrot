use std::thread::JoinHandle;

use anyhow::{Context, Result, bail};
use crossbeam_channel::{Receiver, Sender};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_SYSTEM_FOREGROUND, GetMessageW, HHOOK, MSG, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP, WM_HOTKEY,
};

use super::{hooks, hotkeys, overlay, winevent};
use crate::model::{OverlayScene, PlatformCommand, RawInputEvent};

/// Posted to the service thread so the `GetMessageW` loop wakes up and drains the command channel.
const WM_DRAIN: u32 = WM_APP + 1;

/// Handle to the Win32 service thread that hosts the hooks, hotkeys and the overlay window.
pub struct Win32Handle {
    cmd_tx: Sender<PlatformCommand>,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
    relay: Option<JoinHandle<()>>,
}

impl Win32Handle {
    /// Clone of the command sender for threads that must not own the handle; sends wake the loop too.
    pub fn cmd_sender(&self) -> Sender<PlatformCommand> {
        self.cmd_tx.clone()
    }

    pub fn send(&self, cmd: PlatformCommand) {
        if self.cmd_tx.send(cmd).is_err() {
            log::warn!("platform service thread is gone");
            return;
        }
        wake(self.thread_id);
    }

    /// Thread that owns the hooks, for callers that need to post their own messages to it.
    pub fn thread_id(&self) -> u32 {
        self.thread_id
    }
}

impl Drop for Win32Handle {
    fn drop(&mut self) {
        self.send(PlatformCommand::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if let Some(t) = self.relay.take() {
            let _ = t.join();
        }
    }
}

/// Starts the service thread: low-level hooks, the foreground WinEvent hook, hotkeys and a message loop.
pub fn spawn_win32_service(raw_tx: Sender<RawInputEvent>) -> Result<Win32Handle> {
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<PlatformCommand>();
    let (loop_tx, loop_rx) = crossbeam_channel::unbounded::<PlatformCommand>();
    let (ready_tx, ready_rx) = crossbeam_channel::bounded::<Result<u32, String>>(1);

    let thread = std::thread::Builder::new().name("win32".into()).spawn(move || {
        match Service::install(raw_tx) {
            Ok(mut service) => {
                // SAFETY: plain Win32 call without arguments.
                let _ = ready_tx.send(Ok(unsafe { GetCurrentThreadId() }));
                service.run(&loop_rx);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(format!("{e:#}")));
            }
        }
    })?;

    let thread_id = match ready_rx.recv() {
        Ok(Ok(id)) => id,
        Ok(Err(e)) => {
            let _ = thread.join();
            bail!("win32 service thread failed to start: {e}");
        }
        Err(_) => {
            let _ = thread.join();
            bail!("win32 service thread died during startup");
        }
    };

    let relay = std::thread::Builder::new().name("win32-wake".into()).spawn(move || {
        while let Ok(cmd) = cmd_rx.recv() {
            let last = matches!(cmd, PlatformCommand::Shutdown);
            if loop_tx.send(cmd).is_err() {
                break;
            }
            wake(thread_id);
            if last {
                break;
            }
        }
    })?;

    Ok(Win32Handle { cmd_tx, thread_id, thread: Some(thread), relay: Some(relay) })
}

fn wake(thread_id: u32) {
    // SAFETY: posts a private message to a thread that owns a message queue for as long as it runs.
    if let Err(e) = unsafe { PostThreadMessageW(thread_id, WM_DRAIN, WPARAM(0), LPARAM(0)) } {
        log::debug!("PostThreadMessageW to the win32 thread failed: {e}");
    }
}

/// Hooks and registrations owned by the service thread; unwound in `Drop`.
struct Service {
    keyboard: HHOOK,
    mouse: HHOOK,
    winevent: HWINEVENTHOOK,
    hotkeys: hotkeys::HotkeyRegistry,
    /// Last configuration received, re-applied whenever the busy state flips.
    hotkey_config: crate::model::HotkeyConfig,
    overlay: OverlayState,
}

/// Overlay lifecycle: the window is created on demand and stays hidden while recording or playing.
#[derive(Default)]
struct OverlayState {
    window: Option<overlay::Overlay>,
    /// Last scene the GUI asked for, cleared only by `OverlayHide`.
    scene: Option<OverlayScene>,
    playing: bool,
    recording: bool,
}

impl OverlayState {
    fn show(&mut self, scene: OverlayScene) {
        self.scene = Some(scene);
        self.refresh();
    }

    fn hide(&mut self) {
        self.scene = None;
        if let Some(window) = self.window.as_mut() {
            window.hide();
        }
    }

    fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
        self.refresh();
    }

    fn set_recording(&mut self, recording: bool) {
        self.recording = recording;
        self.refresh();
    }

    /// Draws the remembered scene, or hides the window while recording or playback suppresses it.
    fn refresh(&mut self) {
        if self.playing || self.recording || self.scene.is_none() {
            if let Some(window) = self.window.as_mut() {
                window.hide();
            }
            return;
        }
        if self.window.is_none() {
            match overlay::Overlay::new() {
                Ok(window) => self.window = Some(window),
                Err(e) => {
                    log::error!("the overlay window could not be created: {e:#}");
                    return;
                }
            }
        }
        let (Some(window), Some(scene)) = (self.window.as_mut(), self.scene.as_ref()) else {
            return;
        };
        if let Err(e) = window.show(scene) {
            log::error!("drawing the overlay failed: {e:#}");
        }
    }
}

impl Service {
    fn install(raw_tx: Sender<RawInputEvent>) -> Result<Self> {
        // SAFETY: raises this thread's priority so the OS does not time out the hook callbacks.
        unsafe {
            if let Err(e) = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL) {
                log::warn!("SetThreadPriority failed: {e}");
            }
        }
        hooks::init(raw_tx)?;

        // SAFETY: global low-level hooks with `None` module and thread 0, procedures live for the process.
        let keyboard = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hooks::keyboard_proc), None, 0) }
            .context("SetWindowsHookExW(WH_KEYBOARD_LL)")?;
        let mouse = match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(hooks::mouse_proc), None, 0) } {
            Ok(hook) => hook,
            Err(e) => {
                // SAFETY: undoes the keyboard hook installed a moment ago.
                unsafe { UnhookWindowsHookEx(keyboard) }.ok();
                return Err(e).context("SetWindowsHookExW(WH_MOUSE_LL)");
            }
        };
        // SAFETY: out-of-context WinEvent hook, the callback runs on this thread's message loop.
        let winevent = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(winevent::win_event_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        if winevent.is_invalid() {
            log::warn!("SetWinEventHook(EVENT_SYSTEM_FOREGROUND) failed, window changes are not tracked");
        }
        log::info!("win32 service thread ready");
        Ok(Self {
            keyboard,
            mouse,
            winevent,
            hotkeys: hotkeys::HotkeyRegistry::default(),
            hotkey_config: crate::model::HotkeyConfig::default(),
            overlay: OverlayState::default(),
        })
    }

    fn run(&mut self, cmd_rx: &Receiver<PlatformCommand>) {
        let mut msg = MSG::default();
        loop {
            if !self.drain(cmd_rx) {
                break;
            }
            // SAFETY: `msg` is a live local; blocks until a message arrives for this thread.
            let result = unsafe { GetMessageW(&mut msg, None, 0, 0) }.0;
            if result == -1 {
                log::error!("GetMessageW failed: {}", windows::core::Error::from_thread());
                break;
            }
            if result == 0 {
                break;
            }
            match msg.message {
                WM_DRAIN => {}
                WM_HOTKEY => {
                    if let Some(action) = self.hotkeys.action_for_id(msg.wParam.0 as i32)
                        && let Some(ctx) = hooks::ctx()
                    {
                        ctx.send(RawInputEvent::Hotkey(action));
                    }
                }
                // SAFETY: forwards anything else, which is how the overlay window gets its messages.
                _ => unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                },
            }
        }
    }

    /// Handles every queued command; returns false when the service must shut down.
    fn drain(&mut self, cmd_rx: &Receiver<PlatformCommand>) -> bool {
        loop {
            match cmd_rx.try_recv() {
                Ok(PlatformCommand::Shutdown) => return false,
                Ok(cmd) => self.handle(cmd),
                Err(crossbeam_channel::TryRecvError::Empty) => return true,
                Err(crossbeam_channel::TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn handle(&mut self, cmd: PlatformCommand) {
        let Some(ctx) = hooks::ctx() else { return };
        match cmd {
            PlatformCommand::EnableHooks(enabled) => {
                ctx.set_forward_moves(enabled);
                self.overlay.set_recording(enabled);
                self.apply_hotkeys(ctx);
            }
            PlatformCommand::SetHotkeys(config) => {
                self.hotkey_config = config;
                self.apply_hotkeys(ctx);
            }
            PlatformCommand::PlaybackStarted(control) => {
                ctx.arm_playback(control);
                self.overlay.set_playing(true);
                self.apply_hotkeys(ctx);
            }
            PlatformCommand::PlaybackStopped => {
                ctx.disarm_playback();
                self.overlay.set_playing(false);
                self.apply_hotkeys(ctx);
            }
            PlatformCommand::OverlayShow(scene) => self.overlay.show(scene),
            PlatformCommand::OverlayHide => self.overlay.hide(),
            PlatformCommand::Shutdown => {}
        }
    }

    /// Re-registers the chords for the current busy state and hands refused ones to the hook.
    fn apply_hotkeys(&mut self, ctx: &hooks::HookCtx) {
        let busy = self.overlay.playing || self.overlay.recording;
        let fallback = self.hotkeys.set(&self.hotkey_config, busy);
        ctx.set_hotkeys(hotkeys::chord_vks(&self.hotkey_config), fallback);
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.overlay.window = None;
        self.hotkeys.unregister_all();
        // SAFETY: releases the hooks this thread installed; each handle is used once.
        unsafe {
            if !self.winevent.is_invalid() {
                let _ = UnhookWinEvent(self.winevent);
            }
            UnhookWindowsHookEx(self.mouse).ok();
            UnhookWindowsHookEx(self.keyboard).ok();
        }
        log::info!("win32 service thread stopped");
    }
}
