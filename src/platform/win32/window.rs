use std::ffi::c_void;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, TRUE};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GWL_EXSTYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_RESTORE, SetForegroundWindow,
    ShowWindow, SwitchToThisWindow, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, PWSTR};

use super::injector::Win32Injector;
use super::keys;
use crate::model::vk;
use crate::platform::{InputInjector, WindowInfo, WindowManager, WindowRef};

/// Time spent polling `GetForegroundWindow` before the next escalation step.
const STEP_TIMEOUT: Duration = Duration::from_millis(200);
const POLL_INTERVAL: Duration = Duration::from_millis(10);
/// Alt tap, `AttachThreadInput` and `SwitchToThisWindow`, tried in that order after a plain attempt.
const ESCALATIONS: u32 = 3;

/// Finds and activates top-level windows.
#[derive(Default)]
pub struct Win32Windows;

impl WindowManager for Win32Windows {
    fn find(&self, title_contains: &str, process_name: &str) -> Option<WindowRef> {
        let mut state = FindState {
            title_contains: title_contains.to_owned(),
            process_name: process_name.to_owned(),
            own_pid: own_pid(),
            found: None,
        };
        // SAFETY: the callback only touches the `FindState` we pass by pointer and lives for the call.
        let _ = unsafe { EnumWindows(Some(enum_proc), LPARAM(&mut state as *mut FindState as isize)) };
        state.found
    }

    fn activate(&self, window: WindowRef, timeout: Duration) -> Result<()> {
        let hwnd = hwnd_of(window);
        let deadline = Instant::now() + timeout;
        // SAFETY: plain Win32 calls on a handle the caller obtained from `find` or `foreground`.
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
        }
        for step in 0..=ESCALATIONS {
            // SAFETY: activation request for a top-level window handle.
            unsafe {
                let _ = SetForegroundWindow(hwnd);
            }
            let until =
                if step == ESCALATIONS { deadline } else { (Instant::now() + STEP_TIMEOUT).min(deadline) };
            if wait_for_foreground(hwnd, until) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                break;
            }
            match step {
                0 => tap_alt(),
                1 => attach_and_raise(hwnd),
                // SAFETY: last resort activation, ignores failure by design.
                _ => unsafe { SwitchToThisWindow(hwnd, true) },
            }
        }
        bail!("could not activate window {:#x} ('{}') within {:?}", window.0, window_title(hwnd), timeout);
    }

    fn foreground(&self) -> Option<WindowInfo> {
        // SAFETY: plain Win32 call, returns a null handle when nothing is in the foreground.
        let hwnd = unsafe { GetForegroundWindow() };
        window_info(hwnd)
    }
}

/// Title, process name and handle of a window, or `None` for a null handle.
pub fn window_info(hwnd: HWND) -> Option<WindowInfo> {
    if hwnd.0.is_null() {
        return None;
    }
    Some(WindowInfo {
        handle: WindowRef(hwnd.0 as isize),
        title: window_title(hwnd),
        process_name: process_name(hwnd).unwrap_or_default(),
    })
}

pub fn window_title(hwnd: HWND) -> String {
    // SAFETY: length query followed by a read into a buffer sized from it.
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; len as usize + 1];
        let written = GetWindowTextW(hwnd, &mut buffer).max(0) as usize;
        String::from_utf16_lossy(&buffer[..written])
    }
}

/// Executable file name of the window's process, such as `notepad.exe`.
pub fn process_name(hwnd: HWND) -> Option<String> {
    process_name_of_pid(window_pid(hwnd))
}

pub fn window_pid(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    // SAFETY: `pid` is a live local.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

pub fn own_pid() -> u32 {
    // SAFETY: plain Win32 call without arguments.
    unsafe { GetCurrentProcessId() }
}

fn process_name_of_pid(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    // SAFETY: the handle is closed on every path and the buffer outlives the query.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 512];
        let mut len = buffer.len() as u32;
        let result =
            QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut len);
        let _ = CloseHandle(handle);
        result.ok()?;
        let path = String::from_utf16_lossy(&buffer[..len as usize]);
        Some(path.rsplit(['\\', '/']).next().unwrap_or("").to_string())
    }
}

