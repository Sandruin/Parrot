pub mod mock;
mod traits;
#[cfg(windows)]
pub mod win32;

pub use traits::{InputInjector, ScreenCapture, Sleeper, WaitResult, WindowInfo, WindowManager, WindowRef};
