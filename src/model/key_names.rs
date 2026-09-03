use super::Key;

/// Well known Windows virtual-key codes used across the model and the UI.
pub mod vk {
    pub const BACK: u16 = 0x08;
    pub const TAB: u16 = 0x09;
    pub const RETURN: u16 = 0x0D;
    pub const SHIFT: u16 = 0x10;
    pub const CONTROL: u16 = 0x11;
    pub const MENU: u16 = 0x12;
    pub const PAUSE: u16 = 0x13;
    pub const CAPITAL: u16 = 0x14;
    pub const ESCAPE: u16 = 0x1B;
    pub const SPACE: u16 = 0x20;
    pub const PRIOR: u16 = 0x21;
    pub const NEXT: u16 = 0x22;
    pub const END: u16 = 0x23;
    pub const HOME: u16 = 0x24;
    pub const LEFT: u16 = 0x25;
    pub const UP: u16 = 0x26;
    pub const RIGHT: u16 = 0x27;
    pub const DOWN: u16 = 0x28;
    pub const SNAPSHOT: u16 = 0x2C;
    pub const INSERT: u16 = 0x2D;
    pub const DELETE: u16 = 0x2E;
    pub const LWIN: u16 = 0x5B;
    pub const RWIN: u16 = 0x5C;
    pub const APPS: u16 = 0x5D;
    pub const NUMPAD0: u16 = 0x60;
    pub const MULTIPLY: u16 = 0x6A;
    pub const ADD: u16 = 0x6B;
    pub const SUBTRACT: u16 = 0x6D;
    pub const DECIMAL: u16 = 0x6E;
    pub const DIVIDE: u16 = 0x6F;
    pub const F1: u16 = 0x70;
    pub const F24: u16 = 0x87;
    pub const NUMLOCK: u16 = 0x90;
    pub const SCROLL: u16 = 0x91;
    pub const LSHIFT: u16 = 0xA0;
    pub const RSHIFT: u16 = 0xA1;
    pub const LCONTROL: u16 = 0xA2;
    pub const RCONTROL: u16 = 0xA3;
    pub const LMENU: u16 = 0xA4;
    pub const RMENU: u16 = 0xA5;
}

impl Key {
    /// Key with only the virtual-key code set; the platform layer fills in the scan code.
    pub fn from_vk(vk: u16) -> Self {
        Self { vk, scancode: 0, extended: false }
    }

    /// Display name such as "F9", "Ctrl (right)" or "A".
    pub fn name(&self) -> String {
        vk_name(self.vk)
    }
}

/// Human readable name for a virtual-key code.
pub fn vk_name(code: u16) -> String {
    use vk::*;
    let fixed = match code {
        BACK => "Backspace",
        TAB => "Tab",
        RETURN => "Enter",
        SHIFT => "Shift",
        CONTROL => "Ctrl",
        MENU => "Alt",
        PAUSE => "Pause",
        CAPITAL => "Caps Lock",
        ESCAPE => "Esc",
        SPACE => "Space",
        PRIOR => "Page Up",
        NEXT => "Page Down",
        END => "End",
        HOME => "Home",
        LEFT => "Left",
        UP => "Up",
        RIGHT => "Right",
        DOWN => "Down",
        SNAPSHOT => "Print Screen",
        INSERT => "Insert",
        DELETE => "Delete",
        LWIN => "Win (left)",
        RWIN => "Win (right)",
        APPS => "Menu",
        MULTIPLY => "Num *",
        ADD => "Num +",
        SUBTRACT => "Num -",
        DECIMAL => "Num .",
        DIVIDE => "Num /",
        NUMLOCK => "Num Lock",
        SCROLL => "Scroll Lock",
        LSHIFT => "Shift (left)",
        RSHIFT => "Shift (right)",
        LCONTROL => "Ctrl (left)",
        RCONTROL => "Ctrl (right)",
        LMENU => "Alt (left)",
        RMENU => "Alt (right)",
        0xBA => ";",
        0xBB => "=",
        0xBC => ",",
        0xBD => "-",
        0xBE => ".",
        0xBF => "/",
        0xC0 => "`",
        0xDB => "[",
        0xDC => "\\",
        0xDD => "]",
        0xDE => "'",
        0xE2 => "<",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.to_string();
    }
    match code {
        0x30..=0x39 | 0x41..=0x5A => char::from(code as u8).to_string(),
        NUMPAD0..=0x69 => format!("Num {}", code - NUMPAD0),
        F1..=F24 => format!("F{}", code - F1 + 1),
        _ => format!("VK 0x{code:02X}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_cover_letters_digits_and_function_keys() {
        assert_eq!(vk_name(0x41), "A");
        assert_eq!(vk_name(0x37), "7");
        assert_eq!(vk_name(vk::F1 + 8), "F9");
        assert_eq!(vk_name(vk::NUMPAD0 + 5), "Num 5");
        assert_eq!(vk_name(vk::RCONTROL), "Ctrl (right)");
        assert_eq!(vk_name(0xFF), "VK 0xFF");
    }
}
