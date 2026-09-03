use std::collections::HashMap;
use std::fs::File;
use std::io::Write as _;
use std::os::fd::AsFd as _;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use anyhow::{Context as _, Result};
use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::EventQueue;
use wayland_client::protocol::wl_keyboard::KeymapFormat;
use wayland_client::protocol::wl_pointer::{Axis, AxisSource, ButtonState};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1;

use super::hyprland::Hyprland;
use super::keymap::Xkb;
use super::keys;
use super::wayland::{State, Wayland};
use crate::model::{Key, MouseButton, Point};
use crate::platform::{CharKey, InputInjector};

/// Linux button codes from `input-event-codes.h`.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
const BTN_SIDE: u32 = 0x113;
const BTN_EXTRA: u32 = 0x114;

/// Extra roundtrips spent waiting for the seat's keymap after the outputs have settled.
const KEYMAP_WAIT_ROUNDTRIPS: usize = 5;

/// Input injection through the virtual pointer and virtual keyboard protocols on its own connection.
pub struct WaylandInjector {
    inner: Mutex<Inner>,
}

struct Inner {
    wayland: Wayland,
    queue: EventQueue<State>,
    start: Instant,
    /// One output-bound virtual pointer per monitor connector name.
    pointers: HashMap<String, ZwlrVirtualPointerV1>,
    /// Virtual pointer with no output, used for relative motion, buttons and the wheel.
    unbound_pointer: Option<ZwlrVirtualPointerV1>,
    layout_keyboard: ZwpVirtualKeyboardV1,
    /// Compiled once from the seat's keymap at connect time; later keymap events are echoes of our
    /// own uploads re-serialized by the compositor and cannot be told apart from a layout change.
    layout_xkb: Option<Xkb>,
    unicode_keyboard: ZwpVirtualKeyboardV1,
    /// Character assigned to each unicode-keyboard slot, `None` when the slot is free.
    unicode_slots: Vec<Option<char>>,
    /// Whether a slot's key is currently held down, so it is never evicted while pressed.
    unicode_held: Vec<bool>,
    /// Keymap text last uploaded to `unicode_keyboard`.
    unicode_keymap_text: String,
    last_pos: Option<Point>,
    hyprland: Option<Hyprland>,
}

impl WaylandInjector {
    pub fn new() -> Result<Self> {
        let (mut wayland, mut queue) = Wayland::connect().context("connecting to Wayland")?;
        for _ in 0..KEYMAP_WAIT_ROUNDTRIPS {
            if wayland.state.keymap.is_some() {
                break;
            }
            wayland.roundtrip(&mut queue).context("waiting for the seat keymap")?;
        }
        let layout_keymap_text =
            wayland.state.keymap.clone().context("the seat advertised no keyboard keymap")?;

        let manager = wayland
            .state
            .virtual_keyboard_manager
            .as_ref()
            .context("the compositor does not advertise zwp_virtual_keyboard_manager_v1")?;
        let seat = wayland.state.seat.as_ref().context("the compositor has no wl_seat")?;

        let layout_keyboard = manager.create_virtual_keyboard(seat, &wayland.qh, ());
        upload_keymap(&layout_keyboard, &layout_keymap_text)?;
        let layout_xkb = match Xkb::new(&layout_keymap_text) {
            Ok(xkb) => Some(xkb),
            Err(e) => {
                log::warn!("compiling the seat keymap failed, layout-aware key injection is disabled: {e:#}");
                None
            }
        };

        let unicode_keyboard = manager.create_virtual_keyboard(seat, &wayland.qh, ());
        let unicode_keymap_text = Xkb::unicode_keymap_text(&[]);
        upload_keymap(&unicode_keyboard, &unicode_keymap_text)?;

        wayland.flush()?;

        let inner = Inner {
            wayland,
            queue,
            start: Instant::now(),
            pointers: HashMap::new(),
            unbound_pointer: None,
            layout_keyboard,
            layout_xkb,
            unicode_keyboard,
            unicode_slots: Vec::new(),
            unicode_held: Vec::new(),
            unicode_keymap_text,
            last_pos: None,
            hyprland: Hyprland::detect(),
        };
        log::debug!("wayland injector ready, hyprland ipc {}", inner.hyprland.is_some());
        Ok(Self { inner: Mutex::new(inner) })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poison| poison.into_inner())
    }
}

