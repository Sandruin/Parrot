use std::collections::HashSet;
use std::io::{ErrorKind, Read};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use calloop::channel::{Channel, Event as ChannelEvent};
use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction, RegistrationToken};
use calloop_wayland_source::WaylandSource;
use crossbeam_channel::Sender;
use evdev::Device;

use super::hotkeys::{self, Hotkeys};
use super::hyprland::{self, Hyprland};
use super::input::{self, Decoder, InputEvent};
use super::keymap::Xkb;
use super::keys;
use super::overlay::{self, Overlay};
use super::wayland::Wayland;
use crate::model::{HotkeyConfig, OverlayScene, PlatformCommand, PlayerControl, Point, RawInputEvent};

/// Foreign input is ignored for this long after playback starts, so the play hotkey does not stop it.
pub const AUTO_STOP_GRACE: Duration = Duration::from_millis(300);
/// How often the cursor is read from the compositor while recording.
const CURSOR_POLL: Duration = Duration::from_millis(16);
/// How often `/dev/input` is checked for plugged or unplugged devices.
const DEVICE_RESCAN: Duration = Duration::from_secs(2);

/// Handle to the service thread that reads input devices, owns the overlay and handles commands.
pub struct LinuxHandle {
    cmd_tx: Sender<PlatformCommand>,
    thread: Option<JoinHandle<()>>,
    relay: Option<JoinHandle<()>>,
}

impl LinuxHandle {
    /// Clone of the command sender for threads that must not own the handle.
    pub fn cmd_sender(&self) -> Sender<PlatformCommand> {
        self.cmd_tx.clone()
    }

    pub fn send(&self, cmd: PlatformCommand) {
        if self.cmd_tx.send(cmd).is_err() {
            log::warn!("platform service thread is gone");
        }
    }
}

impl Drop for LinuxHandle {
    fn drop(&mut self) {
        self.send(PlatformCommand::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if let Some(t) = self.relay.take() {
            let _ = t.join();
        }
    }
}

/// Starts the service thread and a relay that forwards commands into its event loop.
pub fn spawn_linux_service(raw_tx: Sender<RawInputEvent>) -> Result<LinuxHandle> {
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<PlatformCommand>();
    let (loop_tx, loop_rx) = calloop::channel::channel::<PlatformCommand>();
    let (ready_tx, ready_rx) = crossbeam_channel::bounded::<Result<(), String>>(1);

    let thread =
        std::thread::Builder::new().name("platform".into()).spawn(move || {
            match Service::install(raw_tx, loop_rx) {
                Ok((mut event_loop, mut service)) => {
                    let _ = ready_tx.send(Ok(()));
                    if let Err(e) = event_loop.run(None, &mut service, |_| {}) {
                        log::error!("platform event loop failed: {e}");
                    }
                    log::info!("platform service thread stopped");
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("{e:#}")));
                }
            }
        })?;

    match ready_rx.recv() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let _ = thread.join();
            bail!("platform service thread failed to start: {e}");
        }
        Err(_) => {
            let _ = thread.join();
            bail!("platform service thread died during startup");
        }
    }

    let relay = std::thread::Builder::new().name("platform-relay".into()).spawn(move || {
        while let Ok(cmd) = cmd_rx.recv() {
            let last = matches!(cmd, PlatformCommand::Shutdown);
            if loop_tx.send(cmd).is_err() || last {
                break;
            }
        }
    })?;

    Ok(LinuxHandle { cmd_tx, thread: Some(thread), relay: Some(relay) })
}

struct Playback {
    ctl: Arc<PlayerControl>,
    start: Instant,
}

/// Overlay lifecycle: the scene is remembered and stays hidden while recording or playing.
struct OverlayState {
    window: Overlay,
    scene: Option<OverlayScene>,
    playing: bool,
    recording: bool,
}

/// Everything the service thread owns; lives inside the calloop event loop as its shared data.
struct Service {
    raw_tx: Sender<RawInputEvent>,
    handle: LoopHandle<'static, Service>,
    signal: LoopSignal,
    wayland: Wayland,
    hyprland: Option<Hyprland>,
    hotkeys: Hotkeys,
    hotkey_config: HotkeyConfig,
    /// Virtual-key codes that belong to a configured hotkey chord and never trigger auto-stop.
    chord_vks: Vec<u16>,
    overlay: OverlayState,
    /// Input nodes already opened or found uninteresting, so rescans only touch new ones.
    devices: HashSet<PathBuf>,
    xkb: Option<Xkb>,
    xkb_serial: u32,
    recording: bool,
    cursor_timer: Option<RegistrationToken>,
    last_cursor: Option<Point>,
    playback: Option<Playback>,
    event_buf: Vec<u8>,
    own_pid: u32,
}

