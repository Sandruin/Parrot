use std::ffi::c_void;

use anyhow::{Result, bail};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, w};

/// Whether the process runs with an elevated token, `None` when its token cannot be read.
pub fn process_is_elevated(pid: u32) -> Option<bool> {
    if pid == 0 {
        return None;
    }
    // SAFETY: the process handle is closed on every path below.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let elevated = token_is_elevated(process);
        let _ = CloseHandle(process);
        elevated
    }
}

/// Whether our own process runs elevated; `false` when the token cannot be read.
pub fn current_is_elevated() -> bool {
    // SAFETY: the pseudo handle of `GetCurrentProcess` needs no closing.
    unsafe { token_is_elevated(GetCurrentProcess()).unwrap_or(false) }
}

/// Whether the window's owning process runs elevated, `None` when that cannot be determined.
pub fn window_is_elevated(handle: isize) -> Option<bool> {
    let hwnd = HWND(handle as *mut c_void);
    if hwnd.0.is_null() {
        return None;
    }
    process_is_elevated(super::window::window_pid(hwnd))
}

/// Starts our own executable again through the `runas` verb, which raises the UAC prompt.
pub fn relaunch_elevated() -> Result<()> {
    let exe = std::env::current_exe()?;
    let path = HSTRING::from(exe.as_os_str());
    // SAFETY: both string arguments outlive the call and the shell copies what it needs.
    let result =
        unsafe { ShellExecuteW(None, w!("runas"), &path, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    if result.0 as usize <= 32 {
        bail!("ShellExecuteW(runas) failed: {}", windows::core::Error::from_thread());
    }
    Ok(())
}

/// Reads `TokenElevation` from the process token; the caller owns `process`.
unsafe fn token_is_elevated(process: HANDLE) -> Option<bool> {
    // SAFETY: the token handle is closed before returning and the output buffer is a live local.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
        let mut elevation = TOKEN_ELEVATION::default();
        let mut written = 0u32;
        let result = GetTokenInformation(
            token,
            TokenElevation,
            Some(&raw mut elevation as *mut c_void),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut written,
        );
        let _ = CloseHandle(token);
        result.ok()?;
        Some(elevation.TokenIsElevated != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_own_process_reports_a_definite_elevation_state() {
        let pid = super::super::window::own_pid();
        assert_eq!(process_is_elevated(pid), Some(current_is_elevated()));
    }

    #[test]
    fn pid_zero_has_no_token() {
        assert_eq!(process_is_elevated(0), None);
        assert_eq!(window_is_elevated(0), None);
    }
}
