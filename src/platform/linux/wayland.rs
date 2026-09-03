use std::fs::File;
use std::os::fd::{AsFd, OwnedFd};

use anyhow::{Context as _, Result, bail};
use memmap2::{MmapMut, MmapOptions};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_keyboard::{self, WlKeyboard};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1;
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::{self, ZxdgOutputV1};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::ZwlrLayerShellV1;
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1;

use super::layout::{Layout, Monitor};
use super::protocols::hyprland_global_shortcuts_manager_v1::HyprlandGlobalShortcutsManagerV1;
use super::{capture, hotkeys, overlay};
use crate::model::Rect;

/// Roundtrips spent waiting for every output to be fully described at connect time.
const SETTLE_ROUNDTRIPS: usize = 4;

/// One `wl_output` and what the compositor has told us about it so far.
#[derive(Debug)]
pub struct Output {
    /// Registry name of the global, doubles as the user data of its objects.
    pub name: u32,
    pub output: WlOutput,
    pub xdg: Option<ZxdgOutputV1>,
    /// Connector name such as `eDP-1`.
    pub label: String,
    pub logical_pos: Option<(i32, i32)>,
    pub logical_size: Option<(i32, i32)>,
    /// Current mode in pixels, before the transform.
    pub mode: Option<(i32, i32)>,
    pub transform: wl_output::Transform,
    pub done: bool,
}

impl Output {
    /// The monitor for the layout once position, logical size and mode are all known.
    pub fn monitor(&self) -> Option<Monitor> {
        let (x, y) = self.logical_pos?;
        let (lw, lh) = self.logical_size?;
        let (mw, mh) = self.mode?;
        let rotated = matches!(
            self.transform,
            wl_output::Transform::_90
                | wl_output::Transform::_270
                | wl_output::Transform::Flipped90
                | wl_output::Transform::Flipped270
        );
        let (width, height) = if rotated { (mh, mw) } else { (mw, mh) };
        if lw <= 0 || lh <= 0 || width <= 0 || height <= 0 {
            return None;
        }
        Some(Monitor { name: self.label.clone(), logical: Rect::new(x, y, lw, lh), width, height })
    }
}

/// Registry globals plus the bookkeeping every Wayland connection of this backend shares.
pub struct State {
    pub compositor: Option<WlCompositor>,
    pub shm: Option<WlShm>,
    pub seat: Option<WlSeat>,
    pub keyboard: Option<WlKeyboard>,
    pub layer_shell: Option<ZwlrLayerShellV1>,
    pub viewporter: Option<WpViewporter>,
    pub fractional_scale: Option<WpFractionalScaleManagerV1>,
    pub xdg_output_manager: Option<ZxdgOutputManagerV1>,
    pub screencopy: Option<ZwlrScreencopyManagerV1>,
    pub virtual_pointer_manager: Option<ZwlrVirtualPointerManagerV1>,
    pub virtual_keyboard_manager: Option<ZwpVirtualKeyboardManagerV1>,
    pub global_shortcuts: Option<HyprlandGlobalShortcutsManagerV1>,
    pub outputs: Vec<Output>,
    /// Text of the seat's xkb keymap, replaced whenever the compositor sends a new one.
    pub keymap: Option<String>,
    /// Incremented on every keymap event so readers can tell when to recompile.
    pub keymap_serial: u32,
    pub overlay: overlay::WlState,
    pub capture: capture::WlState,
    pub shortcuts: hotkeys::WlState,
}

/// A connection with its bound globals; the event queue is handed out separately so a loop can own it.
pub struct Wayland {
    pub conn: Connection,
    pub qh: QueueHandle<State>,
    pub state: State,
}

