use egui::{Key, Modifiers};

use crate::model::{modifiers, vk};

/// Windows virtual-key code for an egui key, `None` for keys without a stable mapping.
pub fn vk_from_key(key: Key) -> Option<u16> {
    let code = match key {
        Key::ArrowDown => vk::DOWN,
        Key::ArrowLeft => vk::LEFT,
        Key::ArrowRight => vk::RIGHT,
        Key::ArrowUp => vk::UP,
        Key::Escape => vk::ESCAPE,
        Key::Tab => vk::TAB,
        Key::Backspace => vk::BACK,
        Key::Enter => vk::RETURN,
        Key::Space => vk::SPACE,
        Key::Insert => vk::INSERT,
        Key::Delete => vk::DELETE,
        Key::Home => vk::HOME,
        Key::End => vk::END,
        Key::PageUp => vk::PRIOR,
        Key::PageDown => vk::NEXT,
        Key::Comma => 0xBC,
        Key::Backslash => 0xDC,
        Key::Slash => 0xBF,
        Key::OpenBracket => 0xDB,
        Key::CloseBracket => 0xDD,
        Key::Backtick => 0xC0,
        Key::Minus => 0xBD,
        Key::Period => 0xBE,
        Key::Plus => vk::ADD,
        Key::Equals => 0xBB,
        Key::Semicolon => 0xBA,
        Key::Colon => 0xBA,
        Key::Quote => 0xDE,
        Key::IntlBackslash => 0xE2,
        Key::Num0 => 0x30,
        Key::Num1 => 0x31,
        Key::Num2 => 0x32,
        Key::Num3 => 0x33,
        Key::Num4 => 0x34,
        Key::Num5 => 0x35,
        Key::Num6 => 0x36,
        Key::Num7 => 0x37,
        Key::Num8 => 0x38,
        Key::Num9 => 0x39,
        Key::A => 0x41,
        Key::B => 0x42,
        Key::C => 0x43,
        Key::D => 0x44,
        Key::E => 0x45,
        Key::F => 0x46,
        Key::G => 0x47,
        Key::H => 0x48,
        Key::I => 0x49,
        Key::J => 0x4A,
        Key::K => 0x4B,
        Key::L => 0x4C,
        Key::M => 0x4D,
        Key::N => 0x4E,
        Key::O => 0x4F,
        Key::P => 0x50,
        Key::Q => 0x51,
        Key::R => 0x52,
        Key::S => 0x53,
        Key::T => 0x54,
        Key::U => 0x55,
        Key::V => 0x56,
        Key::W => 0x57,
        Key::X => 0x58,
        Key::Y => 0x59,
        Key::Z => 0x5A,
        Key::ShiftLeft => vk::LSHIFT,
        Key::ShiftRight => vk::RSHIFT,
        Key::ControlLeft => vk::LCONTROL,
        Key::ControlRight => vk::RCONTROL,
        Key::AltLeft => vk::LMENU,
        Key::AltRight => vk::RMENU,
        Key::SuperLeft => vk::LWIN,
        Key::SuperRight => vk::RWIN,
        _ => return function_vk(key),
    };
    Some(code)
}

/// True for the physical modifier keys, which a hotkey chord takes from the modifier flags instead.
pub fn is_modifier_key(key: Key) -> bool {
    matches!(
        key,
        Key::ShiftLeft
            | Key::ShiftRight
            | Key::ControlLeft
            | Key::ControlRight
            | Key::AltLeft
            | Key::AltRight
            | Key::SuperLeft
            | Key::SuperRight
    )
}

/// Win32 `MOD_*` flags for the modifiers egui reports.
pub fn modifier_flags(m: Modifiers) -> u8 {
    let mut flags = 0;
    if m.ctrl || m.command {
        flags |= modifiers::CONTROL;
    }
    if m.alt {
        flags |= modifiers::ALT;
    }
    if m.shift {
        flags |= modifiers::SHIFT;
    }
    flags
}

fn function_vk(key: Key) -> Option<u16> {
    let index = match key {
        Key::F1 => 0,
        Key::F2 => 1,
        Key::F3 => 2,
        Key::F4 => 3,
        Key::F5 => 4,
        Key::F6 => 5,
        Key::F7 => 6,
        Key::F8 => 7,
        Key::F9 => 8,
        Key::F10 => 9,
        Key::F11 => 10,
        Key::F12 => 11,
        Key::F13 => 12,
        Key::F14 => 13,
        Key::F15 => 14,
        Key::F16 => 15,
        Key::F17 => 16,
        Key::F18 => 17,
        Key::F19 => 18,
        Key::F20 => 19,
        Key::F21 => 20,
        Key::F22 => 21,
        Key::F23 => 22,
        Key::F24 => 23,
        _ => return None,
    };
    Some(vk::F1 + index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::vk_name;

    #[test]
    fn maps_letters_digits_and_function_keys() {
        assert_eq!(vk_from_key(Key::A).map(vk_name).as_deref(), Some("A"));
        assert_eq!(vk_from_key(Key::Num7).map(vk_name).as_deref(), Some("7"));
        assert_eq!(vk_from_key(Key::F9), Some(vk::F1 + 8));
        assert_eq!(vk_from_key(Key::F24), Some(vk::F24));
        assert_eq!(vk_from_key(Key::Escape), Some(vk::ESCAPE));
    }

    #[test]
    fn unmapped_keys_return_none() {
        assert_eq!(vk_from_key(Key::Copy), None);
        assert_eq!(vk_from_key(Key::BrowserBack), None);
    }

    #[test]
    fn modifier_flags_follow_egui_modifiers() {
        let m = Modifiers { ctrl: true, shift: true, ..Default::default() };
        assert_eq!(modifier_flags(m), modifiers::CONTROL | modifiers::SHIFT);
        assert!(is_modifier_key(Key::ControlLeft));
        assert!(!is_modifier_key(Key::A));
    }
}
