pub mod mock;
pub mod sleeper;
mod traits;
#[cfg(windows)]
pub mod win32;

pub use traits::{
    CharKey, InputInjector, Ocr, OcrLine, OcrWord, ScreenCapture, Sleeper, WaitResult, WindowInfo,
    WindowManager, WindowRef,
};
