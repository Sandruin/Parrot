use std::time::Duration;

use anyhow::{Result, bail};

use crate::platform::{WindowInfo, WindowManager, WindowRef};

/// Top-level window lookup and activation. Placeholder until the platform implementation lands.
#[derive(Default)]
pub struct Win32Windows;

impl WindowManager for Win32Windows {
    fn find(&self, _title_contains: &str, _process_name: &str) -> Option<WindowRef> {
        None
    }

    fn activate(&self, _window: WindowRef, _timeout: Duration) -> Result<()> {
        bail!("window activation not implemented yet")
    }

    fn foreground(&self) -> Option<WindowInfo> {
        None
    }
}