impl Wayland {
    /// Connects to the compositor, binds every global we know and waits until the outputs are described.
    pub fn connect() -> Result<(Self, EventQueue<State>)> {
        let conn = Connection::connect_to_env().context("connecting to the Wayland display")?;
        let (globals, mut queue) =
            registry_queue_init::<State>(&conn).context("reading the Wayland registry")?;
        let qh = queue.handle();
        let mut state = State {
            compositor: globals.bind(&qh, 4..=6, ()).ok(),
            shm: globals.bind(&qh, 1..=2, ()).ok(),
            seat: globals.bind(&qh, 5..=9, ()).ok(),
            keyboard: None,
            layer_shell: globals.bind(&qh, 1..=5, ()).ok(),
            viewporter: globals.bind(&qh, 1..=1, ()).ok(),
            fractional_scale: globals.bind(&qh, 1..=1, ()).ok(),
            xdg_output_manager: globals.bind(&qh, 1..=3, ()).ok(),
            screencopy: globals.bind(&qh, 1..=3, ()).ok(),
            virtual_pointer_manager: globals.bind(&qh, 1..=2, ()).ok(),
            virtual_keyboard_manager: globals.bind(&qh, 1..=1, ()).ok(),
            global_shortcuts: globals.bind(&qh, 1..=1, ()).ok(),
            outputs: Vec::new(),
            keymap: None,
            keymap_serial: 0,
            overlay: overlay::WlState::default(),
            capture: capture::WlState::default(),
            shortcuts: hotkeys::WlState::default(),
        };
        let registry = globals.registry();
        globals.contents().with_list(|list| {
            for global in list.iter().filter(|g| g.interface == WlOutput::interface().name) {
                state.add_output(registry, global.name, global.version, &qh);
            }
        });
        let mut wayland = Self { conn, qh, state };
        for _ in 0..SETTLE_ROUNDTRIPS {
            wayland.roundtrip(&mut queue)?;
            if wayland.state.outputs.iter().all(|o| o.done && o.monitor().is_some()) {
                break;
            }
        }
        if wayland.state.outputs.is_empty() {
            bail!("the compositor advertises no outputs");
        }
        log::debug!("wayland connected, layout {:?}", wayland.layout());
        Ok((wayland, queue))
    }

    /// Flushes requests and blocks until the compositor has answered everything sent so far.
    pub fn roundtrip(&mut self, queue: &mut EventQueue<State>) -> Result<()> {
        queue.roundtrip(&mut self.state).context("wayland roundtrip")?;
        Ok(())
    }

    pub fn flush(&self) -> Result<()> {
        self.conn.flush().context("flushing the wayland connection")
    }

    pub fn layout(&self) -> Layout {
        self.state.layout()
    }
}

impl State {
    fn add_output(&mut self, registry: &WlRegistry, name: u32, version: u32, qh: &QueueHandle<State>) {
        let output: WlOutput = registry.bind(name, version.min(4), qh, name);
        let xdg = self.xdg_output_manager.as_ref().map(|manager| manager.get_xdg_output(&output, qh, name));
        self.outputs.push(Output {
            name,
            output,
            xdg,
            label: String::new(),
            logical_pos: None,
            logical_size: None,
            mode: None,
            transform: wl_output::Transform::Normal,
            done: false,
        });
    }

    fn output_mut(&mut self, name: u32) -> Option<&mut Output> {
        self.outputs.iter_mut().find(|o| o.name == name)
    }

    /// Current monitor layout, from every output that is fully described.
    pub fn layout(&self) -> Layout {
        Layout::new(self.outputs.iter().filter_map(Output::monitor).collect())
    }

    /// The output object for a connector name from the layout.
    pub fn output_named(&self, label: &str) -> Option<&Output> {
        self.outputs.iter().find(|o| o.label == label)
    }
}

/// A `wl_buffer` backed by anonymous shared memory that this side can write or read directly.
pub struct ShmBuffer {
    pub buffer: WlBuffer,
    pub map: MmapMut,
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pool: WlShmPool,
}

impl ShmBuffer {
    /// Allocates a zeroed buffer of `width` x `height` pixels in the given format, 4 bytes per pixel.
    pub fn new(
        shm: &WlShm,
        qh: &QueueHandle<State>,
        width: i32,
        height: i32,
        format: wl_shm::Format,
    ) -> Result<Self> {
        if width <= 0 || height <= 0 {
            bail!("shm buffer of {width}x{height} is empty");
        }
        let stride = width * 4;
        let size = stride as usize * height as usize;
        let fd = rustix::fs::memfd_create("macro-recorder", rustix::fs::MemfdFlags::CLOEXEC)
            .context("memfd_create")?;
        let file = File::from(fd);
        file.set_len(size as u64).context("sizing the shm file")?;
        // SAFETY: the file was just created by us and stays alive as long as the mapping.
        let map = unsafe { MmapOptions::new().len(size).map_mut(&file) }.context("mapping the shm file")?;
        let pool = shm.create_pool(file.as_fd(), size as i32, qh, ());
        let buffer = pool.create_buffer(0, width, height, stride, format, qh, ());
        Ok(Self { buffer, map, width, height, stride, pool })
    }
}

