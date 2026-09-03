use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::{Action, ActionId, ActionItem};

/// Format version written into every macro file; bump when the schema changes.
pub const CURRENT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Repeat {
    Count(u32),
    Infinite,
}

impl Default for Repeat {
    fn default() -> Self {
        Repeat::Count(1)
    }
}

/// How recorded `MouseMove` paths are replayed.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MousePathMode {
    /// Replay every recorded sample with its original timing.
    #[default]
    AsRecorded,
    /// Jump straight to the final point.
    Straight,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct MacroSettings {
    /// Playback speed in percent, 100 is real time.
    pub speed_percent: u32,
    pub repeat: Repeat,
    pub mouse_path: MousePathMode,
    /// Stop playback as soon as the user presses a key or mouse button.
    pub stop_on_user_input: bool,
}

impl Default for MacroSettings {
    fn default() -> Self {
        Self {
            speed_percent: 100,
            repeat: Repeat::default(),
            mouse_path: MousePathMode::default(),
            stop_on_user_input: true,
        }
    }
}

impl MacroSettings {
    /// Multiplier applied to durations; 200% speed halves every wait.
    pub fn speed_factor(&self) -> f64 {
        100.0 / self.speed_percent.max(1) as f64
    }
}

/// A saved macro: ordered actions plus playback settings.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Macro {
    pub version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub settings: MacroSettings,
    #[serde(default)]
    pub items: Vec<ActionItem>,
}

impl Default for Macro {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            name: String::new(),
            settings: MacroSettings::default(),
            items: Vec::new(),
        }
    }
}

