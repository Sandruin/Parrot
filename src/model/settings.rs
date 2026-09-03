use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::{HotkeyConfig, RecordOptions};

const MAX_RECENT: usize = 10;

/// Per-user application settings persisted in the config directory.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct AppSettings {
    pub hotkeys: HotkeyConfig,
    pub record: RecordOptions,
    pub recent_files: Vec<PathBuf>,
    /// Draw the overlay when an action is selected in the list.
    pub show_overlay: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            hotkeys: HotkeyConfig::default(),
            record: RecordOptions::default(),
            recent_files: Vec::new(),
            show_overlay: true,
        }
    }
}

impl AppSettings {
    /// Settings file location, `%APPDATA%/macro-recorder/settings.json` on Windows.
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("macro-recorder").join("settings.json"))
    }

    /// Loads settings from the default path, falling back to defaults on any error.
    pub fn load_or_default() -> Self {
        Self::default_path().and_then(|p| Self::load(&p).ok()).unwrap_or_default()
    }

    pub fn load(path: &Path) -> Result<Self> {
        let json = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&json).context("parsing settings")
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let json = serde_json::to_string_pretty(self).context("serializing settings")?;
        std::fs::write(path, json).with_context(|| format!("writing {}", path.display()))
    }

    pub fn save_default(&self) -> Result<()> {
        match Self::default_path() {
            Some(p) => self.save(&p),
            None => anyhow::bail!("no config directory available"),
        }
    }

    /// Moves `path` to the front of the recent list, dropping duplicates and old entries.
    pub fn push_recent(&mut self, path: PathBuf) {
        self.recent_files.retain(|p| p != &path);
        self.recent_files.insert(0, path);
        self.recent_files.truncate(MAX_RECENT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_files_dedupe_and_cap() {
        let mut s = AppSettings::default();
        for i in 0..12 {
            s.push_recent(PathBuf::from(format!("m{i}.json")));
        }
        s.push_recent(PathBuf::from("m5.json"));
        assert_eq!(s.recent_files.len(), MAX_RECENT);
        assert_eq!(s.recent_files[0], PathBuf::from("m5.json"));
        assert_eq!(s.recent_files.iter().filter(|p| p.as_os_str() == "m5.json").count(), 1);
    }

    #[test]
    fn settings_round_trip() {
        let s = AppSettings::default();
        let json = serde_json::to_string(&s).unwrap();
        let back: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