/// Whether a window passes the user's filters; empty filters match anything and case is ignored.
pub fn matches(title: &str, process_name: &str, title_contains: &str, process_filter: &str) -> bool {
    let title_ok = title_contains.is_empty() || title.to_lowercase().contains(&title_contains.to_lowercase());
    let process_ok =
        process_filter.is_empty() || process_name.to_lowercase() == process_filter.to_lowercase();
    title_ok && process_ok
}

struct FindState {
    title_contains: String,
    process_name: String,
    own_pid: u32,
    found: Option<WindowRef>,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: `find` passes a pointer to a `FindState` that outlives the enumeration.
    let state = unsafe { &mut *(lparam.0 as *mut FindState) };
    // SAFETY: plain Win32 queries on the enumerated handle.
    let (visible, ex_style) =
        unsafe { (IsWindowVisible(hwnd).as_bool(), GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32) };
    if !visible || ex_style & WS_EX_TOOLWINDOW.0 != 0 || window_pid(hwnd) == state.own_pid {
        return TRUE;
    }
    let title = window_title(hwnd);
    if title.is_empty() {
        return TRUE;
    }
    let process = process_name(hwnd).unwrap_or_default();
    if matches(&title, &process, &state.title_contains, &state.process_name) {
        state.found = Some(WindowRef(hwnd.0 as isize));
        return BOOL(0);
    }
    TRUE
}

fn hwnd_of(window: WindowRef) -> HWND {
    HWND(window.0 as *mut c_void)
}

fn wait_for_foreground(hwnd: HWND, until: Instant) -> bool {
    loop {
        // SAFETY: plain Win32 call.
        if unsafe { GetForegroundWindow() } == hwnd {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Presses and releases Alt, which clears the foreground lock Windows applies to background processes.
fn tap_alt() {
    let injector = Win32Injector;
    let key = keys::key_from_vk(vk::MENU);
    if let Err(e) = injector.key(key, true).and_then(|()| injector.key(key, false)) {
        log::debug!("alt tap failed: {e}");
    }
}

fn attach_and_raise(hwnd: HWND) {
    // SAFETY: input is detached again on every path; failures are informational only.
    unsafe {
        let foreground = GetForegroundWindow();
        let target = GetWindowThreadProcessId(foreground, None);
        let ours = GetCurrentThreadId();
        if target == 0 || target == ours {
            return;
        }
        if !AttachThreadInput(ours, target, true).as_bool() {
            return;
        }
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        let _ = AttachThreadInput(ours, target, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_filters_match_anything() {
        assert!(matches("Untitled - Notepad", "notepad.exe", "", ""));
        assert!(matches("", "", "", ""));
    }

    #[test]
    fn title_filter_is_a_case_insensitive_substring() {
        assert!(matches("Untitled - Notepad", "notepad.exe", "NOTEpad", ""));
        assert!(matches("Untitled - Notepad", "notepad.exe", "untitled -", ""));
        assert!(!matches("Untitled - Notepad", "notepad.exe", "calculator", ""));
    }

    #[test]
    fn process_filter_compares_the_whole_file_name_case_insensitively() {
        assert!(matches("Untitled - Notepad", "Notepad.exe", "", "notepad.exe"));
        assert!(matches("Untitled - Notepad", "notepad.exe", "", "NOTEPAD.EXE"));
        assert!(!matches("Untitled - Notepad", "notepad.exe", "", "notepad"));
        assert!(!matches("Untitled - Notepad", "notepad.exe", "", "wordpad.exe"));
    }

    #[test]
    fn both_filters_must_match() {
        assert!(matches("Calculator", "calc.exe", "calc", "calc.exe"));
        assert!(!matches("Calculator", "calc.exe", "calc", "notepad.exe"));
        assert!(!matches("Calculator", "calc.exe", "notepad", "calc.exe"));
    }
}