impl Macro {
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("serializing macro")
    }

    pub fn from_json(json: &str) -> Result<Self> {
        let mut m: Macro = serde_json::from_str(json).context("parsing macro")?;
        m.migrate()?;
        Ok(m)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.to_json()?).with_context(|| format!("writing {}", path.display()))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let json = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::from_json(&json)
    }

    /// Next unused item id.
    pub fn next_id(&self) -> ActionId {
        self.items.iter().map(|i| i.id).max().map_or(1, |m| m + 1)
    }

    /// Appends a new item and returns its id.
    pub fn push(&mut self, action: Action) -> ActionId {
        let id = self.next_id();
        self.items.push(ActionItem::new(id, action));
        id
    }

    /// Inserts a new item at `index` (clamped to the list length) and returns its id.
    pub fn insert(&mut self, index: usize, action: Action) -> ActionId {
        let id = self.next_id();
        let index = index.min(self.items.len());
        self.items.insert(index, ActionItem::new(id, action));
        id
    }

    pub fn index_of(&self, id: ActionId) -> Option<usize> {
        self.items.iter().position(|i| i.id == id)
    }

    pub fn item(&self, id: ActionId) -> Option<&ActionItem> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn item_mut(&mut self, id: ActionId) -> Option<&mut ActionItem> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    pub fn remove(&mut self, id: ActionId) -> Option<ActionItem> {
        let idx = self.index_of(id)?;
        Some(self.items.remove(idx))
    }

    /// Duplicates the item directly after itself and returns the copy's id.
    pub fn duplicate(&mut self, id: ActionId) -> Option<ActionId> {
        let idx = self.index_of(id)?;
        let mut copy = self.items[idx].clone();
        copy.id = self.next_id();
        let new_id = copy.id;
        self.items.insert(idx + 1, copy);
        Some(new_id)
    }

    /// Moves the item by `delta` positions, clamped to the list bounds.
    pub fn shift(&mut self, id: ActionId, delta: isize) -> bool {
        let Some(idx) = self.index_of(id) else {
            return false;
        };
        let target = (idx as isize + delta).clamp(0, self.items.len() as isize - 1) as usize;
        if target == idx {
            return false;
        }
        let item = self.items.remove(idx);
        self.items.insert(target, item);
        true
    }

    fn migrate(&mut self) -> Result<()> {
        if self.version > CURRENT_VERSION {
            anyhow::bail!(
                "macro file version {} is newer than supported version {}",
                self.version,
                CURRENT_VERSION
            );
        }
        self.version = CURRENT_VERSION;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn every_action() -> Vec<Action> {
        let key = Key { vk: 0x41, scancode: 0x1E, extended: false };
        vec![
            Action::Wait { duration: 1.5, unit: TimeUnit::S },
            Action::KeyDown { key },
            Action::KeyUp { key },
            Action::KeyPress { key },
            Action::TypeText { text: "héllo".into(), mode: TextMode::Unicode, char_delay_ms: 10 },
            Action::MouseMove {
                path: vec![PathPoint { x: 1, y: 2, dt_ms: 0 }, PathPoint { x: 3, y: 4, dt_ms: 16 }],
            },
            Action::MouseButton {
                button: MouseButton::Right,
                event: ButtonEvent::Down,
                pos: Some(Point::new(10, 20)),
            },
            Action::MouseWheel { delta: -120, horizontal: false, pos: None },
            Action::WindowActivate {
                title_contains: "Notepad".into(),
                process_name: "notepad.exe".into(),
                timeout_ms: 5000,
            },
            Action::WaitForImage {
                region: Rect::new(0, 0, 4, 4),
                template_png: vec![0x89, b'P', b'N', b'G', 0, 255],
                similarity: 0.9,
                poll_ms: 250,
                timeout_ms: 10_000,
                mode: ImageMatchMode::Search,
            },
            Action::WaitForText {
                region: Rect::new(5, 5, 100, 20),
                text: "Ready".into(),
                case_sensitive: false,
                poll_ms: 500,
                timeout_ms: 0,
            },
            Action::ClickOnText {
                region: Rect::new(0, 0, 800, 600),
                text: "Start".into(),
                case_sensitive: false,
                button: MouseButton::Left,
                poll_ms: 250,
                timeout_ms: 5000,
            },
            Action::MouseMoveRelative {
                steps: vec![PathPoint { x: 10, y: -5, dt_ms: 0 }, PathPoint { x: 4, y: 0, dt_ms: 8 }],
                scale: 1.5,
            },
            Action::WaitForFile { path: "C:/out/*.png".into(), timeout_ms: 60_000 },
            Action::Comment { text: "note".into() },
            Action::Label { name: "start".into() },
        ]
    }

    #[test]
    fn round_trips_every_action_variant() {
        let mut m = Macro { name: "test".into(), ..Default::default() };
        for a in every_action() {
            m.push(a);
        }
        m.items[0].comment = "first".into();
        m.items[1].enabled = false;
        m.settings.repeat = Repeat::Infinite;
        let json = m.to_json().unwrap();
        let back = Macro::from_json(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn template_is_base64_in_json() {
        let mut m = Macro::default();
        m.push(Action::WaitForImage {
            region: Rect::default(),
            template_png: vec![1, 2, 3],
            similarity: 1.0,
            poll_ms: 1,
            timeout_ms: 1,
            mode: ImageMatchMode::Exact,
        });
        let json = m.to_json().unwrap();
        assert!(json.contains("\"template_png\": \"AQID\""));
    }

    #[test]
    fn missing_optional_fields_use_defaults() {
        let json = r#"{"version":1,"items":[{"id":7,"action":{"type":"label","name":"x"}}]}"#;
        let m = Macro::from_json(json).unwrap();
        assert_eq!(m.items[0].id, 7);
        assert!(m.items[0].enabled);
        assert_eq!(m.items[0].comment, "");
        assert_eq!(m.settings, MacroSettings::default());
    }

    #[test]
    fn rejects_newer_version() {
        let json = r#"{"version":99,"items":[]}"#;
        assert!(Macro::from_json(json).is_err());
    }

    #[test]
    fn list_editing_helpers() {
        let mut m = Macro::default();
        let a = m.push(Action::Label { name: "a".into() });
        let b = m.push(Action::Label { name: "b".into() });
        let c = m.push(Action::Label { name: "c".into() });
        assert!(m.shift(c, -2));
        assert_eq!(m.items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![c, a, b]);
        assert!(!m.shift(c, -1));
        let d = m.duplicate(a).unwrap();
        assert_eq!(m.index_of(d), Some(2));
        assert_eq!(m.item(d).unwrap().action, Action::Label { name: "a".into() });
        assert!(m.remove(b).is_some());
        assert_eq!(m.items.len(), 3);
        assert_eq!(m.next_id(), d + 1);
    }

    #[test]
    fn speed_factor_scales_inversely() {
        let s = MacroSettings { speed_percent: 200, ..Default::default() };
        assert_eq!(s.speed_factor(), 0.5);
        let zero = MacroSettings { speed_percent: 0, ..Default::default() };
        assert_eq!(zero.speed_factor(), 100.0);
    }
}
