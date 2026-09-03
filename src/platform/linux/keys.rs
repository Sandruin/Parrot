use evdev::KeyCode;

use crate::model::{Key, vk};

/// Position table: evdev key code, the Windows virtual-key code of the US layout, set 1 scan code, E0 prefix.
/// Letters and digits get their layout-dependent virtual key from xkb at runtime; this holds the US default.
const TABLE: &[(KeyCode, u16, u16, bool)] = &[
    (KeyCode::KEY_ESC, vk::ESCAPE, 0x01, false),
    (KeyCode::KEY_1, 0x31, 0x02, false),
    (KeyCode::KEY_2, 0x32, 0x03, false),
    (KeyCode::KEY_3, 0x33, 0x04, false),
    (KeyCode::KEY_4, 0x34, 0x05, false),
    (KeyCode::KEY_5, 0x35, 0x06, false),
    (KeyCode::KEY_6, 0x36, 0x07, false),
    (KeyCode::KEY_7, 0x37, 0x08, false),
    (KeyCode::KEY_8, 0x38, 0x09, false),
    (KeyCode::KEY_9, 0x39, 0x0A, false),
    (KeyCode::KEY_0, 0x30, 0x0B, false),
    (KeyCode::KEY_MINUS, 0xBD, 0x0C, false),
    (KeyCode::KEY_EQUAL, 0xBB, 0x0D, false),
    (KeyCode::KEY_BACKSPACE, vk::BACK, 0x0E, false),
    (KeyCode::KEY_TAB, vk::TAB, 0x0F, false),
    (KeyCode::KEY_Q, 0x51, 0x10, false),
    (KeyCode::KEY_W, 0x57, 0x11, false),
    (KeyCode::KEY_E, 0x45, 0x12, false),
    (KeyCode::KEY_R, 0x52, 0x13, false),
    (KeyCode::KEY_T, 0x54, 0x14, false),
    (KeyCode::KEY_Y, 0x59, 0x15, false),
    (KeyCode::KEY_U, 0x55, 0x16, false),
    (KeyCode::KEY_I, 0x49, 0x17, false),
    (KeyCode::KEY_O, 0x4F, 0x18, false),
    (KeyCode::KEY_P, 0x50, 0x19, false),
    (KeyCode::KEY_LEFTBRACE, 0xDB, 0x1A, false),
    (KeyCode::KEY_RIGHTBRACE, 0xDD, 0x1B, false),
    (KeyCode::KEY_ENTER, vk::RETURN, 0x1C, false),
    (KeyCode::KEY_LEFTCTRL, vk::LCONTROL, 0x1D, false),
    (KeyCode::KEY_A, 0x41, 0x1E, false),
    (KeyCode::KEY_S, 0x53, 0x1F, false),
    (KeyCode::KEY_D, 0x44, 0x20, false),
    (KeyCode::KEY_F, 0x46, 0x21, false),
    (KeyCode::KEY_G, 0x47, 0x22, false),
    (KeyCode::KEY_H, 0x48, 0x23, false),
    (KeyCode::KEY_J, 0x4A, 0x24, false),
    (KeyCode::KEY_K, 0x4B, 0x25, false),
    (KeyCode::KEY_L, 0x4C, 0x26, false),
    (KeyCode::KEY_SEMICOLON, 0xBA, 0x27, false),
    (KeyCode::KEY_APOSTROPHE, 0xDE, 0x28, false),
    (KeyCode::KEY_GRAVE, 0xC0, 0x29, false),
    (KeyCode::KEY_LEFTSHIFT, vk::LSHIFT, 0x2A, false),
    (KeyCode::KEY_BACKSLASH, 0xDC, 0x2B, false),
    (KeyCode::KEY_Z, 0x5A, 0x2C, false),
    (KeyCode::KEY_X, 0x58, 0x2D, false),
    (KeyCode::KEY_C, 0x43, 0x2E, false),
    (KeyCode::KEY_V, 0x56, 0x2F, false),
    (KeyCode::KEY_B, 0x42, 0x30, false),
    (KeyCode::KEY_N, 0x4E, 0x31, false),
    (KeyCode::KEY_M, 0x4D, 0x32, false),
    (KeyCode::KEY_COMMA, 0xBC, 0x33, false),
    (KeyCode::KEY_DOT, 0xBE, 0x34, false),
    (KeyCode::KEY_SLASH, 0xBF, 0x35, false),
    (KeyCode::KEY_RIGHTSHIFT, vk::RSHIFT, 0x36, false),
    (KeyCode::KEY_KPASTERISK, vk::MULTIPLY, 0x37, false),
    (KeyCode::KEY_LEFTALT, vk::LMENU, 0x38, false),
    (KeyCode::KEY_SPACE, vk::SPACE, 0x39, false),
    (KeyCode::KEY_CAPSLOCK, vk::CAPITAL, 0x3A, false),
    (KeyCode::KEY_F1, vk::F1, 0x3B, false),
    (KeyCode::KEY_F2, vk::F1 + 1, 0x3C, false),
    (KeyCode::KEY_F3, vk::F1 + 2, 0x3D, false),
    (KeyCode::KEY_F4, vk::F1 + 3, 0x3E, false),
    (KeyCode::KEY_F5, vk::F1 + 4, 0x3F, false),
    (KeyCode::KEY_F6, vk::F1 + 5, 0x40, false),
    (KeyCode::KEY_F7, vk::F1 + 6, 0x41, false),
    (KeyCode::KEY_F8, vk::F1 + 7, 0x42, false),
    (KeyCode::KEY_F9, vk::F1 + 8, 0x43, false),
    (KeyCode::KEY_F10, vk::F1 + 9, 0x44, false),
    (KeyCode::KEY_NUMLOCK, vk::NUMLOCK, 0x45, true),
    (KeyCode::KEY_SCROLLLOCK, vk::SCROLL, 0x46, false),
    (KeyCode::KEY_KP7, vk::NUMPAD0 + 7, 0x47, false),
    (KeyCode::KEY_KP8, vk::NUMPAD0 + 8, 0x48, false),
    (KeyCode::KEY_KP9, vk::NUMPAD0 + 9, 0x49, false),
    (KeyCode::KEY_KPMINUS, vk::SUBTRACT, 0x4A, false),
    (KeyCode::KEY_KP4, vk::NUMPAD0 + 4, 0x4B, false),
    (KeyCode::KEY_KP5, vk::NUMPAD0 + 5, 0x4C, false),
    (KeyCode::KEY_KP6, vk::NUMPAD0 + 6, 0x4D, false),
    (KeyCode::KEY_KPPLUS, vk::ADD, 0x4E, false),
    (KeyCode::KEY_KP1, vk::NUMPAD0 + 1, 0x4F, false),
    (KeyCode::KEY_KP2, vk::NUMPAD0 + 2, 0x50, false),
    (KeyCode::KEY_KP3, vk::NUMPAD0 + 3, 0x51, false),
    (KeyCode::KEY_KP0, vk::NUMPAD0, 0x52, false),
    (KeyCode::KEY_KPDOT, vk::DECIMAL, 0x53, false),
    (KeyCode::KEY_102ND, 0xE2, 0x56, false),
    (KeyCode::KEY_F11, vk::F1 + 10, 0x57, false),
    (KeyCode::KEY_F12, vk::F1 + 11, 0x58, false),
    (KeyCode::KEY_F13, vk::F1 + 12, 0x64, false),
    (KeyCode::KEY_F14, vk::F1 + 13, 0x65, false),
    (KeyCode::KEY_F15, vk::F1 + 14, 0x66, false),
    (KeyCode::KEY_F16, vk::F1 + 15, 0x67, false),
    (KeyCode::KEY_F17, vk::F1 + 16, 0x68, false),
    (KeyCode::KEY_F18, vk::F1 + 17, 0x69, false),
    (KeyCode::KEY_F19, vk::F1 + 18, 0x6A, false),
    (KeyCode::KEY_F20, vk::F1 + 19, 0x6B, false),
    (KeyCode::KEY_F21, vk::F1 + 20, 0x6C, false),
    (KeyCode::KEY_F22, vk::F1 + 21, 0x6D, false),
    (KeyCode::KEY_F23, vk::F1 + 22, 0x6E, false),
    (KeyCode::KEY_F24, vk::F24, 0x76, false),
    (KeyCode::KEY_KPENTER, vk::RETURN, 0x1C, true),
    (KeyCode::KEY_RIGHTCTRL, vk::RCONTROL, 0x1D, true),
    (KeyCode::KEY_KPSLASH, vk::DIVIDE, 0x35, true),
    (KeyCode::KEY_SYSRQ, vk::SNAPSHOT, 0x37, true),
    (KeyCode::KEY_RIGHTALT, vk::RMENU, 0x38, true),
    (KeyCode::KEY_HOME, vk::HOME, 0x47, true),
    (KeyCode::KEY_UP, vk::UP, 0x48, true),
    (KeyCode::KEY_PAGEUP, vk::PRIOR, 0x49, true),
    (KeyCode::KEY_LEFT, vk::LEFT, 0x4B, true),
    (KeyCode::KEY_RIGHT, vk::RIGHT, 0x4D, true),
    (KeyCode::KEY_END, vk::END, 0x4F, true),
    (KeyCode::KEY_DOWN, vk::DOWN, 0x50, true),
    (KeyCode::KEY_PAGEDOWN, vk::NEXT, 0x51, true),
    (KeyCode::KEY_INSERT, vk::INSERT, 0x52, true),
    (KeyCode::KEY_DELETE, vk::DELETE, 0x53, true),
    (KeyCode::KEY_PAUSE, vk::PAUSE, 0x45, false),
    (KeyCode::KEY_LEFTMETA, vk::LWIN, 0x5B, true),
    (KeyCode::KEY_RIGHTMETA, vk::RWIN, 0x5C, true),
    (KeyCode::KEY_COMPOSE, vk::APPS, 0x5D, true),
    (KeyCode::KEY_MUTE, 0xAD, 0x20, true),
    (KeyCode::KEY_VOLUMEDOWN, 0xAE, 0x2E, true),
    (KeyCode::KEY_VOLUMEUP, 0xAF, 0x30, true),
    (KeyCode::KEY_NEXTSONG, 0xB0, 0x19, true),
    (KeyCode::KEY_PREVIOUSSONG, 0xB1, 0x10, true),
    (KeyCode::KEY_STOPCD, 0xB2, 0x24, true),
    (KeyCode::KEY_PLAYPAUSE, 0xB3, 0x22, true),
    (KeyCode::KEY_HOMEPAGE, 0xAC, 0x32, true),
    (KeyCode::KEY_BACK, 0xA6, 0x6A, true),
    (KeyCode::KEY_FORWARD, 0xA7, 0x69, true),
    (KeyCode::KEY_MAIL, 0xB4, 0x6C, true),
    (KeyCode::KEY_CALC, 0xB7, 0x21, true),
    (KeyCode::KEY_SLEEP, 0x5F, 0x5F, true),
];