impl Service {
    fn install(
        raw_tx: Sender<RawInputEvent>,
        commands: Channel<PlatformCommand>,
    ) -> Result<(EventLoop<'static, Service>, Service)> {
        let hyprland = Hyprland::detect();
        if hyprland.is_none() {
            log::warn!(
                "not running under Hyprland: cursor tracking, window activation and hotkey binds are off"
            );
        }
        let (wayland, queue) = Wayland::connect()?;
        let event_loop = EventLoop::<Service>::try_new().context("creating the event loop")?;
        let handle = event_loop.handle();

        handle
            .insert_source(
                WaylandSource::new(wayland.conn.clone(), queue),
                |_, queue, service: &mut Service| {
                    let dispatched = queue.dispatch_pending(&mut service.wayland.state)?;
                    service.after_wayland();
                    Ok(dispatched)
                },
            )
            .map_err(|e| anyhow!("registering the wayland source: {e}"))?;
        handle
            .insert_source(commands, |event, _, service: &mut Service| match event {
                ChannelEvent::Msg(cmd) => service.handle(cmd),
                ChannelEvent::Closed => service.signal.stop(),
            })
            .map_err(|e| anyhow!("registering the command channel: {e}"))?;
        handle
            .insert_source(Timer::from_duration(DEVICE_RESCAN), |_, _, service: &mut Service| {
                service.rescan_devices();
                TimeoutAction::ToDuration(DEVICE_RESCAN)
            })
            .map_err(|e| anyhow!("registering the device rescan timer: {e}"))?;
        if let Some(h) = &hyprland {
            match h.events() {
                Ok(stream) => {
                    stream.set_nonblocking(true).context("hyprland event socket")?;
                    handle
                        .insert_source(
                            Generic::new(stream, Interest::READ, Mode::Level),
                            |_, stream, service| service.read_hyprland_events(stream),
                        )
                        .map_err(|e| anyhow!("registering the hyprland event socket: {e}"))?;
                }
                Err(e) => log::warn!("hyprland events unavailable, window changes are not tracked: {e:#}"),
            }
        }

        let hotkeys = Hotkeys::new(hyprland.clone());
        let mut service = Service {
            raw_tx,
            handle,
            signal: event_loop.get_signal(),
            wayland,
            hyprland,
            hotkeys,
            hotkey_config: HotkeyConfig::default(),
            chord_vks: hotkeys::chord_vks(&HotkeyConfig::default()),
            overlay: OverlayState { window: Overlay::new(), scene: None, playing: false, recording: false },
            devices: HashSet::new(),
            xkb: None,
            xkb_serial: 0,
            recording: false,
            cursor_timer: None,
            last_cursor: None,
            playback: None,
            event_buf: Vec::new(),
            own_pid: std::process::id(),
        };
        service.refresh_xkb();
        service.rescan_devices();
        service.apply_layer_rule();
        if service.devices.is_empty() {
            log::warn!("no input devices could be opened; is this user in the `input` group?");
        }
        log::info!("platform service thread ready");
        Ok((event_loop, service))
    }

    fn send(&self, event: RawInputEvent) {
        let _ = self.raw_tx.send(event);
    }

    fn handle(&mut self, cmd: PlatformCommand) {
        match cmd {
            PlatformCommand::EnableHooks(enabled) => {
                self.recording = enabled;
                self.overlay.recording = enabled;
                self.refresh_overlay();
                self.apply_hotkeys();
                if enabled {
                    self.last_cursor = self.poll_cursor();
                    self.start_cursor_timer();
                }
            }
            PlatformCommand::SetHotkeys(config) => {
                self.chord_vks = hotkeys::chord_vks(&config);
                self.hotkey_config = config;
                self.apply_hotkeys();
            }
            PlatformCommand::PlaybackStarted(ctl) => {
                self.playback = Some(Playback { ctl, start: Instant::now() });
                self.overlay.playing = true;
                self.refresh_overlay();
                self.apply_hotkeys();
            }
            PlatformCommand::PlaybackStopped => {
                self.playback = None;
                self.overlay.playing = false;
                self.refresh_overlay();
                self.apply_hotkeys();
            }
            PlatformCommand::OverlayShow(scene) => {
                self.overlay.scene = Some(scene);
                self.refresh_overlay();
            }
            PlatformCommand::OverlayHide => {
                self.overlay.scene = None;
                self.overlay.window.hide(&mut self.wayland);
            }
            PlatformCommand::Shutdown => self.signal.stop(),
        }
        if let Err(e) = self.wayland.flush() {
            log::debug!("{e:#}");
        }
    }

