use windows::Win32::UI::Input::KeyboardAndMouse::{MAPVK_VK_TO_VSC_EX, MapVirtualKeyW};

use crate::model::{Key, vk};

/// High byte of an extended scan code, as returned by `MAPVK_VK_TO_VSC_EX`.
pub const PREFIX_E0: u32 = 0xE0;
/// High byte of the Pause scan-code sequence, which `SendInput` cannot express as a scan code.
pub const PREFIX_E1: u32 = 0xE1;

/// Keys that need `KEYEVENTF_EXTENDEDKEY` even though `MAPVK_VK_TO_VSC_EX` omits the 0xE0 prefix.
/// Without it their scan codes address the numeric keypad instead: 0x52 is Num 0, not Insert.
const ALWAYS_EXTENDED: &[u16] = &[
    vk::PRIOR,
    vk::NEXT,
    vk::END,
    vk::HOME,
    vk::LEFT,
    vk::UP,
    vk::RIGHT,
    vk::DOWN,
    vk::INSERT,
    vk::DELETE,
    vk::NUMLOCK,
    vk::SNAPSHOT,
];

/// Print Screen maps to the SysReq code 0x54, which applications ignore; the real key is 0xE037.
const SNAPSHOT_SCANCODE: u16 = 0x37;

/// Raw `MAPVK_VK_TO_VSC_EX` result: scan code in the low byte, `0xE0` or `0xE1` prefix in the high byte.
pub fn scancode_ex(code: u16) -> u32 {
    // SAFETY: pure lookup against the active keyboard layout, no pointers involved.
    unsafe { MapVirtualKeyW(code as u32, MAPVK_VK_TO_VSC_EX) }
}

/// Builds a `Key` from a virtual-key code, filling in scan code and the extended flag.
pub fn key_from_vk(code: u16) -> Key {
    let mapped = scancode_ex(code);
    let scancode = if code == vk::SNAPSHOT { SNAPSHOT_SCANCODE } else { (mapped & 0xFF) as u16 };
    let extended = mapped >> 8 == PREFIX_E0 || ALWAYS_EXTENDED.contains(&code);
    Key { vk: code, scancode, extended }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_key_table_is_marked_extended() {
        let extended = [
            (vk::RCONTROL, 0x1D),
            (vk::RMENU, 0x38),
            (vk::INSERT, 0x52),
            (vk::DELETE, 0x53),
            (vk::HOME, 0x47),
            (vk::END, 0x4F),
            (vk::PRIOR, 0x49),
            (vk::NEXT, 0x51),
            (vk::LEFT, 0x4B),
            (vk::UP, 0x48),
            (vk::RIGHT, 0x4D),
            (vk::DOWN, 0x50),
            (vk::NUMLOCK, 0x45),
            (vk::DIVIDE, 0x35),
            (vk::LWIN, 0x5B),
            (vk::RWIN, 0x5C),
            (vk::APPS, 0x5D),
            (vk::SNAPSHOT, SNAPSHOT_SCANCODE),
        ];
        for (code, scancode) in extended {
            let key = key_from_vk(code);
            assert!(key.extended, "vk 0x{code:02X} should be extended, got {key:?}");
            assert_eq!(key.scancode, scancode, "vk 0x{code:02X} scan code");
        }
    }

    #[test]
    fn left_and_right_modifiers_map_to_different_codes() {
        let pairs = [
            (vk::LSHIFT, 0x2A, vk::RSHIFT, 0x36),
            (vk::LCONTROL, 0x1D, vk::RCONTROL, 0x1D),
            (vk::LMENU, 0x38, vk::RMENU, 0x38),
        ];
        for (left, left_scan, right, right_scan) in pairs {
            let l = key_from_vk(left);
            let r = key_from_vk(right);
            assert!(!l.extended, "vk 0x{left:02X} should not be extended");
            assert_eq!(l.scancode, left_scan);
            assert_eq!(r.scancode, right_scan);
            assert!(
                (l.scancode, l.extended) != (r.scancode, r.extended),
                "0x{left:02X} and 0x{right:02X} are indistinguishable"
            );
        }
    }

    #[test]
    fn pause_uses_the_e1_prefix() {
        assert_eq!(scancode_ex(vk::PAUSE) >> 8, PREFIX_E1);
    }

    #[test]
    fn plain_keys_are_not_extended() {
        for code in [0x41u16, vk::SPACE, vk::RETURN, vk::F1, vk::SCROLL, vk::NUMPAD0] {
            let key = key_from_vk(code);
            assert!(!key.extended, "vk 0x{code:02X} should not be extended");
            assert_ne!(key.scancode, 0);
        }
    }
}
