pub mod capture;
pub mod dpi;
pub mod elevation;
pub mod hooks;
pub mod hotkeys;
pub mod injector;
pub mod keys;
pub mod ocr;
pub mod overlay;
pub mod service_thread;
pub mod window;
pub mod winevent;

use std::sync::Arc;

use anyhow::Result;
use crossbeam_channel::Sender;

pub use service_thread::{Win32Handle, spawn_win32_service};

use crate::model::RawInputEvent;
use crate::platform::PlatformServices;

/// Handle to the platform service thread.
pub type ServiceHandle = Win32Handle;

/// Process-wide setup that must run before any window exists.
pub fn init() {
    dpi::ensure_per_monitor_v2();
}

/// Starts the service thread that feeds raw input and handles [`crate::model::PlatformCommand`]s.
pub fn spawn_service(raw_tx: Sender<RawInputEvent>) -> Result<ServiceHandle> {
    spawn_win32_service(raw_tx)
}

/// The Win32 implementations of the platform traits.
pub fn services() -> Result<PlatformServices> {
    Ok(PlatformServices {
        injector: Arc::new(injector::Win32Injector),
        capture: Arc::new(capture::Win32Capture),
        windows: Arc::new(window::Win32Windows),
        ocr: Arc::new(ocr::Win32Ocr),
    })
}
