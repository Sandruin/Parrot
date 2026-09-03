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

pub use service_thread::{Win32Handle, spawn_win32_service};