impl Inner {
    fn now_ms(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    /// Roundtrips the connection so the output layout stays fresh.
    fn sync(&mut self) {
        if let Err(e) = self.wayland.roundtrip(&mut self.queue) {
            log::debug!("wayland roundtrip failed: {e:#}");
        }
    }

    fn pointer_for_monitor(&mut self, label: &str) -> Result<ZwlrVirtualPointerV1> {
        if let Some(pointer) = self.pointers.get(label) {
            return Ok(pointer.clone());
        }
        let manager = self
            .wayland
            .state
            .virtual_pointer_manager
            .as_ref()
            .context("the compositor does not advertise zwlr_virtual_pointer_manager_v1")?;
        let seat = self.wayland.state.seat.as_ref();
        let output = self.wayland.state.output_named(label).map(|o| &o.output);
        let pointer = manager.create_virtual_pointer_with_output(seat, output, &self.wayland.qh, ());
        self.wayland.flush()?;
        self.pointers.insert(label.to_string(), pointer.clone());
        Ok(pointer)
    }

    fn unbound_pointer(&mut self) -> Result<ZwlrVirtualPointerV1> {
        if let Some(pointer) = &self.unbound_pointer {
            return Ok(pointer.clone());
        }
        let manager = self
            .wayland
            .state
            .virtual_pointer_manager
            .as_ref()
            .context("the compositor does not advertise zwlr_virtual_pointer_manager_v1")?;
        let seat = self.wayland.state.seat.as_ref();
        let pointer = manager.create_virtual_pointer(seat, &self.wayland.qh, ());
        self.wayland.flush()?;
        self.unbound_pointer = Some(pointer.clone());
        Ok(pointer)
    }

    /// Evdev code for `key`, preferring its position and falling back to the layout for letters and digits.
    fn evdev_for_key(&self, key: Key) -> Result<u16> {
        if key.scancode == 0
            && let Some(evdev) = self.layout_xkb.as_ref().and_then(|xkb| xkb.evdev_for_vk(key.vk))
        {
            return Ok(evdev);
        }
        keys::evdev_from_key(key).with_context(|| format!("no evdev key for {key:?}"))
    }

    /// The slot index assigned to `ch` on the unicode keyboard, uploading a new keymap if needed.
    fn ensure_unicode_slot(&mut self, ch: char) -> Result<usize> {
        if let Some(i) = self.unicode_slots.iter().position(|s| *s == Some(ch)) {
            return Ok(i);
        }
        let index = match self.unicode_slots.iter().position(Option::is_none) {
            Some(i) => i,
            None => match self.unicode_held.iter().position(|&held| !held) {
                Some(i) => i,
                None => {
                    self.unicode_slots.push(None);
                    self.unicode_held.push(false);
                    self.unicode_slots.len() - 1
                }
            },
        };
        self.unicode_slots[index] = Some(ch);
        let text = Xkb::unicode_keymap_text(&self.unicode_slots);
        upload_keymap(&self.unicode_keyboard, &text)?;
        self.unicode_keymap_text = text;
        self.wayland.flush()?;
        Ok(index)
    }
}

impl InputInjector for WaylandInjector {
    fn key(&self, key: Key, down: bool) -> Result<()> {
        let mut inner = self.lock();
        let evdev = inner.evdev_for_key(key)?;
        let time = inner.now_ms();
        inner.layout_keyboard.key(time, evdev as u32, if down { 1 } else { 0 });
        if let Some(xkb) = inner.layout_xkb.as_mut() {
            let (depressed, latched, locked, group) = xkb.update_key(evdev, down);
            inner.layout_keyboard.modifiers(depressed, latched, locked, group);
        }
        inner.wayland.flush()
    }

    fn unicode(&self, ch: char, down: bool) -> Result<()> {
        let mut inner = self.lock();
        let slot = inner.ensure_unicode_slot(ch)?;
        let evdev = slot as u32 + 1;
        let time = inner.now_ms();
        inner.unicode_keyboard.key(time, evdev, if down { 1 } else { 0 });
        inner.unicode_held[slot] = down;
        inner.wayland.flush()
    }

