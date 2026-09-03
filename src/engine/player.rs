use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use image::RgbaImage;

use crate::engine::matcher;
use crate::engine::scheduler::Scheduler;
use crate::model::{
    Action, ButtonEvent, ImageMatchMode, Key, Macro, MacroSettings, MouseButton, MousePathMode,
    PlaybackOutcome, PlayerControl, Rect, Repeat, TextMode, vk,
};
use crate::platform::{CharKey, InputInjector, Ocr, ScreenCapture, Sleeper, WaitResult, WindowManager};

/// Reports the item index and the 1-based repeat iteration currently running.
pub type ProgressFn = Box<dyn FnMut(usize, u32) + Send>;

const CLICK_HOLD_MS: f64 = 30.0;

/// Everything a playback needs from the platform layer.
#[derive(Clone)]
pub struct PlayerDeps {
    pub injector: Arc<dyn InputInjector>,
    pub capture: Arc<dyn ScreenCapture>,
    pub windows: Arc<dyn WindowManager>,
    pub sleeper: Arc<dyn Sleeper>,
    pub ocr: Arc<dyn Ocr>,
}

/// Executes one macro against the platform traits, releasing everything it pressed when it ends.
pub struct Player {
    deps: PlayerDeps,
    ctl: Arc<PlayerControl>,
    progress: ProgressFn,
    pressed_keys: Vec<Key>,
    pressed_buttons: Vec<MouseButton>,
}

enum Flow {
    Continue,
    Stopped,
}

/// The polling parameters of a `WaitForImage` action, without the encoded template.
#[derive(Clone, Copy)]
struct ImageWait {
    region: Rect,
    similarity: f32,
    poll_ms: u32,
    timeout_ms: u32,
    mode: ImageMatchMode,
}

impl Player {
    pub fn new(deps: PlayerDeps, ctl: Arc<PlayerControl>, progress: ProgressFn) -> Self {
        Self { deps, ctl, progress, pressed_keys: Vec::new(), pressed_buttons: Vec::new() }
    }

    /// Runs the macro on a new thread and hands the outcome to `on_finish`.
    pub fn spawn(
        deps: PlayerDeps,
        ctl: Arc<PlayerControl>,
        macro_: Arc<Macro>,
        start_index: usize,
        progress: ProgressFn,
        on_finish: Box<dyn FnOnce(PlaybackOutcome) + Send>,
    ) -> std::io::Result<JoinHandle<()>> {
        std::thread::Builder::new().name("player".into()).spawn(move || {
            let outcome = Player::new(deps, ctl, progress).run(&macro_, start_index);
            on_finish(outcome);
        })
    }

    /// Plays `macro_` from `start_index` and always leaves the input state clean.
    pub fn run(&mut self, macro_: &Macro, start_index: usize) -> PlaybackOutcome {
        let outcome = self.play(macro_, start_index);
        self.release_all();
        outcome
    }

    fn play(&mut self, macro_: &Macro, start_index: usize) -> PlaybackOutcome {
        let settings = &macro_.settings;
        if !macro_.items.iter().any(|item| item.enabled) {
            return PlaybackOutcome::Completed;
        }
        let mut sched = Scheduler::new(self.deps.sleeper.clone(), settings.speed_factor());
        let mut index = start_index.min(macro_.items.len());
        let mut iteration: u32 = 0;
        loop {
            iteration = iteration.saturating_add(1);
            while index < macro_.items.len() {
                let item = &macro_.items[index];
                if !item.enabled {
                    index += 1;
                    continue;
                }
                if self.ctl.is_stopped() {
                    return self.stopped();
                }
                (self.progress)(index, iteration);
                match self.exec(&item.action, settings, &mut sched) {
                    Ok(Flow::Continue) => {}
                    Ok(Flow::Stopped) => return self.stopped(),
                    Err(error) => {
                        return PlaybackOutcome::Failed { index, error: format!("{error:#}") };
                    }
                }
                index += 1;
            }
            if self.ctl.is_stopped() {
                return self.stopped();
            }
            match settings.repeat {
                Repeat::Count(n) if iteration >= n.max(1) => return PlaybackOutcome::Completed,
                _ => index = 0,
            }
        }
    }