/// Side-neutral modifier codes the player uses, resolved to the left-hand key.
const GENERIC_MODIFIERS: &[(u16, KeyCode)] = &[
    (vk::SHIFT, KeyCode::KEY_LEFTSHIFT),
    (vk::CONTROL, KeyCode::KEY_LEFTCTRL),
    (vk::MENU, KeyCode::KEY_LEFTALT),
];

/// The `Key` a physical evdev key records as, `None` for keys Windows has no code for.
pub fn key_from_evdev(code: u16) -> Option<Key> {
    TABLE.iter().find(|(k, ..)| k.code() == code).map(|&(_, vk, scancode, extended)| Key {
        vk,
        scancode,
        extended,
    })
}

/// The evdev code to press for a recorded key, preferring its position over its virtual-key code.
pub fn evdev_from_key(key: Key) -> Option<u16> {
    if key.scancode != 0 {
        let exact = TABLE.iter().find(|&&(_, _, sc, ext)| sc == key.scancode && ext == key.extended);
        if let Some((k, ..)) = exact.or_else(|| TABLE.iter().find(|&&(_, _, sc, _)| sc == key.scancode)) {
            return Some(k.code());
        }
    }
    evdev_from_vk(key.vk)
}

/// The evdev code for a virtual-key code on the US layout.
pub fn evdev_from_vk(vk: u16) -> Option<u16> {
    if let Some((_, k)) = GENERIC_MODIFIERS.iter().find(|(v, _)| *v == vk) {
        return Some(k.code());
    }
    TABLE.iter().find(|(_, v, ..)| *v == vk).map(|(k, ..)| k.code())
}

