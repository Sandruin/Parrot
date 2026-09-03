use serde::{Deserialize, Serialize};

use super::key_names::vk_name;

/// Modifier bit flags matching the Win32 `MOD_*` constants.
pub mod modifiers {
    pub const ALT: u8 = 0x01;
    pub const CONTROL: u8 = 0x02;
    pub const SHIFT: u8 = 0x04;
    pub const WIN: u8 = 0x08;
}

/// A global key chord: modifier flags plus a virtual-key code.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hotkey {
    pub modifiers: u8,
    pub vk: u16,
}

impl Hotkey {
    pub const fn new(modifiers: u8, vk: u16) -> Self {
        Self { modifiers, vk }
    }

    pub fn has(&self, flag: u8) -> bool {
        self.modifiers & flag != 0
    }
}

impl std::fmt::Display for Hotkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.has(modifiers::CONTROL) {
            write!(f, "Ctrl+")?;
        }
        if self.has(modifiers::ALT) {
            write!(f, "Alt+")?;
        }
        if self.has(modifiers::SHIFT) {
            write!(f, "Shift+")?;
        }
        if self.has(modifiers::WIN) {
            write!(f, "Win+")?;
        }
        write!(f, "{}", vk_name(self.vk))
    }
}

/// What a registered hotkey triggers.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HotkeyAction {
    ToggleRecord,
    TogglePlay,
    Stop,
}

impl HotkeyAction {
    pub const ALL: [HotkeyAction; 3] =
        [HotkeyAction::ToggleRecord, HotkeyAction::TogglePlay, HotkeyAction::Stop];

    pub fn label(&self) -> &'static str {
        match self {
            HotkeyAction::ToggleRecord => "Start / stop recording",
            HotkeyAction::TogglePlay => "Start / stop playback",
            HotkeyAction::Stop => "Stop",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct HotkeyConfig {
    pub toggle_record: Option<Hotkey>,
    pub toggle_play: Option<Hotkey>,
    pub stop: Option<Hotkey>,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            toggle_record: Some(Hotkey::new(0, super::vk::F1 + 8)),
            toggle_play: Some(Hotkey::new(0, super::vk::F1 + 9)),
            stop: Some(Hotkey::new(0, super::vk::ESCAPE)),
        }
    }
}

impl HotkeyConfig {
    pub fn get(&self, action: HotkeyAction) -> Option<Hotkey> {
        match action {
            HotkeyAction::ToggleRecord => self.toggle_record,
            HotkeyAction::TogglePlay => self.toggle_play,
            HotkeyAction::Stop => self.stop,
        }
    }

    pub fn set(&mut self, action: HotkeyAction, hotkey: Option<Hotkey>) {
        match action {
            HotkeyAction::ToggleRecord => self.toggle_record = hotkey,
            HotkeyAction::TogglePlay => self.toggle_play = hotkey,
            HotkeyAction::Stop => self.stop = hotkey,
        }
    }

    /// All (action, hotkey) pairs that are configured.
    pub fn bindings(&self) -> impl Iterator<Item = (HotkeyAction, Hotkey)> + '_ {
        HotkeyAction::ALL.into_iter().filter_map(|a| self.get(a).map(|h| (a, h)))
    }

    /// Virtual-key codes that take part in any configured chord, used to filter them out of recordings.
    pub fn chord_vks(&self) -> Vec<u16> {
        self.bindings().map(|(_, h)| h.vk).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_lists_modifiers_in_fixed_order() {
        let h = Hotkey::new(modifiers::SHIFT | modifiers::CONTROL, 0x41);
        assert_eq!(h.to_string(), "Ctrl+Shift+A");
        assert_eq!(Hotkey::new(0, 0x70).to_string(), "F1");
    }

    #[test]
    fn defaults_are_f9_f10_esc() {
        let c = HotkeyConfig::default();
        assert_eq!(c.toggle_record.unwrap().to_string(), "F9");
        assert_eq!(c.toggle_play.unwrap().to_string(), "F10");
        assert_eq!(c.stop.unwrap().to_string(), "Esc");
        assert_eq!(c.bindings().count(), 3);
    }
}
