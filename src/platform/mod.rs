#[cfg(target_os = "linux")]
pub mod linux;
pub mod mock;
pub mod overlay_render;
pub mod sleeper;
mod traits;
#[cfg(windows)]
pub mod win32;

/// The backend for the operating system this build targets; every backend exposes the same entry points.
#[cfg(target_os = "linux")]
pub use linux as native;
#[cfg(windows)]
pub use win32 as native;

pub use traits::{
    CharKey, InputInjector, Ocr, OcrLine, OcrWord, PlatformServices, ScreenCapture, Sleeper, WaitResult,
    WindowInfo, WindowManager, WindowRef,
};
