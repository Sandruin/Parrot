use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use evdev::{Device, EventSummary, KeyCode, RelativeAxisCode};

use crate::model::MouseButton;

/// One notch of a plain wheel, in the hi-res units the kernel and Windows share.
const NOTCH: i32 = 120;

/// What one evdev event means to the recorder; cursor positions are attached later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Key { code: u16, down: bool },
    Button { button: MouseButton, down: bool },
    Wheel { delta: i32, horizontal: bool },
}

/// Paths of every `/dev/input/event*` node.
pub fn device_paths() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir("/dev/input") else { return Vec::new() };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("event")))
        .collect();
    paths.sort();
    paths
}

/// Opens a device for non-blocking reads.
pub fn open(path: &Path) -> Result<Device> {
    let device = Device::open(path).with_context(|| format!("opening {}", path.display()))?;
    device.set_nonblocking(true).with_context(|| format!("non-blocking mode on {}", path.display()))?;
    Ok(device)
}

/// Keyboards and anything with mouse buttons, which includes touchpads; gamepads and touchscreens are skipped.
pub fn is_interesting(device: &Device) -> bool {
    let keys = device.supported_keys();
    let keyboard = keys.is_some_and(|k| {
        k.contains(KeyCode::KEY_A) || k.contains(KeyCode::KEY_ENTER) || k.contains(KeyCode::KEY_F1)
    });
    let pointer = keys.is_some_and(|k| k.contains(KeyCode::BTN_LEFT));
    keyboard || pointer
}

/// Per-device decoding state: devices with hi-res wheels report both resolutions and we keep one.
#[derive(Clone, Copy, Debug, Default)]
pub struct Decoder {
    hires_wheel: bool,
    hires_hwheel: bool,
}

impl Decoder {
    pub fn new(device: &Device) -> Self {
        let rel = device.supported_relative_axes();
        Self {
            hires_wheel: rel.is_some_and(|r| r.contains(RelativeAxisCode::REL_WHEEL_HI_RES)),
            hires_hwheel: rel.is_some_and(|r| r.contains(RelativeAxisCode::REL_HWHEEL_HI_RES)),
        }
    }

    /// Decodes one event; autorepeats, motion and everything else the recorder ignores yield `None`.
    pub fn decode(&self, event: &evdev::InputEvent) -> Option<InputEvent> {
        match event.destructure() {
            EventSummary::Key(_, code, value) => {
                if value == 2 {
                    return None;
                }
                let down = value != 0;
                let button = match code {
                    KeyCode::BTN_LEFT => Some(MouseButton::Left),
                    KeyCode::BTN_RIGHT => Some(MouseButton::Right),
                    KeyCode::BTN_MIDDLE => Some(MouseButton::Middle),
                    KeyCode::BTN_SIDE => Some(MouseButton::X1),
                    KeyCode::BTN_EXTRA => Some(MouseButton::X2),
                    _ => None,
                };
                match button {
                    Some(button) => Some(InputEvent::Button { button, down }),
                    None if (0x100..0x300).contains(&code.code()) => None,
                    None => Some(InputEvent::Key { code: code.code(), down }),
                }
            }
            EventSummary::RelativeAxis(_, axis, value) => match axis {
                RelativeAxisCode::REL_WHEEL_HI_RES => {
                    Some(InputEvent::Wheel { delta: value, horizontal: false })
                }
                RelativeAxisCode::REL_HWHEEL_HI_RES => {
                    Some(InputEvent::Wheel { delta: value, horizontal: true })
                }
                RelativeAxisCode::REL_WHEEL if !self.hires_wheel => {
                    Some(InputEvent::Wheel { delta: value * NOTCH, horizontal: false })
                }
                RelativeAxisCode::REL_HWHEEL if !self.hires_hwheel => {
                    Some(InputEvent::Wheel { delta: value * NOTCH, horizontal: true })
                }
                _ => None,
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use evdev::{EventType, InputEvent as Raw};

    use super::*;

    fn key(code: KeyCode, value: i32) -> Raw {
        Raw::new(EventType::KEY.0, code.code(), value)
    }

    fn rel(axis: RelativeAxisCode, value: i32) -> Raw {
        Raw::new(EventType::RELATIVE.0, axis.0, value)
    }

    #[test]
    fn keys_buttons_and_repeats() {
        let d = Decoder::default();
        assert_eq!(d.decode(&key(KeyCode::KEY_A, 1)), Some(InputEvent::Key { code: 30, down: true }));
        assert_eq!(d.decode(&key(KeyCode::KEY_A, 2)), None);
        assert_eq!(d.decode(&key(KeyCode::KEY_A, 0)), Some(InputEvent::Key { code: 30, down: false }));
        assert_eq!(
            d.decode(&key(KeyCode::BTN_SIDE, 1)),
            Some(InputEvent::Button { button: MouseButton::X1, down: true })
        );
        assert_eq!(d.decode(&key(KeyCode::BTN_TOUCH, 1)), None);
        assert_eq!(d.decode(&rel(RelativeAxisCode::REL_X, 5)), None);
    }

    #[test]
    fn wheel_resolution_is_taken_once() {
        let plain = Decoder::default();
        assert_eq!(
            plain.decode(&rel(RelativeAxisCode::REL_WHEEL, -1)),
            Some(InputEvent::Wheel { delta: -120, horizontal: false })
        );
        let hires = Decoder { hires_wheel: true, hires_hwheel: false };
        assert_eq!(hires.decode(&rel(RelativeAxisCode::REL_WHEEL, 1)), None);
        assert_eq!(
            hires.decode(&rel(RelativeAxisCode::REL_WHEEL_HI_RES, 60)),
            Some(InputEvent::Wheel { delta: 60, horizontal: false })
        );
        assert_eq!(
            hires.decode(&rel(RelativeAxisCode::REL_HWHEEL, 1)),
            Some(InputEvent::Wheel { delta: 120, horizontal: true })
        );
    }
}