impl Drop for ShmBuffer {
    fn drop(&mut self) {
        self.buffer.destroy();
        self.pool.destroy();
    }
}

/// Reads the keymap text the compositor shares through a file descriptor.
fn read_keymap(fd: OwnedFd, size: u32) -> Result<String> {
    let file = File::from(fd);
    // SAFETY: the compositor hands out a read-only mapping of exactly `size` bytes.
    let map = unsafe { MmapOptions::new().len(size as usize).map(&file) }.context("mapping the keymap")?;
    let text = std::str::from_utf8(&map).context("keymap is not UTF-8")?;
    Ok(text.trim_end_matches('\0').to_owned())
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global { name, interface, version }
                if interface == WlOutput::interface().name =>
            {
                state.add_output(registry, name, version, qh);
            }
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(index) = state.outputs.iter().position(|o| o.name == name) {
                    let output = state.outputs.remove(index);
                    if let Some(xdg) = output.xdg {
                        xdg.destroy();
                    }
                    if output.output.version() >= 3 {
                        output.output.release();
                    }
                    log::debug!("output {} went away", output.label);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, u32> for State {
    fn event(
        state: &mut Self,
        _: &WlOutput,
        event: wl_output::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = state.output_mut(*name) else { return };
        match event {
            wl_output::Event::Geometry { transform: WEnum::Value(transform), .. } => {
                output.transform = transform
            }
            wl_output::Event::Mode { flags, width, height, .. } => {
                if let WEnum::Value(flags) = flags
                    && flags.contains(wl_output::Mode::Current)
                {
                    output.mode = Some((width, height));
                }
            }
            wl_output::Event::Name { name } => output.label = name,
            wl_output::Event::Done => output.done = true,
            _ => {}
        }
    }
}

impl Dispatch<ZxdgOutputV1, u32> for State {
    fn event(
        state: &mut Self,
        _: &ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = state.output_mut(*name) else { return };
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => output.logical_pos = Some((x, y)),
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                output.logical_size = Some((width, height))
            }
            zxdg_output_v1::Event::Name { name } if output.label.is_empty() => output.label = name,
            zxdg_output_v1::Event::Done => output.done = true,
            _ => {}
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(caps) } = event {
            let has_keyboard = caps.contains(wl_seat::Capability::Keyboard);
            match (&state.keyboard, has_keyboard) {
                (None, true) => state.keyboard = Some(seat.get_keyboard(qh, ())),
                (Some(keyboard), false) => {
                    keyboard.release();
                    state.keyboard = None;
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Keymap { format, fd, size } = event {
            if format != WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) {
                log::warn!("unsupported keymap format {format:?}");
                return;
            }
            match read_keymap(fd, size) {
                Ok(text) => {
                    state.keymap = Some(text);
                    state.keymap_serial += 1;
                }
                Err(e) => log::warn!("reading the keymap failed: {e:#}"),
            }
        }
    }
}

delegate_noop!(State: ignore WlCompositor);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: ignore WlBuffer);
delegate_noop!(State: ignore WlSurface);
delegate_noop!(State: WlRegion);
delegate_noop!(State: WpViewporter);
delegate_noop!(State: WpViewport);
delegate_noop!(State: WpFractionalScaleManagerV1);
delegate_noop!(State: ZxdgOutputManagerV1);
delegate_noop!(State: ZwlrLayerShellV1);
delegate_noop!(State: ZwlrScreencopyManagerV1);
delegate_noop!(State: ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ZwlrVirtualPointerV1);
delegate_noop!(State: ZwpVirtualKeyboardManagerV1);
delegate_noop!(State: ZwpVirtualKeyboardV1);
delegate_noop!(State: HyprlandGlobalShortcutsManagerV1);
