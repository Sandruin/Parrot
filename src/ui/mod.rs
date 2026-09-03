pub mod action_list;
pub mod fake_engine;
pub mod files;
pub mod keymap;
pub mod overlay_scene;
pub mod properties;
pub mod region_picker;
pub mod settings_window;
pub mod status_bar;
pub mod style;
pub mod toolbar;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::engine::EngineHandle;
use crate::model::{
    Action, ActionId, AppSettings, EngineCommand, EngineEvent, HotkeyAction, Macro, PlaybackOutcome, Point,
};
use crate::platform::{InputInjector, ScreenCapture, WindowManager};

/// How often the foreground window's elevation is re-checked.
const ELEVATION_INTERVAL: Duration = Duration::from_secs(1);

/// Delay between hiding the overlay and screenshotting for the region picker, so it is not in the shot.
const PICK_DELAY: Duration = Duration::from_millis(150);

/// How often the cursor is re-read while a cursor-anchored overlay is up.
const CURSOR_INTERVAL: Duration = Duration::from_millis(100);

/// Platform services the GUI itself needs, cloned from the ones the engine runs on.
#[derive(Clone)]
pub struct UiServices {
    pub injector: Arc<dyn InputInjector>,
    pub capture: Arc<dyn ScreenCapture>,
    pub windows: Arc<dyn WindowManager>,
}

impl UiServices {
    /// Where the mouse cursor is right now, in physical virtual-screen pixels.
    pub fn cursor_pos(&self) -> Point {
        self.injector.cursor_pos().unwrap_or_default()
    }
}

/// What the app is currently doing; drives which buttons are enabled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Idle,
    Recording,
    Playing,
}

impl Mode {
    pub fn label(&self) -> &'static str {
        match self {
            Mode::Idle => "Idle",
            Mode::Recording => "Recording",
            Mode::Playing => "Playing",
        }
    }

    pub fn is_busy(&self) -> bool {
        !matches!(self, Mode::Idle)
    }
}

/// Status bar message with a severity so failures can be shown in the error colour.
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub text: String,
    pub error: bool,
}

/// Where playback currently is, mirrored from the progress events.
#[derive(Clone, Copy, Debug)]
pub struct Progress {
    pub index: usize,
    pub total: usize,
    pub iteration: u32,
}

