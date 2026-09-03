use windows::Win32::Foundation::ERROR_HOTKEY_ALREADY_REGISTERED;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey,
};

use crate::model::{Hotkey, HotkeyAction, HotkeyConfig, modifiers, vk};

/// Base for the hotkey ids we hand to `RegisterHotKey`; must stay below 0xBFFF.
const ID_BASE: i32 = 0x4D52;

/// Owns the thread's `RegisterHotKey` registrations and the chords Windows refused.
#[derive(Default)]
pub struct HotkeyRegistry {
    registered: Vec<(i32, HotkeyAction)>,
    fallback: Vec<(Hotkey, HotkeyAction)>,
}

impl HotkeyRegistry {
    /// Replaces the registrations with `config`; returns the chords that must be matched in the hook.
    pub fn set(&mut self, config: &HotkeyConfig) -> Vec<(Hotkey, HotkeyAction)> {
        self.unregister_all();
        for (index, (action, hotkey)) in config.bindings().enumerate() {
            let id = ID_BASE + index as i32;
            let flags = win32_modifiers(hotkey.modifiers) | MOD_NOREPEAT;
            // SAFETY: registers against the calling thread's message queue, no pointers involved.
            match unsafe { RegisterHotKey(None, id, flags, hotkey.vk as u32) } {
                Ok(()) => self.registered.push((id, action)),
                Err(e) if e.code() == ERROR_HOTKEY_ALREADY_REGISTERED.to_hresult() => {
                    log::info!("hotkey {hotkey} is taken, matching it in the keyboard hook instead");
                    self.fallback.push((hotkey, action));
                }
                Err(e) => {
                    log::warn!("RegisterHotKey for {hotkey} failed: {e}");
                    self.fallback.push((hotkey, action));
                }
            }
        }
        self.fallback.clone()
    }

    pub fn action_for_id(&self, id: i32) -> Option<HotkeyAction> {
        self.registered.iter().find(|(i, _)| *i == id).map(|(_, a)| *a)
    }

    pub fn unregister_all(&mut self) {
        for (id, _) in self.registered.drain(..) {
            // SAFETY: unregisters an id this thread registered itself.
            if let Err(e) = unsafe { UnregisterHotKey(None, id) } {
                log::warn!("UnregisterHotKey({id}) failed: {e}");
            }
        }
        self.fallback.clear();
    }
}

fn win32_modifiers(flags: u8) -> HOT_KEY_MODIFIERS {
    let mut out = HOT_KEY_MODIFIERS(0);
    if flags & modifiers::ALT != 0 {
        out |= MOD_ALT;
    }
    if flags & modifiers::CONTROL != 0 {
        out |= MOD_CONTROL;
    }
    if flags & modifiers::SHIFT != 0 {
        out |= MOD_SHIFT;
    }
    if flags & modifiers::WIN != 0 {
        out |= MOD_WIN;
    }
    out
}

/// Virtual-key codes taking part in any configured chord, including its modifier keys.
/// The hooks use these to keep the play hotkey from triggering auto-stop.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chord_vks_cover_triggers_and_modifiers() {
        let config = HotkeyConfig {
            toggle_play: Some(Hotkey::new(modifiers::CONTROL | modifiers::SHIFT, 0x50)),
            ..Default::default()
        };
        let codes = chord_vks(&config);
        assert!(codes.contains(&0x50));
        assert!(codes.contains(&vk::LCONTROL));
        assert!(codes.contains(&vk::RSHIFT));
        assert!(codes.contains(&vk::ESCAPE));
        assert!(!codes.contains(&vk::LWIN));
    }

    #[test]
    fn modifier_translation_matches_win32_bits() {
        let all = modifiers::ALT | modifiers::CONTROL | modifiers::SHIFT | modifiers::WIN;
        assert_eq!(win32_modifiers(all), MOD_ALT | MOD_CONTROL | MOD_SHIFT | MOD_WIN);
        assert_eq!(win32_modifiers(0), HOT_KEY_MODIFIERS(0));
    }
}