    fn mouse_move_abs(&self, pos: Point) -> Result<()> {
        let mut inner = self.lock();
        inner.sync();
        let layout = inner.wayland.layout();
        let monitor = layout.monitor_near(pos).context("no monitor to move the cursor onto")?;
        let rect = layout.physical(monitor);
        let offset_x = (pos.x - rect.x).clamp(0, (rect.w - 1).max(0));
        let offset_y = (pos.y - rect.y).clamp(0, (rect.h - 1).max(0));
        let label = monitor.name.clone();
        let pointer = inner.pointer_for_monitor(&label)?;
        let time = inner.now_ms();
        // Extents equal to the monitor's pixel size map the offset onto pixels one to one.
        let x_extent = rect.w as u32;
        let y_extent = rect.h as u32;
        let x = offset_x as u32;
        let y = offset_y as u32;
        pointer.motion_absolute(time, x, y, x_extent, y_extent);
        pointer.frame();
        inner.wayland.flush()?;
        inner.last_pos = Some(pos);
        Ok(())
    }

    fn mouse_move_rel(&self, dx: i32, dy: i32) -> Result<()> {
        let mut inner = self.lock();
        let pointer = inner.unbound_pointer()?;
        let time = inner.now_ms();
        pointer.motion(time, dx as f64, dy as f64);
        pointer.frame();
        inner.wayland.flush()
    }

    fn mouse_button(&self, button: MouseButton, down: bool) -> Result<()> {
        let mut inner = self.lock();
        let pointer = inner.unbound_pointer()?;
        let code = match button {
            MouseButton::Left => BTN_LEFT,
            MouseButton::Right => BTN_RIGHT,
            MouseButton::Middle => BTN_MIDDLE,
            MouseButton::X1 => BTN_SIDE,
            MouseButton::X2 => BTN_EXTRA,
        };
        let state = if down { ButtonState::Pressed } else { ButtonState::Released };
        let time = inner.now_ms();
        pointer.button(time, code, state);
        pointer.frame();
        inner.wayland.flush()
    }

    fn mouse_wheel(&self, delta: i32, horizontal: bool) -> Result<()> {
        if delta == 0 {
            return Ok(());
        }
        let mut inner = self.lock();
        let pointer = inner.unbound_pointer()?;
        let axis = if horizontal { Axis::HorizontalScroll } else { Axis::VerticalScroll };
        // The model is positive for up and right; the protocol is positive for down and right.
        let signed = if horizontal { delta } else { -delta };
        let time = inner.now_ms();
        pointer.axis_source(AxisSource::Wheel);
        if signed % 120 == 0 {
            let discrete = signed / 120;
            pointer.axis_discrete(time, axis, 15.0 * discrete as f64, discrete);
        } else {
            pointer.axis(time, axis, 15.0 * signed as f64 / 120.0);
        }
        pointer.frame();
        inner.wayland.flush()
    }

    /// Cursor position from Hyprland, exact to one pixel on fractionally scaled outputs since the IPC
    /// reports whole logical units.
    fn cursor_pos(&self) -> Result<Point> {
        let mut inner = self.lock();
        inner.sync();
        if let Some(hyprland) = &inner.hyprland {
            let (x, y) = hyprland.cursor_pos()?;
            let layout = inner.wayland.layout();
            return layout.to_physical(x, y).context("cursor position is outside every known monitor");
        }
        inner
            .last_pos
            .context("cursor position is unknown: Hyprland is not running and nothing has moved it yet")
    }

    fn key_for_char(&self, ch: char) -> Option<CharKey> {
        let inner = self.lock();
        inner.layout_xkb.as_ref()?.key_for_char(ch)
    }
}

/// Uploads `text` as the keymap of `keyboard` through a memfd, NUL-terminated like the compositor does.
fn upload_keymap(keyboard: &ZwpVirtualKeyboardV1, text: &str) -> Result<()> {
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0);
    let fd =
        memfd_create("macro-recorder-keymap", MemfdFlags::CLOEXEC).context("memfd_create for the keymap")?;
    let mut file = File::from(fd);
    file.write_all(&bytes).context("writing the keymap to the memfd")?;
    keyboard.keymap(KeymapFormat::XkbV1 as u32, file.as_fd(), bytes.len() as u32);
    Ok(())
}
