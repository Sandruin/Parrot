use anyhow::{Result, bail};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE, MOUSE_EVENT_FLAGS,
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP,
    MOUSEINPUT, SendInput, VIRTUAL_KEY, VkKeyScanExW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, SetCursorPos, XBUTTON1, XBUTTON2,
};

use super::keys;
use crate::model::{Key, MouseButton, Point, Rect};
use crate::platform::{CharKey, InputInjector};

/// `dwExtraInfo` stamped on every event we inject, so the hooks can tell our own input apart.
/// Must stay inside 32 bits: Windows truncates the extra info of mouse events to the low `DWORD`.
pub const MAGIC: usize = 0x4D52_0001;

/// Sends keyboard and mouse input through `SendInput`, tagged with [`MAGIC`].
#[derive(Default)]
pub struct Win32Injector;

impl InputInjector for Win32Injector {
    fn key(&self, key: Key, down: bool) -> Result<()> {
        if key.vk != 0 && keys::scancode_ex(key.vk) >> 8 == keys::PREFIX_E1 {
            return send(&[key_input(VIRTUAL_KEY(key.vk), 0, flags_for(down, KEYBD_EVENT_FLAGS(0)))]);
        }
        let (scancode, extended) = if key.scancode != 0 {
            (key.scancode, key.extended)
        } else {
            let mapped = keys::key_from_vk(key.vk);
            (mapped.scancode, key.extended || mapped.extended)
        };
        if scancode == 0 {
            if key.vk == 0 {
                bail!("cannot inject a key with neither virtual-key code nor scan code");
            }
            return send(&[key_input(VIRTUAL_KEY(key.vk), 0, flags_for(down, KEYBD_EVENT_FLAGS(0)))]);
        }
        let mut base = KEYEVENTF_SCANCODE;
        if extended {
            base |= KEYEVENTF_EXTENDEDKEY;
        }
        send(&[key_input(VIRTUAL_KEY(0), scancode, flags_for(down, base))])
    }

    fn unicode(&self, utf16: u16, down: bool) -> Result<()> {
        send(&[key_input(VIRTUAL_KEY(0), utf16, flags_for(down, KEYEVENTF_UNICODE))])
    }

    fn mouse_move_abs(&self, pos: Point) -> Result<()> {
        let screen = virtual_screen();
        let x = normalize(pos.x, screen.x, screen.w);
        let y = normalize(pos.y, screen.y, screen.h);
        send(&[mouse_input(x, y, 0, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK)])?;
        if self.cursor_pos()? != pos {
            // SAFETY: plain Win32 call, absorbs the rounding of the 0..65535 absolute range.
            unsafe { SetCursorPos(pos.x, pos.y) }?;
        }
        Ok(())
    }

