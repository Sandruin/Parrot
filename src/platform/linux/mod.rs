pub mod capture;
pub mod hotkeys;
pub mod hyprland;
pub mod injector;
pub mod input;
pub mod keymap;
pub mod keys;
pub mod layout;
pub mod ocr;
pub mod overlay;
pub mod protocols;
pub mod service;
pub mod wayland;
pub mod window;

use std::sync::Arc;

use anyhow::Result;
use crossbeam_channel::Sender;

pub use service::{LinuxHandle, spawn_linux_service};

use crate::model::RawInputEvent;
use crate::platform::PlatformServices;

/// Handle to the platform service thread.
pub type ServiceHandle = LinuxHandle;

/// Process-wide setup that must run before any window exists; nothing is needed on Linux.
pub fn init() {}

/// Starts the service thread that feeds raw input and handles [`crate::model::PlatformCommand`]s.
pub fn spawn_service(raw_tx: Sender<RawInputEvent>) -> Result<ServiceHandle> {
    spawn_linux_service(raw_tx)
}

/// The Wayland and Hyprland implementations of the platform traits.
pub fn services() -> Result<PlatformServices> {
    Ok(PlatformServices {
        injector: Arc::new(injector::WaylandInjector::new()?),
        capture: Arc::new(capture::WaylandCapture::new()?),
        windows: Arc::new(window::HyprlandWindows::new()),
        ocr: Arc::new(ocr::TesseractOcr::default()),
    })
}
