use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use crossbeam_channel::Sender;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, LLKHF_UP, LLMHF_INJECTED,
    MSLLHOOKSTRUCT, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_XBUTTONDOWN, WM_XBUTTONUP, XBUTTON1,
};

use super::injector::MAGIC;
use super::keys;
use crate::model::{
    Hotkey, HotkeyAction, Key, MouseButton, PlayerControl, Point, RawInputEvent, modifiers, vk,
};

/// Foreign input is ignored for this long after playback starts, so the play hotkey does not stop it.
pub const AUTO_STOP_GRACE: Duration = Duration::from_millis(300);

const NO_VK: u32 = u32::MAX;

static HOOK_CTX: OnceLock<HookCtx> = OnceLock::new();

struct Playback {
    ctl: Arc<PlayerControl>,
    start: Instant,
}

/// State the hook procedures read; installed once per process by the service thread.
pub struct HookCtx {
    tx: Sender<RawInputEvent>,
    forward_moves: AtomicBool,
    playing: AtomicBool,
    playback: Mutex<Option<Playback>>,
    /// Virtual-key codes that belong to a configured hotkey chord and never trigger auto-stop.
    chord_vks: Mutex<Vec<u16>>,
    fallback_active: AtomicBool,
    fallback: Mutex<Vec<(Hotkey, HotkeyAction)>>,
    /// Modifier flags as seen by the keyboard hook, for matching fallback chords.
    mods: AtomicU8,
    /// Trigger key whose release must be swallowed together with its fallback chord.
    swallowed_vk: AtomicU32,
}

/// Publishes the hook context; fails if a service thread already installed one in this process.
pub fn init(tx: Sender<RawInputEvent>) -> Result<&'static HookCtx> {
    let mut installed = false;
    let ctx = HOOK_CTX.get_or_init(|| {
        installed = true;
        HookCtx {
            tx,
            forward_moves: AtomicBool::new(true),
            playing: AtomicBool::new(false),
            playback: Mutex::new(None),
            chord_vks: Mutex::new(Vec::new()),
            fallback_active: AtomicBool::new(false),
            fallback: Mutex::new(Vec::new()),
            mods: AtomicU8::new(0),
            swallowed_vk: AtomicU32::new(NO_VK),
        }
    });
    if !installed {
        bail!("the win32 service thread can only be spawned once per process");
    }
    Ok(ctx)
}

pub fn ctx() -> Option<&'static HookCtx> {
    HOOK_CTX.get()
}

impl HookCtx {
    /// Mouse `Move` events are only forwarded while this is true; every other event always is.
    pub fn set_forward_moves(&self, forward: bool) {
        self.forward_moves.store(forward, Ordering::Relaxed);
    }

    pub fn set_hotkeys(&self, chord_vks: Vec<u16>, fallback: Vec<(Hotkey, HotkeyAction)>) {
        *self.chord_vks.lock().unwrap_or_else(|e| e.into_inner()) = chord_vks;
        self.fallback_active.store(!fallback.is_empty(), Ordering::Relaxed);
        *self.fallback.lock().unwrap_or_else(|e| e.into_inner()) = fallback;
    }

    /// Arms auto-stop: foreign key or button presses interrupt this player.
    pub fn arm_playback(&self, ctl: Arc<PlayerControl>) {
        *self.playback.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(Playback { ctl, start: Instant::now() });
        self.playing.store(true, Ordering::SeqCst);
    }

