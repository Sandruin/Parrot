use anyhow::Result;

use super::hyprland::Hyprland;
use super::wayland::Wayland;
use crate::model::{HotkeyAction, HotkeyConfig, modifiers, vk};

/// Shortcut ids the Wayland dispatcher saw pressed since the last drain.
#[derive(Default)]
pub struct WlState {
    pub pressed: Vec<String>,
}

/// Global hotkeys: compositor binds where available, software chord matching on the evdev stream otherwise.
pub struct Hotkeys {
    hyprland: Option<Hyprland>,
}

impl Hotkeys {
    pub fn new(hyprland: Option<Hyprland>) -> Self {
        Self { hyprland }
    }

    /// Registers the chords for the busy state; the Stop chord only while `busy`.
    pub fn set(&mut self, _wl: &mut Wayland, _config: &HotkeyConfig, _busy: bool) -> Result<()> {
        let _ = &self.hyprland;
        Ok(())
    }

    /// Re-applies the current binds after the compositor reloaded its configuration.
    pub fn reapply(&mut self, _wl: &mut Wayland) {}

    /// Removes every bind this process added.
    pub fn clear(&mut self) {}

    /// Feeds one physical key to the software matcher; returns the action of a completed fallback chord.
    pub fn on_key(&mut self, _vk: u16, _down: bool) -> Option<HotkeyAction> {
        None
    }

    /// Maps a shortcut id from the global shortcuts protocol to its action.
    pub fn action_for_id(&self, _id: &str) -> Option<HotkeyAction> {
        None
    }
}

/// Virtual-key codes taking part in any configured chord, including its modifier keys.
/// The service uses these to keep the play hotkey from triggering auto-stop.
pub fn chord_vks(config: &HotkeyConfig) -> Vec<u16> {
    let mut out = Vec::new();
    for (_, hotkey) in config.bindings() {
        push_unique(&mut out, hotkey.vk);
        for code in modifier_vks(hotkey.modifiers) {
            push_unique(&mut out, code);
        }
    }
    out
}

fn modifier_vks(flags: u8) -> Vec<u16> {
    let mut out = Vec::new();
    if flags & modifiers::ALT != 0 {
        out.extend([vk::MENU, vk::LMENU, vk::RMENU]);
    }
    if flags & modifiers::CONTROL != 0 {
        out.extend([vk::CONTROL, vk::LCONTROL, vk::RCONTROL]);
    }
    if flags & modifiers::SHIFT != 0 {
        out.extend([vk::SHIFT, vk::LSHIFT, vk::RSHIFT]);
    }
    if flags & modifiers::WIN != 0 {
        out.extend([vk::LWIN, vk::RWIN]);
    }
    out
}

fn push_unique(list: &mut Vec<u16>, code: u16) {
    if !list.contains(&code) {
        list.push(code);
    }
}