    fn stopped(&self) -> PlaybackOutcome {
        if self.ctl.is_interrupted() {
            PlaybackOutcome::InterruptedByUserInput
        } else {
            PlaybackOutcome::StoppedByUser
        }
    }

    fn exec(&mut self, action: &Action, settings: &MacroSettings, sched: &mut Scheduler) -> Result<Flow> {
        match action {
            Action::Wait { .. } => {
                let ms = action.wait_millis().unwrap_or(0.0);
                return Ok(self.wait(ms, sched));
            }
            Action::KeyDown { key } => self.key(*key, true)?,
            Action::KeyUp { key } => self.key(*key, false)?,
            Action::KeyPress { key } => {
                self.key(*key, true)?;
                if let Flow::Stopped = self.wait(CLICK_HOLD_MS, sched) {
                    return Ok(Flow::Stopped);
                }
                self.key(*key, false)?;
            }
            Action::TypeText { text, mode, char_delay_ms } => {
                for (i, ch) in text.chars().enumerate() {
                    if i > 0
                        && let Flow::Stopped = self.wait(*char_delay_ms as f64, sched)
                    {
                        return Ok(Flow::Stopped);
                    }
                    match mode {
                        TextMode::Unicode => self.type_unicode(ch)?,
                        TextMode::ScanCodes => match self.deps.injector.key_for_char(ch) {
                            Some(chord) => self.type_chord(chord)?,
                            None => self.type_unicode(ch)?,
                        },
                    }
                }
            }
            Action::MouseMove { path } => {
                if settings.mouse_path == MousePathMode::Straight || path.len() == 1 {
                    if let Some(last) = path.last() {
                        self.deps.injector.mouse_move_abs(last.pos())?;
                    }
                } else {
                    for (i, point) in path.iter().enumerate() {
                        if i > 0
                            && let Flow::Stopped = self.wait(point.dt_ms as f64, sched)
                        {
                            return Ok(Flow::Stopped);
                        }
                        self.deps.injector.mouse_move_abs(point.pos())?;
                    }
                }
            }
            Action::MouseButton { button, event, pos } => {
                if let Some(pos) = pos {
                    self.deps.injector.mouse_move_abs(*pos)?;
                }
                match event {
                    ButtonEvent::Down => self.button(*button, true)?,
                    ButtonEvent::Up => self.button(*button, false)?,
                    ButtonEvent::Click => {
                        self.button(*button, true)?;
                        if let Flow::Stopped = self.wait(CLICK_HOLD_MS, sched) {
                            return Ok(Flow::Stopped);
                        }
                        self.button(*button, false)?;
                    }
                }
            }
            Action::MouseWheel { delta, horizontal, pos } => {
                if let Some(pos) = pos {
                    self.deps.injector.mouse_move_abs(*pos)?;
                }
                self.deps.injector.mouse_wheel(*delta, *horizontal)?;
            }
            Action::WindowActivate { title_contains, process_name, timeout_ms } => {
                let window = self.deps.windows.find(title_contains, process_name).with_context(|| {
                    format!("no window matching title {title_contains:?} and process {process_name:?}")
                })?;
                self.deps
                    .windows
                    .activate(window, Duration::from_millis(*timeout_ms as u64))
                    .context("activating window")?;
                sched.resync();
            }
            Action::WaitForImage { region, template_png, similarity, poll_ms, timeout_ms, mode } => {
                let template =
                    image::load_from_memory(template_png).context("decoding image template")?.to_rgba8();
                let spec = ImageWait {
                    region: *region,
                    similarity: *similarity,
                    poll_ms: *poll_ms,
                    timeout_ms: *timeout_ms,
                    mode: *mode,
                };
                return self.wait_for_image(&spec, &template, sched);
            }
            Action::WaitForText { .. } => bail!("wait for text is not supported yet"),
            Action::ClickOnText { .. } => bail!("click on text is not supported yet"),
            Action::MouseMoveRelative { .. } => bail!("relative mouse moves are not supported yet"),
            Action::WaitForFile { .. } => bail!("wait for file is not supported yet"),
            Action::Comment { .. } | Action::Label { .. } => {}
        }
        Ok(Flow::Continue)
    }