pub struct App {
    pub doc: Macro,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub selected: Option<ActionId>,
    pub mode: Mode,
    pub settings: AppSettings,
    pub engine: EngineHandle,
    pub services: UiServices,
    pub status: Status,
    pub progress: Option<Progress>,
    /// Item the player is currently executing, highlighted in the list.
    pub running: Option<ActionId>,
    pub dialog: Option<properties::Dialog>,
    pub settings_open: bool,
    pub hotkey_capture: Option<HotkeyAction>,
    /// Item whose comment cell is being edited inline.
    pub editing_comment: Option<ActionId>,
    /// Item the list should scroll to on the next frame.
    pub scroll_to: Option<ActionId>,
    pub confirm: Option<files::Pending>,
    /// Armed while the fullscreen region picker viewport is up.
    pub region_picker: Option<region_picker::Picker>,
    /// Region pick waiting for the overlay to disappear before the screenshot is taken.
    region_pick_pending: Option<(ActionId, Instant)>,
    /// Whether we ourselves run elevated, checked once at startup.
    pub elevated: bool,
    /// Set while the foreground window is elevated and we are not.
    pub elevation_warning: bool,
    elevation_checked: Option<Instant>,
    overlay_sent: Option<Action>,
    /// Cursor position the overlay scene currently on screen was drawn from.
    overlay_anchor: Option<Point>,
    anchor_checked: Option<Instant>,
    title: String,
    wake_until: Option<Instant>,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        engine: EngineHandle,
        settings: AppSettings,
        services: UiServices,
    ) -> Self {
        style::apply(&cc.egui_ctx);
        let demo = std::env::var_os("MACRO_DEMO_DOC").is_some();
        Self {
            doc: if demo { fake_engine::demo_doc() } else { Macro::default() },
            path: None,
            dirty: false,
            selected: None,
            mode: Mode::Idle,
            settings,
            engine,
            services,
            status: Status { text: "Ready".into(), error: false },
            progress: None,
            running: None,
            dialog: None,
            settings_open: false,
            hotkey_capture: None,
            editing_comment: None,
            scroll_to: None,
            confirm: None,
            region_picker: None,
            region_pick_pending: None,
            elevated: self_is_elevated(),
            elevation_warning: false,
            elevation_checked: None,
            overlay_sent: None,
            overlay_anchor: None,
            anchor_checked: None,
            title: String::new(),
            wake_until: None,
        }
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.status = Status { text: text.into(), error: false };
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.status = Status { text: text.into(), error: true };
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected.and_then(|id| self.doc.index_of(id))
    }

    /// Inserts an action after the selection (or at the end), selects it and opens its properties.
    pub fn add_action(&mut self, action: Action) {
        let index = self.selected_index().map_or(self.doc.items.len(), |i| i + 1);
        let id = self.doc.insert(index, action);
        self.dirty = true;
        self.select(Some(id));
        self.scroll_to = Some(id);
        self.open_properties();
    }

    pub fn select(&mut self, id: Option<ActionId>) {
        if self.selected != id {
            self.editing_comment = None;
        }
        self.selected = id;
    }

    pub fn open_properties(&mut self) {
        if let Some(item) = self.selected.and_then(|id| self.doc.item(id)) {
            self.dialog = Some(properties::Dialog::new(item));
        }
    }

    pub fn duplicate_selected(&mut self) {
        if let Some(new_id) = self.selected.and_then(|id| self.doc.duplicate(id)) {
            self.dirty = true;
            self.select(Some(new_id));
            self.scroll_to = Some(new_id);
        }
    }

    pub fn delete_selected(&mut self) {
        let Some(id) = self.selected else { return };
        let index = self.doc.index_of(id);
        if self.doc.remove(id).is_some() {
            self.dirty = true;
            let next = index
                .and_then(|i| self.doc.items.get(i).or_else(|| self.doc.items.last()))
                .map(|item| item.id);
            self.select(next);
        }
    }

    pub fn move_selected(&mut self, delta: isize) {
        if let Some(id) = self.selected
            && self.doc.shift(id, delta)
        {
            self.dirty = true;
            self.scroll_to = Some(id);
        }
    }

    pub fn start_recording(&mut self) {
        self.engine.send(EngineCommand::StartRecording(self.settings.record.clone()));
        self.info("Starting recording");
        self.wake();
    }

    pub fn toggle_record(&mut self) {
        match self.mode {
            Mode::Recording => {
                self.engine.send(EngineCommand::StopRecording);
                self.wake();
            }
            Mode::Playing => {}
            Mode::Idle => self.start_recording(),
        }
    }

    /// Starts playback, from the selected item when `from_selection` is set.
    pub fn start_playback(&mut self, from_selection: bool) {
        if self.doc.items.is_empty() {
            self.error("Nothing to play, the macro is empty");
            return;
        }
        let start_index = if from_selection { self.selected_index().unwrap_or(0) } else { 0 };
        self.engine.send(EngineCommand::Play { macro_: Arc::new(self.doc.clone()), start_index });
        self.info(if start_index == 0 {
            "Starting playback".to_string()
        } else {
            format!("Starting playback at {}", start_index + 1)
        });
        self.wake();
    }

    pub fn toggle_play(&mut self, from_selection: bool) {
        match self.mode {
            Mode::Playing => {
                self.engine.send(EngineCommand::StopPlayback);
                self.wake();
            }
            Mode::Recording => {}
            Mode::Idle => self.start_playback(from_selection),
        }
    }

    pub fn stop(&mut self) {
        match self.mode {
            Mode::Recording => self.engine.send(EngineCommand::StopRecording),
            Mode::Playing => self.engine.send(EngineCommand::StopPlayback),
            Mode::Idle => return,
        }
        self.wake();
    }

    /// Keeps the frame loop running for a moment so engine answers arrive without user input.
    fn wake(&mut self) {
        self.wake_until = Some(Instant::now() + Duration::from_secs(2));
    }

    fn handle_event(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::RecordingStarted => {
                self.mode = Mode::Recording;
                self.info("Recording");
            }
            EngineEvent::Recorded(item) => {
                let (comment, enabled) = (item.comment, item.enabled);
                let id = self.doc.push(item.action);
                if let Some(new) = self.doc.item_mut(id) {
                    new.comment = comment;
                    new.enabled = enabled;
                }
                self.dirty = true;
                self.select(Some(id));
                self.scroll_to = Some(id);
            }
            EngineEvent::RecordingStopped => {
                self.mode = Mode::Idle;
                self.info(format!("Recorded {} actions", self.doc.items.len()));
            }
            EngineEvent::PlaybackStarted { total } => {
                self.mode = Mode::Playing;
                self.progress = Some(Progress { index: 0, total, iteration: 1 });
                self.info("Playing");
            }
            EngineEvent::PlaybackProgress { index, iteration } => {
                let total = self.progress.map_or(self.doc.items.len(), |p| p.total);
                self.progress = Some(Progress { index, total, iteration });
                self.running = self.doc.items.get(index).map(|item| item.id);
                self.scroll_to = self.running;
            }
            EngineEvent::PlaybackFinished(outcome) => {
                self.mode = Mode::Idle;
                self.running = None;
                self.progress = None;
                match outcome {
                    PlaybackOutcome::Completed => self.info("Playback completed"),
                    PlaybackOutcome::StoppedByUser => self.info("Playback stopped"),
                    PlaybackOutcome::InterruptedByUserInput => {
                        self.info("Playback interrupted by user input")
                    }
                    PlaybackOutcome::Failed { index, error } => {
                        self.error(format!("Action {} failed: {error}", index + 1))
                    }
                }
            }
            EngineEvent::HotkeyPressed(action) => match action {
                HotkeyAction::ToggleRecord => self.toggle_record(),
                HotkeyAction::TogglePlay => self.toggle_play(false),
                HotkeyAction::Stop => self.stop(),
            },
            EngineEvent::Error(msg) => self.error(msg),
        }
    }

    /// Mirrors the selection to the overlay, hiding it while recording or playing.
    fn sync_overlay(&mut self) {
        let picking = self.region_pick_pending.is_some() || self.region_picker.is_some();
        let wanted = if self.settings.show_overlay && !self.mode.is_busy() && !picking {
            self.selected
                .and_then(|id| self.doc.item(id))
                .filter(|item| item.action.is_positional())
                .map(|item| item.action.clone())
        } else {
            None
        };
        if wanted == self.overlay_sent {
            return;
        }
        match &wanted {
            Some(action) => {
                let cursor = self.services.cursor_pos();
                self.overlay_anchor = Some(cursor);
                self.engine.send(EngineCommand::ShowOverlay(overlay_scene::for_action_from(action, cursor)));
            }
            None => {
                self.overlay_anchor = None;
                self.engine.send(EngineCommand::HideOverlay);
            }
        }
        self.anchor_checked = Some(Instant::now());
        self.overlay_sent = wanted;
    }

    /// Redraws the relative-move overlay as the cursor wanders, since its path starts there.
    fn follow_cursor(&mut self, ctx: &egui::Context) {
        let Some(action @ Action::MouseMoveRelative { .. }) = self.overlay_sent.clone() else {
            return;
        };
        ctx.request_repaint_after(CURSOR_INTERVAL);
        if self.anchor_checked.is_some_and(|at| at.elapsed() < CURSOR_INTERVAL) {
            return;
        }
        self.anchor_checked = Some(Instant::now());
        let cursor = self.services.cursor_pos();
        if self.overlay_anchor.is_some_and(|anchor| !overlay_scene::cursor_moved(anchor, cursor)) {
            return;
        }
        self.overlay_anchor = Some(cursor);
        self.engine.send(EngineCommand::ShowOverlay(overlay_scene::for_action_from(&action, cursor)));
    }

    fn sync_title(&mut self, ctx: &egui::Context) {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".to_string());
        let title = format!("{}{name} - Macro Recorder", if self.dirty { "*" } else { "" });
        if title != self.title {
            self.title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }

    /// Re-checks once per second whether input would be swallowed by an elevated foreground window.
    fn poll_elevation(&mut self) {
        if self.elevation_checked.is_some_and(|at| at.elapsed() < ELEVATION_INTERVAL) {
            return;
        }
        self.elevation_checked = Some(Instant::now());
        self.elevation_warning = !self.elevated && foreground_is_elevated(&self.services);
    }

    /// Starts the region picker for the action the properties dialog is editing, once the overlay is gone.
    pub fn pick_region(&mut self, target: ActionId) {
        self.region_pick_pending = Some((target, Instant::now()));
    }

    /// Takes the region picker screenshot after the overlay had a moment to disappear.
    fn poll_region_pick(&mut self, ctx: &egui::Context) {
        let Some((target, since)) = self.region_pick_pending else { return };
        if since.elapsed() < PICK_DELAY {
            ctx.request_repaint_after(PICK_DELAY);
            return;
        }
        self.region_pick_pending = None;
        region_picker::open(self, target);
    }

    /// True while a modal or a text field owns the keyboard, so list shortcuts stay quiet.
    pub fn keyboard_busy(&self, ctx: &egui::Context) -> bool {
        self.dialog.is_some()
            || self.confirm.is_some()
            || self.region_picker.is_some()
            || self.hotkey_capture.is_some()
            || self.editing_comment.is_some()
            || ctx.egui_wants_keyboard_input()
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for event in self.engine.drain() {
            self.handle_event(event);
        }
        self.sync_overlay();
        self.poll_region_pick(ctx);
        self.follow_cursor(ctx);
        self.sync_title(ctx);
        self.poll_elevation();

        let waking = self.wake_until.is_some_and(|t| Instant::now() < t);
        if self.mode.is_busy() || waking {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else {
            self.wake_until = None;
        }
        if self.region_picker.is_some() {
            ctx.request_repaint();
        }
        ctx.request_repaint_after(ELEVATION_INTERVAL);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        files::shortcuts(self, &ctx);

        let chrome = style::chrome_frame(ui.style());
        egui::Panel::top("toolbar").frame(chrome).show(ui, |ui| toolbar::show(self, ui));
        egui::Panel::bottom("status").frame(chrome).show(ui, |ui| status_bar::show(self, ui));
        egui::CentralPanel::default().show(ui, |ui| action_list::show(self, ui));

        properties::show(self, &ctx);
        region_picker::show(self, &ctx);
        settings_window::show(self, &ctx);
        files::show_confirm(self, &ctx);
    }
}

#[cfg(windows)]
fn self_is_elevated() -> bool {
    crate::platform::win32::elevation::current_is_elevated()
}

#[cfg(not(windows))]
fn self_is_elevated() -> bool {
    false
}

#[cfg(windows)]
fn foreground_is_elevated(services: &UiServices) -> bool {
    services
        .windows
        .foreground()
        .and_then(|window| crate::platform::win32::elevation::window_is_elevated(window.handle.0))
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn foreground_is_elevated(_services: &UiServices) -> bool {
    false
}

/// File name of our own executable, used to skip ourselves when reading the foreground window.
pub fn own_process_name() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        std::env::current_exe()
            .ok()
            .and_then(|path| path.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_default()
    })
}
