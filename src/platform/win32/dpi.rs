use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};

/// Makes every coordinate the process sees a physical pixel; a no-op when the manifest already did it.
pub fn ensure_per_monitor_v2() {
    // SAFETY: plain Win32 call with a constant argument, safe to call any time before windows exist.
    if let Err(e) = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } {
        log::debug!("SetProcessDpiAwarenessContext: {e} (already set by manifest?)");
    }
}
