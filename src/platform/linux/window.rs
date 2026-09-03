use std::time::Duration;

use anyhow::{Result, bail};

use super::hyprland::Hyprland;
use crate::platform::{WindowInfo, WindowManager, WindowRef};

/// Finds and focuses windows through the Hyprland IPC socket.
pub struct HyprlandWindows {
    hyprland: Option<Hyprland>,
}

impl HyprlandWindows {
    pub fn new() -> Self {
        Self { hyprland: Hyprland::detect() }
    }
}

impl Default for HyprlandWindows {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowManager for HyprlandWindows {
    fn find(&self, _title_contains: &str, _process_name: &str) -> Option<WindowRef> {
        let _ = &self.hyprland;
        None
    }

    fn activate(&self, _window: WindowRef, _timeout: Duration) -> Result<()> {
        bail!("window activation is not implemented yet")
    }

    fn foreground(&self) -> Option<WindowInfo> {
        None
    }
}