    fn mouse_button(&self, button: MouseButton, down: bool) -> Result<()> {
        let (flags, data) = match (button, down) {
            (MouseButton::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
            (MouseButton::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
            (MouseButton::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
            (MouseButton::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
            (MouseButton::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
            (MouseButton::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
            (MouseButton::X1, true) => (MOUSEEVENTF_XDOWN, XBUTTON1 as u32),
            (MouseButton::X1, false) => (MOUSEEVENTF_XUP, XBUTTON1 as u32),
            (MouseButton::X2, true) => (MOUSEEVENTF_XDOWN, XBUTTON2 as u32),
            (MouseButton::X2, false) => (MOUSEEVENTF_XUP, XBUTTON2 as u32),
        };
        send(&[mouse_input(0, 0, data, flags)])
    }

    fn mouse_move_rel(&self, dx: i32, dy: i32) -> Result<()> {
        send(&[mouse_input(dx, dy, 0, MOUSEEVENTF_MOVE)])
    }

    fn mouse_wheel(&self, delta: i32, horizontal: bool) -> Result<()> {
        let flags = if horizontal { MOUSEEVENTF_HWHEEL } else { MOUSEEVENTF_WHEEL };
        send(&[mouse_input(0, 0, delta as u32, flags)])
    }

    fn cursor_pos(&self) -> Result<Point> {
        let mut pt = POINT::default();
        // SAFETY: `pt` is a live local of the right type.
        unsafe { GetCursorPos(&mut pt) }?;
        Ok(Point::new(pt.x, pt.y))
    }

    fn key_for_char(&self, ch: char) -> Option<CharKey> {
        let mut units = [0u16; 2];
        let encoded = ch.encode_utf16(&mut units);
        if encoded.len() != 1 {
            return None;
        }
        // SAFETY: both calls are pure lookups against the calling thread's layout.
        let scan = unsafe { VkKeyScanExW(units[0], GetKeyboardLayout(0)) };
        if scan == -1 {
            return None;
        }
        let vk = (scan as u16) & 0xFF;
        let state = ((scan as u16) >> 8) & 0xFF;
        Some(CharKey {
            key: keys::key_from_vk(vk),
            shift: state & 0x01 != 0,
            ctrl: state & 0x02 != 0,
            alt: state & 0x04 != 0,
        })
    }
}

/// Bounding rectangle of all monitors in physical pixels.
pub fn virtual_screen() -> Rect {
    // SAFETY: plain Win32 calls with constant arguments.
    unsafe {
        Rect::new(
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

/// Maps a virtual-screen pixel onto the 0..65535 range that `MOUSEEVENTF_ABSOLUTE` expects.
fn normalize(value: i32, origin: i32, size: i32) -> i32 {
    let span = (size - 1).max(1) as i64;
    (((value - origin) as i64 * 65535 + span / 2) / span) as i32
}

/// Inverse of [`normalize`], used to check the round trip stays within a pixel.
#[cfg(test)]
fn denormalize(value: i32, origin: i32, size: i32) -> i32 {
    let span = (size - 1).max(1) as i64;
    origin + ((value as i64 * span + 32767) / 65535) as i32
}

fn flags_for(down: bool, base: KEYBD_EVENT_FLAGS) -> KEYBD_EVENT_FLAGS {
    if down { base } else { base | KEYEVENTF_KEYUP }
}

fn key_input(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: MAGIC },
        },
    }
}

fn mouse_input(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { dx, dy, mouseData: data, dwFlags: flags, time: 0, dwExtraInfo: MAGIC },
        },
    }
}

fn send(inputs: &[INPUT]) -> Result<()> {
    // SAFETY: `inputs` is a live slice and the size argument matches its element type.
    let sent = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) } as usize;
    if sent != inputs.len() {
        bail!(
            "SendInput accepted {sent} of {} events: {}",
            inputs.len(),
            windows::core::Error::from_thread()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_coordinates_round_trip_within_one_pixel() {
        let geometries = [
            (0, 0, 1920, 1080),
            (0, 0, 3840, 2160),
            (-1920, -300, 5760, 2160),
            (-2560, 0, 2560, 1440),
            (0, 0, 1, 1),
        ];
        for (ox, oy, w, h) in geometries {
            for (x, y) in [
                (ox, oy),
                (ox + w - 1, oy + h - 1),
                (ox + w / 2, oy + h / 2),
                (ox + 1, oy + h - 2),
                (ox + w / 3, oy + h / 7),
            ] {
                let back_x = denormalize(normalize(x, ox, w), ox, w);
                let back_y = denormalize(normalize(y, oy, h), oy, h);
                assert!((back_x - x).abs() <= 1, "x {x} -> {back_x} on {ox},{oy} {w}x{h}");
                assert!((back_y - y).abs() <= 1, "y {y} -> {back_y} on {ox},{oy} {w}x{h}");
            }
        }
    }

    #[test]
    fn normalization_hits_the_range_ends() {
        assert_eq!(normalize(0, 0, 1920), 0);
        assert_eq!(normalize(1919, 0, 1920), 65535);
        assert_eq!(normalize(-1920, -1920, 3840), 0);
        assert_eq!(normalize(1919, -1920, 3840), 65535);
    }
}