/// Fills in scan code and extended flag for a virtual-key code, like the Win32 backend does.
pub fn key_from_vk(vk: u16) -> Key {
    evdev_from_vk(vk).and_then(key_from_evdev).map(|k| Key { vk, ..k }).unwrap_or_else(|| Key::from_vk(vk))
}

/// Virtual-key code for a letter or digit keysym, which is where Windows codes follow the layout.
pub fn vk_for_keysym(keysym: u32) -> Option<u16> {
    match keysym {
        0x30..=0x39 => Some(keysym as u16),
        0x41..=0x5A => Some(keysym as u16),
        0x61..=0x7A => Some((keysym - 0x20) as u16),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn evdev_codes_and_scan_codes_are_unique() {
        let mut codes = HashSet::new();
        let mut scans = HashSet::new();
        for &(k, _, sc, ext) in TABLE {
            assert!(codes.insert(k.code()), "duplicate evdev code {k:?}");
            assert!(scans.insert((sc, ext)), "duplicate scan code {sc:#x} ext {ext} for {k:?}");
        }
    }

    #[test]
    fn matches_the_windows_extended_key_table() {
        let expected = [
            (KeyCode::KEY_RIGHTCTRL, vk::RCONTROL, 0x1D, true),
            (KeyCode::KEY_INSERT, vk::INSERT, 0x52, true),
            (KeyCode::KEY_KP0, vk::NUMPAD0, 0x52, false),
            (KeyCode::KEY_NUMLOCK, vk::NUMLOCK, 0x45, true),
            (KeyCode::KEY_PAUSE, vk::PAUSE, 0x45, false),
            (KeyCode::KEY_SYSRQ, vk::SNAPSHOT, 0x37, true),
            (KeyCode::KEY_LEFTMETA, vk::LWIN, 0x5B, true),
        ];
        for (code, vk, sc, ext) in expected {
            assert_eq!(
                key_from_evdev(code.code()),
                Some(Key { vk, scancode: sc, extended: ext }),
                "{code:?}"
            );
            assert_eq!(
                evdev_from_key(Key { vk, scancode: sc, extended: ext }),
                Some(code.code()),
                "{code:?}"
            );
        }
    }

    #[test]
    fn position_wins_over_virtual_key_when_both_are_set() {
        let german_y = Key { vk: 0x59, scancode: 0x2C, extended: false };
        assert_eq!(evdev_from_key(german_y), Some(KeyCode::KEY_Z.code()));
        assert_eq!(evdev_from_key(Key::from_vk(0x59)), Some(KeyCode::KEY_Y.code()));
        assert_eq!(evdev_from_key(Key::from_vk(vk::SHIFT)), Some(KeyCode::KEY_LEFTSHIFT.code()));
        assert_eq!(evdev_from_key(Key::from_vk(vk::RETURN)), Some(KeyCode::KEY_ENTER.code()));
        assert_eq!(evdev_from_key(Key::from_vk(0xFF)), None);
    }

    #[test]
    fn key_from_vk_fills_the_scan_code() {
        assert_eq!(key_from_vk(vk::F1 + 8), Key { vk: vk::F1 + 8, scancode: 0x43, extended: false });
        assert_eq!(key_from_vk(vk::MENU), Key { vk: vk::MENU, scancode: 0x38, extended: false });
        assert_eq!(key_from_vk(0xFF), Key::from_vk(0xFF));
        assert_eq!(vk_for_keysym('y' as u32), Some(0x59));
        assert_eq!(vk_for_keysym('7' as u32), Some(0x37));
        assert_eq!(vk_for_keysym(0xFF1B), None);
    }
}
