use anyhow::{Context as _, Result};
use xkbcommon::xkb;

use super::keys;
use crate::platform::CharKey;

/// First XKB keycode assigned to a unicode-keyboard slot; keycodes below 9 are reserved.
const UNICODE_KEYCODE_BASE: u32 = 9;

/// A compiled xkb keymap used to resolve layout-dependent keys and track modifier state.
pub struct Xkb {
    keymap: xkb::Keymap,
    state: xkb::State,
}

// SAFETY: the raw xkbcommon handles inside `Xkb` are only ever touched while the owning
// `WaylandInjector`'s mutex is held, so the library never sees concurrent access.
unsafe impl Send for Xkb {}

impl Xkb {
    /// Compiles the keymap text the compositor shared.
    pub fn new(text: &str) -> Result<Self> {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_string(
            &context,
            text.to_owned(),
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .context("compiling the xkb keymap")?;
        let state = xkb::State::new(&keymap);
        Ok(Self { keymap, state })
    }

    /// Windows virtual-key code for a letter or digit key on this layout, `None` for other keys.
    pub fn vk_for_keycode(&self, evdev: u16) -> Option<u16> {
        let keycode = xkb::Keycode::new(evdev as u32 + 8);
        let sym = *self.keymap.key_get_syms_by_level(keycode, 0, 0).first()?;
        keys::vk_for_keysym(sym.raw())
    }

    /// Evdev code of the key that produces `vk`'s letter or digit at level 0 on this layout.
    pub fn evdev_for_vk(&self, vk: u16) -> Option<u16> {
        let ch = match vk {
            0x30..=0x39 => vk as u32,
            0x41..=0x5A => vk as u32 + 0x20,
            _ => return None,
        };
        let (keycode, _) = self.keycode_for_keysym(xkb::Keysym::new(ch), &[0])?;
        Some((keycode.raw() - 8) as u16)
    }

    /// Key plus shift state that produces `ch` on this layout, `None` when it needs AltGr or is unmapped.
    pub fn key_for_char(&self, ch: char) -> Option<CharKey> {
        let target = xkb::utf32_to_keysym(ch as u32);
        if target.raw() == 0 {
            return None;
        }
        let (keycode, level) = self.keycode_for_keysym(target, &[0, 1])?;
        let evdev = (keycode.raw() - 8) as u16;
        let mut key = keys::key_from_evdev(evdev)?;
        if let Some(vk) = keys::vk_for_keysym(target.raw()) {
            key.vk = vk;
        }
        Some(CharKey { key, shift: level == 1, ctrl: false, alt: false })
    }

    /// Feeds one physical key event into the tracked modifier state and returns the depressed,
    /// latched, locked modifier masks and the effective group, ready for a `modifiers` request.
    pub fn update_key(&mut self, evdev: u16, down: bool) -> (u32, u32, u32, u32) {
        let keycode = xkb::Keycode::new(evdev as u32 + 8);
        let direction = if down { xkb::KeyDirection::Down } else { xkb::KeyDirection::Up };
        self.state.update_key(keycode, direction);
        (
            self.state.serialize_mods(xkb::STATE_MODS_DEPRESSED),
            self.state.serialize_mods(xkb::STATE_MODS_LATCHED),
            self.state.serialize_mods(xkb::STATE_MODS_LOCKED),
            self.state.serialize_layout(xkb::STATE_LAYOUT_EFFECTIVE),
        )
    }

    /// The first keycode and shift level, among `levels`, whose layout-group-0 keysym is `keysym`.
    fn keycode_for_keysym(
        &self,
        keysym: xkb::Keysym,
        levels: &[xkb::LevelIndex],
    ) -> Option<(xkb::Keycode, xkb::LevelIndex)> {
        let min = self.keymap.min_keycode().raw();
        let max = self.keymap.max_keycode().raw();
        for raw in min..=max {
            let keycode = xkb::Keycode::new(raw);
            for &level in levels {
                if self.keymap.key_get_syms_by_level(keycode, 0, level).contains(&keysym) {
                    return Some((keycode, level));
                }
            }
        }
        None
    }

    /// Keymap text for the unicode virtual keyboard: one `ONE_LEVEL` key per assigned slot,
    /// formatted the way wtype does so plain xkbcommon can compile it.
    pub fn unicode_keymap_text(slots: &[Option<char>]) -> String {
        let mut keycodes = String::new();
        let mut symbols = String::new();
        for (i, slot) in slots.iter().enumerate() {
            let Some(ch) = slot else { continue };
            let code = UNICODE_KEYCODE_BASE + i as u32;
            keycodes.push_str(&format!("<I{i}> = {code};\n"));
            symbols.push_str(&format!("key <I{i}> {{ [ U{:04X} ] }};\n", *ch as u32));
        }
        format!(
            "xkb_keymap {{\n\
             xkb_keycodes \"(unnamed)\" {{\n\
             minimum = 8;\n\
             maximum = 255;\n\
             {keycodes}\
             }};\n\
             xkb_types \"(unnamed)\" {{\n\
             type \"ONE_LEVEL\" {{\n\
             modifiers = none;\n\
             level_name[Level1] = \"Any\";\n\
             }};\n\
             }};\n\
             xkb_compatibility \"(unnamed)\" {{\n\
             }};\n\
             xkb_symbols \"(unnamed)\" {{\n\
             {symbols}\
             }};\n\
             }};\n"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named_keymap(layout: &str) -> String {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_names(
            &context,
            "evdev",
            "pc105",
            layout,
            "",
            None,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .expect("compiling a named keymap");
        keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1)
    }

    #[test]
    fn vk_for_keycode_follows_the_layout() {
        let us = Xkb::new(&named_keymap("us")).unwrap();
        // evdev KEY_A = 30, KEY_Y = 21, KEY_Z = 44.
        assert_eq!(us.vk_for_keycode(30), Some(b'A' as u16));
        assert_eq!(us.vk_for_keycode(21), Some(b'Y' as u16));

        let de = Xkb::new(&named_keymap("de")).unwrap();
        // On the German QWERTZ layout the "Y position" key (evdev 21) types Z and vice versa.
        assert_eq!(de.vk_for_keycode(21), Some(b'Z' as u16));
        assert_eq!(de.vk_for_keycode(44), Some(b'Y' as u16));
    }

    #[test]
    fn evdev_for_vk_is_the_inverse_of_vk_for_keycode() {
        let de = Xkb::new(&named_keymap("de")).unwrap();
        assert_eq!(de.evdev_for_vk(b'Y' as u16), Some(44));
        assert_eq!(de.evdev_for_vk(b'Z' as u16), Some(21));
        assert_eq!(de.evdev_for_vk(0xFF), None);
    }

    #[test]
    fn key_for_char_reports_shift_and_falls_back_for_altgr() {
        let us = Xkb::new(&named_keymap("us")).unwrap();
        let lower = us.key_for_char('a').unwrap();
        assert!(!lower.shift && !lower.ctrl && !lower.alt);
        assert_eq!(lower.key.vk, b'A' as u16);
        let upper = us.key_for_char('A').unwrap();
        assert!(upper.shift);
        assert_eq!(upper.key.vk, b'A' as u16);
        assert_eq!(lower.key.scancode, upper.key.scancode);

        let de = Xkb::new(&named_keymap("de")).unwrap();
        let euro = de.key_for_char('€');
        assert!(euro.is_none(), "AltGr-only characters should fall back to unicode()");
    }

    #[test]
    fn update_key_reports_shift_as_depressed_while_held() {
        let mut us = Xkb::new(&named_keymap("us")).unwrap();
        let shift = keys::evdev_from_vk(crate::model::vk::SHIFT).unwrap();
        let (shift_mask, ..) = us.update_key(shift, true);
        assert_ne!(shift_mask, 0, "shift should be depressed as soon as it is pressed");

        let evdev_a = keys::evdev_from_vk(b'A' as u16).unwrap();
        let (depressed, ..) = us.update_key(evdev_a, true);
        assert_eq!(depressed, shift_mask, "shift should still be depressed while held");
        us.update_key(evdev_a, false);

        let (depressed, ..) = us.update_key(shift, false);
        assert_eq!(depressed, 0, "releasing shift should clear the depressed mask");
    }

    #[test]
    fn unicode_keymap_text_compiles_and_names_slots() {
        let empty = Xkb::unicode_keymap_text(&[]);
        assert!(Xkb::new(&empty).is_ok());

        let slots = vec![Some('a'), None, Some('é'), Some('😀')];
        let text = Xkb::unicode_keymap_text(&slots);
        assert!(text.contains("U0061"));
        assert!(text.contains("U00E9"));
        assert!(text.contains("U1F600"));
        assert!(Xkb::new(&text).is_ok(), "generated unicode keymap must compile:\n{text}");
    }
}