    /// Draws the remembered scene, or hides it while recording or playback suppresses the overlay.
    fn refresh_overlay(&mut self) {
        let busy = self.overlay.playing || self.overlay.recording;
        match (&self.overlay.scene, busy) {
            (Some(scene), false) => {
                let scene = scene.clone();
                if let Err(e) = self.overlay.window.show(&mut self.wayland, &scene) {
                    log::error!("drawing the overlay failed: {e:#}");
                }
            }
            _ => self.overlay.window.hide(&mut self.wayland),
        }
    }

    fn apply_hotkeys(&mut self) {
        let busy = self.overlay.playing || self.overlay.recording;
        if let Err(e) = self.hotkeys.set(&mut self.wayland, &self.hotkey_config, busy) {
            log::warn!("applying hotkeys failed: {e:#}");
        }
    }

    /// Keeps the overlay out of screen captures, so templates picked while it shows stay clean.
    fn apply_layer_rule(&self) {
        let Some(h) = &self.hyprland else { return };
        if std::env::var_os(overlay::CAPTURABLE_ENV).is_some() {
            log::info!("{} is set, the overlay stays visible to screen capture", overlay::CAPTURABLE_ENV);
            return;
        }
        let legacy = format!("layerrule noscreenshare, {}", overlay::NAMESPACE);
        let lua = format!(
            "hl.layer_rule({{ match = {{ namespace = \"{}\" }}, no_screen_share = true }})",
            overlay::NAMESPACE
        );
        if let Err(e) = h.configure(&legacy, &lua) {
            log::info!("the overlay will be visible to screen captures: {e:#}");
        }
    }

    /// Work that follows every batch of Wayland events: overlay configures, shortcuts, keymap changes.
    fn after_wayland(&mut self) {
        self.overlay.window.poll(&mut self.wayland);
        for id in std::mem::take(&mut self.wayland.state.shortcuts.pressed) {
            match self.hotkeys.action_for_id(&id) {
                Some(action) => self.send(RawInputEvent::Hotkey(action)),
                None => log::debug!("unknown global shortcut {id}"),
            }
        }
        if self.wayland.state.keymap_serial != self.xkb_serial {
            self.refresh_xkb();
        }
    }

    fn refresh_xkb(&mut self) {
        self.xkb_serial = self.wayland.state.keymap_serial;
        self.xkb = self.wayland.state.keymap.as_deref().and_then(|text| match Xkb::new(text) {
            Ok(xkb) => Some(xkb),
            Err(e) => {
                log::warn!("compiling the keyboard layout failed, letters record as US: {e:#}");
                None
            }
        });
    }

    /// Opens input nodes that appeared since the last scan and forgets the ones that vanished.
    fn rescan_devices(&mut self) {
        self.devices.retain(|path| path.exists());
        for path in input::device_paths() {
            if self.devices.contains(&path) {
                continue;
            }
            self.devices.insert(path.clone());
            let device = match input::open(&path) {
                Ok(device) => device,
                Err(e) => {
                    log::debug!("{e:#}");
                    continue;
                }
            };
            if !input::is_interesting(&device) {
                continue;
            }
            let name = device.name().unwrap_or("unnamed").to_string();
            let decoder = Decoder::new(&device);
            let node = path.clone();
            let source = Generic::new(device, Interest::READ, Mode::Level);
            match self.handle.insert_source(source, move |_, device, service: &mut Service| {
                // SAFETY: the device is only read from, never closed or replaced, so the poll registration stays valid.
                service.read_device(unsafe { device.get_mut() }, &decoder, &node)
            }) {
                Ok(_) => log::info!("reading input from {name} ({})", path.display()),
                Err(e) => log::warn!("cannot watch {}: {e}", path.display()),
            }
        }
    }

    fn read_device(
        &mut self,
        device: &mut Device,
        decoder: &Decoder,
        path: &Path,
    ) -> std::io::Result<PostAction> {
        loop {
            let events = match device.fetch_events() {
                Ok(events) => events,
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(PostAction::Continue),
                Err(e) => {
                    log::info!("input device {} is gone: {e}", path.display());
                    self.devices.remove(path);
                    return Ok(PostAction::Remove);
                }
            };
            let mut any = false;
            let decoded: Vec<InputEvent> =
                events.inspect(|_| any = true).filter_map(|event| decoder.decode(&event)).collect();
            if !any {
                return Ok(PostAction::Continue);
            }
            for event in decoded {
                self.on_input(event);
            }
        }
    }