    fn wait_for_image(
        &mut self,
        spec: &ImageWait,
        template: &RgbaImage,
        sched: &mut Scheduler,
    ) -> Result<Flow> {
        let ImageWait { region, similarity, poll_ms, timeout_ms, mode } = *spec;
        sched.resync();
        let start = sched.now();
        let timeout = Duration::from_millis(timeout_ms as u64);
        let mut best = 0.0f32;
        loop {
            if self.ctl.is_stopped() {
                return Ok(Flow::Stopped);
            }
            let shot = self.deps.capture.capture(region).context("capturing region")?;
            let (matched, score) = matcher::evaluate(mode, &shot, template, similarity);
            best = best.max(score);
            if matched {
                sched.resync();
                return Ok(Flow::Continue);
            }
            if timeout_ms > 0 && sched.now().saturating_duration_since(start) >= timeout {
                bail!(
                    "image not found within {} ms, best similarity {:.1}% of required {:.1}%",
                    timeout_ms,
                    best * 100.0,
                    similarity * 100.0
                );
            }
            if let Flow::Stopped = self.wait(poll_ms.max(1) as f64, sched) {
                return Ok(Flow::Stopped);
            }
        }
    }

    fn wait(&self, ms: f64, sched: &mut Scheduler) -> Flow {
        match sched.wait(ms, &self.ctl) {
            WaitResult::Elapsed => Flow::Continue,
            WaitResult::Stopped => Flow::Stopped,
        }
    }

    fn type_unicode(&self, ch: char) -> Result<()> {
        let mut buf = [0u16; 2];
        for unit in ch.encode_utf16(&mut buf) {
            self.deps.injector.unicode(*unit, true)?;
            self.deps.injector.unicode(*unit, false)?;
        }
        Ok(())
    }

    fn type_chord(&mut self, chord: CharKey) -> Result<()> {
        let mods = [(chord.shift, vk::SHIFT), (chord.ctrl, vk::CONTROL), (chord.alt, vk::MENU)];
        for (held, code) in mods {
            if held {
                self.key(Key::from_vk(code), true)?;
            }
        }
        self.key(chord.key, true)?;
        self.key(chord.key, false)?;
        for (held, code) in mods.iter().rev() {
            if *held {
                self.key(Key::from_vk(*code), false)?;
            }
        }
        Ok(())
    }

    fn key(&mut self, key: Key, down: bool) -> Result<()> {
        self.deps.injector.key(key, down)?;
        if down {
            if !self.pressed_keys.iter().any(|k| k.vk == key.vk) {
                self.pressed_keys.push(key);
            }
        } else {
            self.pressed_keys.retain(|k| k.vk != key.vk);
        }
        Ok(())
    }

    fn button(&mut self, button: MouseButton, down: bool) -> Result<()> {
        self.deps.injector.mouse_button(button, down)?;
        if down {
            if !self.pressed_buttons.contains(&button) {
                self.pressed_buttons.push(button);
            }
        } else {
            self.pressed_buttons.retain(|b| *b != button);
        }
        Ok(())
    }

    /// Releases every key and button still held, whatever ended the playback.
    fn release_all(&mut self) {
        for button in std::mem::take(&mut self.pressed_buttons).into_iter().rev() {
            let _ = self.deps.injector.mouse_button(button, false);
        }
        for key in std::mem::take(&mut self.pressed_keys).into_iter().rev() {
            let _ = self.deps.injector.key(key, false);
        }
    }
}
