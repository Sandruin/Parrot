use std::time::Instant;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows::Win32::UI::WindowsAndMessaging::{CHILDID_SELF, EVENT_SYSTEM_FOREGROUND, OBJID_WINDOW};

use super::hooks;
use super::window;
use crate::model::RawInputEvent;

/// Reports foreground changes of other processes' windows as [`RawInputEvent::Foreground`].
pub(crate) unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if event != EVENT_SYSTEM_FOREGROUND
        || id_object != OBJID_WINDOW.0
        || id_child != CHILDID_SELF as i32
        || hwnd.0.is_null()
    {
        return;
    }
    let Some(ctx) = hooks::ctx() else { return };
    let pid = window::window_pid(hwnd);
    if pid == 0 || pid == window::own_pid() {
        return;
    }
    ctx.send(RawInputEvent::Foreground {
        hwnd: hwnd.0 as isize,
        title: window::window_title(hwnd),
        process_name: window::process_name(hwnd).unwrap_or_default(),
        at: Instant::now(),
    });
}