    fn on_input(&mut self, event: InputEvent) {
        let at = Instant::now();
        match event {
            InputEvent::Key { code, down } => {
                let Some(mut key) = keys::key_from_evdev(code) else { return };
                if let Some(vk) = self.xkb.as_ref().and_then(|xkb| xkb.vk_for_keycode(code)) {
                    key.vk = vk;
                }
                if let Some(action) = self.hotkeys.on_key(key.vk, down) {
                    self.send(RawInputEvent::Hotkey(action));
                }
                if down {
                    self.maybe_interrupt(Some(key.vk));
                }
                self.send(RawInputEvent::Key { key, down, injected: false, own: false, at });
            }
            InputEvent::Button { button, down } => {
                let pos = self.cursor_now();
                if down {
                    self.maybe_interrupt(None);
                }
                self.send(RawInputEvent::Button { button, down, pos, injected: false, own: false, at });
            }
            InputEvent::Wheel { delta, horizontal } => {
                let pos = self.cursor_now();
                self.send(RawInputEvent::Wheel { delta, horizontal, pos, injected: false, own: false, at });
            }
        }
    }

    /// Stops a running player unless we are still inside the grace period or the key is part of a chord.
    fn maybe_interrupt(&self, trigger_vk: Option<u16>) {
        let Some(playback) = &self.playback else { return };
        if trigger_vk.is_some_and(|vk| self.chord_vks.contains(&vk)) {
            return;
        }
        if playback.start.elapsed() >= AUTO_STOP_GRACE {
            playback.ctl.interrupt();
        }
    }

    /// Fresh cursor position, falling back to the last known one when the compositor cannot tell.
    fn cursor_now(&mut self) -> Point {
        if let Some(pos) = self.poll_cursor() {
            self.last_cursor = Some(pos);
            return pos;
        }
        self.last_cursor.unwrap_or_default()
    }

    fn poll_cursor(&self) -> Option<Point> {
        let hyprland = self.hyprland.as_ref()?;
        let (x, y) = match hyprland.cursor_pos() {
            Ok(pos) => pos,
            Err(e) => {
                log::debug!("reading the cursor position failed: {e:#}");
                return None;
            }
        };
        self.wayland.layout().to_physical(x, y)
    }

    fn start_cursor_timer(&mut self) {
        if self.cursor_timer.is_some() || self.hyprland.is_none() {
            return;
        }
        match self.handle.insert_source(Timer::from_duration(CURSOR_POLL), |_, _, service: &mut Service| {
            service.cursor_tick()
        }) {
            Ok(token) => self.cursor_timer = Some(token),
            Err(e) => log::warn!("cannot poll the cursor, mouse moves are not recorded: {e}"),
        }
    }

    fn cursor_tick(&mut self) -> TimeoutAction {
        if !self.recording {
            self.cursor_timer = None;
            return TimeoutAction::Drop;
        }
        if let Some(pos) = self.poll_cursor()
            && self.last_cursor != Some(pos)
        {
            self.last_cursor = Some(pos);
            self.send(RawInputEvent::Move { pos, injected: false, own: false, at: Instant::now() });
        }
        TimeoutAction::ToDuration(CURSOR_POLL)
    }

    fn read_hyprland_events(&mut self, mut stream: &UnixStream) -> std::io::Result<PostAction> {
        let mut buf = [0u8; 4096];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => {
                    log::warn!("hyprland closed the event socket, window changes are no longer tracked");
                    return Ok(PostAction::Remove);
                }
                Ok(n) => self.event_buf.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        while let Some(end) = self.event_buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.event_buf.drain(..=end).collect();
            if let Ok(text) = std::str::from_utf8(&line)
                && let Some((name, data)) = hyprland::parse_event(text)
            {
                self.on_hyprland_event(name, data);
            }
        }
        Ok(PostAction::Continue)
    }

    fn on_hyprland_event(&mut self, name: &str, data: &str) {
        match name {
            "activewindowv2" => self.foreground_changed(data),
            "configreloaded" => {
                log::info!("hyprland reloaded its config, re-applying binds and rules");
                self.hotkeys.reapply(&mut self.wayland);
                self.apply_layer_rule();
            }
            _ => {}
        }
    }

    /// Reports the newly focused window of another process as a `Foreground` event.
    fn foreground_changed(&mut self, address: &str) {
        let Some(handle) = hyprland::parse_address(address) else { return };
        let Some(hyprland) = &self.hyprland else { return };
        let same = |c: &hyprland::Client| hyprland::parse_address(&c.address) == Some(handle);
        let client = match hyprland.active_window() {
            Ok(Some(client)) if same(&client) => client,
            Ok(_) => match hyprland.clients() {
                Ok(clients) => match clients.into_iter().find(same) {
                    Some(client) => client,
                    None => return,
                },
                Err(_) => return,
            },
            Err(e) => {
                log::debug!("reading the active window failed: {e:#}");
                return;
            }
        };
        if client.pid <= 0 || client.pid as u32 == self.own_pid {
            return;
        }
        self.send(RawInputEvent::Foreground {
            hwnd: handle,
            title: client.title.clone(),
            process_name: client.process_name(),
            at: Instant::now(),
        });
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.hotkeys.clear();
        self.overlay.window.hide(&mut self.wayland);
        let _ = self.wayland.flush();
    }
}