    pub fn disarm_playback(&self) {
        self.playing.store(false, Ordering::SeqCst);
        *self.playback.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub fn send(&self, event: RawInputEvent) {
        let _ = self.tx.send(event);
    }

    fn on_key(&self, info: &KBDLLHOOKSTRUCT) -> bool {
        let vk = info.vkCode as u16;
        let down = !info.flags.contains(LLKHF_UP);
        let own = info.dwExtraInfo == MAGIC;
        let mut key =
            Key { vk, scancode: info.scanCode as u16, extended: info.flags.contains(LLKHF_EXTENDED) };
        if key.scancode == 0 {
            let mapped = keys::key_from_vk(vk);
            key.scancode = mapped.scancode;
            key.extended |= mapped.extended;
        }

        if !own {
            self.track_modifier(vk, down);
            if self.handle_fallback(vk, down) {
                return true;
            }
            if down {
                self.maybe_interrupt(Some(vk));
            }
        }

        self.send(RawInputEvent::Key {
            key,
            down,
            injected: info.flags.contains(LLKHF_INJECTED),
            own,
            at: Instant::now(),
        });
        false
    }

    fn on_mouse(&self, msg: u32, info: &MSLLHOOKSTRUCT) {
        let own = info.dwExtraInfo == MAGIC;
        let injected = info.flags & LLMHF_INJECTED != 0;
        let pos = Point::new(info.pt.x, info.pt.y);
        let at = Instant::now();
        let high = (info.mouseData >> 16) as u16;

        let button =
            |button: MouseButton, down: bool| RawInputEvent::Button { button, down, pos, injected, own, at };
        let event = match msg {
            WM_MOUSEMOVE => {
                if !self.forward_moves.load(Ordering::Relaxed) {
                    return;
                }
                RawInputEvent::Move { pos, injected, own, at }
            }
            WM_LBUTTONDOWN => button(MouseButton::Left, true),
            WM_LBUTTONUP => button(MouseButton::Left, false),
            WM_RBUTTONDOWN => button(MouseButton::Right, true),
            WM_RBUTTONUP => button(MouseButton::Right, false),
            WM_MBUTTONDOWN => button(MouseButton::Middle, true),
            WM_MBUTTONUP => button(MouseButton::Middle, false),
            WM_XBUTTONDOWN | WM_XBUTTONUP => {
                let which = if high == XBUTTON1 { MouseButton::X1 } else { MouseButton::X2 };
                button(which, msg == WM_XBUTTONDOWN)
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => RawInputEvent::Wheel {
                delta: high as i16 as i32,
                horizontal: msg == WM_MOUSEHWHEEL,
                pos,
                injected,
                own,
                at,
            },
            _ => return,
        };

        if !own && matches!(event, RawInputEvent::Button { down: true, .. }) {
            self.maybe_interrupt(None);
        }
        self.send(event);
    }

    /// Stops a running player unless we are still inside the grace period or the key is part of a chord.
    fn maybe_interrupt(&self, trigger_vk: Option<u16>) {
        if !self.playing.load(Ordering::Relaxed) {
            return;
        }
        if let Some(code) = trigger_vk
            && self.chord_vks.lock().unwrap_or_else(|e| e.into_inner()).contains(&code)
        {
            return;
        }
        let guard = self.playback.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(playback) = guard.as_ref()
            && playback.start.elapsed() >= AUTO_STOP_GRACE
        {
            playback.ctl.interrupt();
        }
    }

    fn track_modifier(&self, code: u16, down: bool) {
        let flag = match code {
            vk::SHIFT | vk::LSHIFT | vk::RSHIFT => modifiers::SHIFT,
            vk::CONTROL | vk::LCONTROL | vk::RCONTROL => modifiers::CONTROL,
            vk::MENU | vk::LMENU | vk::RMENU => modifiers::ALT,
            vk::LWIN | vk::RWIN => modifiers::WIN,
            _ => return,
        };
        let mut current = self.mods.load(Ordering::Relaxed);
        loop {
            let next = if down { current | flag } else { current & !flag };
            match self.mods.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return,
                Err(seen) => current = seen,
            }
        }
    }

    /// True when the event belongs to an in-hook hotkey chord and must not reach other applications.
    fn handle_fallback(&self, code: u16, down: bool) -> bool {
        if !self.fallback_active.load(Ordering::Relaxed) {
            return false;
        }
        if !down {
            return self
                .swallowed_vk
                .compare_exchange(code as u32, NO_VK, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok();
        }
        let pressed = self.mods.load(Ordering::Relaxed);
        let matched = {
            let list = self.fallback.lock().unwrap_or_else(|e| e.into_inner());
            list.iter().find(|(h, _)| h.vk == code && h.modifiers == pressed).map(|(_, a)| *a)
        };
        match matched {
            Some(action) => {
                self.swallowed_vk.store(code as u32, Ordering::Relaxed);
                self.send(RawInputEvent::Hotkey(action));
                true
            }
            None => false,
        }
    }
}

pub(crate) unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for HC_ACTION the system guarantees lparam points at a live KBDLLHOOKSTRUCT.
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if let Some(ctx) = HOOK_CTX.get()
            && ctx.on_key(info)
        {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

pub(crate) unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for HC_ACTION the system guarantees lparam points at a live MSLLHOOKSTRUCT.
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        if let Some(ctx) = HOOK_CTX.get() {
            ctx.on_mouse(wparam.0 as u32, info);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
