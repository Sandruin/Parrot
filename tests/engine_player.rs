use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use image::{Rgba, RgbaImage};
use macro_recorder::engine::player::{Player, PlayerDeps, ProgressFn};
use macro_recorder::model::{
    Action, ActionItem, ButtonEvent, ImageMatchMode, Key, Macro, MacroSettings, MouseButton, MousePathMode,
    PathPoint, PlaybackOutcome, PlayerControl, Point, Rect, Repeat, TextMatch, TextMode, TimeUnit,
};
use macro_recorder::platform::mock::{
    InjectedCall, MockCapture, MockInjector, MockOcr, MockSleeper, MockWindowManager,
};
use macro_recorder::platform::sleeper::RealSleeper;
use macro_recorder::platform::{CharKey, InputInjector, OcrLine, OcrWord, Sleeper, WindowInfo, WindowRef};

struct Harness {
    injector: Arc<MockInjector>,
    sleeper: Arc<MockSleeper>,
    windows: Arc<MockWindowManager>,
    capture: Arc<MockCapture>,
    ocr: Arc<MockOcr>,
    ctl: Arc<PlayerControl>,
    progress: Arc<Mutex<Vec<(usize, u32)>>>,
}

impl Harness {
    fn new() -> Self {
        Self {
            injector: Arc::new(MockInjector::default()),
            sleeper: Arc::new(MockSleeper::default()),
            windows: Arc::new(MockWindowManager::default()),
            capture: Arc::new(MockCapture::new(RgbaImage::new(64, 64))),
            ocr: Arc::new(MockOcr::default()),
            ctl: PlayerControl::new(),
            progress: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Widens the mock screen so a capture region beyond 64x64 still succeeds.
    fn with_screen(self, width: u32, height: u32) -> Self {
        *self.capture.screen.lock().unwrap() = RgbaImage::new(width, height);
        self
    }

    fn reads(&self, lines: Vec<OcrLine>) {
        *self.ocr.lines.lock().unwrap() = lines;
    }

    fn deps(&self) -> PlayerDeps {
        PlayerDeps {
            injector: self.injector.clone(),
            capture: self.capture.clone(),
            windows: self.windows.clone(),
            sleeper: self.sleeper.clone(),
            ocr: self.ocr.clone(),
        }
    }

    fn progress_fn(&self) -> ProgressFn {
        let slot = self.progress.clone();
        Box::new(move |index, iteration| slot.lock().unwrap().push((index, iteration)))
    }

    fn run(&self, macro_: &Macro) -> PlaybackOutcome {
        self.run_from(macro_, 0)
    }

    fn run_from(&self, macro_: &Macro, start_index: usize) -> PlaybackOutcome {
        Player::new(self.deps(), self.ctl.clone(), self.progress_fn()).run(macro_, start_index)
    }

    fn calls(&self) -> Vec<InjectedCall> {
        self.injector.calls()
    }

    fn progress(&self) -> Vec<(usize, u32)> {
        self.progress.lock().unwrap().clone()
    }
}

fn macro_of(actions: Vec<Action>) -> Macro {
    macro_with(MacroSettings::default(), actions)
}

fn macro_with(settings: MacroSettings, actions: Vec<Action>) -> Macro {
    let items = actions.into_iter().enumerate().map(|(i, a)| ActionItem::new(i as u64 + 1, a)).collect();
    Macro { settings, items, ..Default::default() }
}

fn key(vk: u16) -> Key {
    Key { vk, scancode: 0, extended: false }
}

fn wait(ms: f64) -> Action {
    Action::Wait { duration: ms, unit: TimeUnit::Ms }
}

fn ocr_line(text: &str, words: &[(&str, Rect)]) -> OcrLine {
    OcrLine {
        text: text.into(),
        words: words.iter().map(|(t, rect)| OcrWord { text: (*t).into(), rect: *rect }).collect(),
    }
}

/// Fresh directory under the system temp dir, unique per call so parallel tests cannot collide.
fn unique_temp_dir(tag: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("macro-recorder-{tag}-{stamp}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn png_of(img: &RgbaImage) -> Vec<u8> {
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
    buf
}

#[test]
fn injected_call_order_for_a_small_macro() {
    let h = Harness::new();
    let m = macro_of(vec![
        Action::MouseButton {
            button: MouseButton::Left,
            event: ButtonEvent::Click,
            pos: Some(Point::new(100, 200)),
        },
        Action::KeyPress { key: key(0x41) },
        wait(50.0),
        Action::MouseWheel { delta: -120, horizontal: false, pos: None },
        Action::TypeText { text: "hi".into(), mode: TextMode::Unicode, char_delay_ms: 5 },
    ]);
    assert_eq!(h.run(&m), PlaybackOutcome::Completed);
    assert_eq!(
        h.calls(),
        vec![
            InjectedCall::MoveAbs(Point::new(100, 200)),
            InjectedCall::Button { button: MouseButton::Left, down: true },
            InjectedCall::Button { button: MouseButton::Left, down: false },
            InjectedCall::Key { key: key(0x41), down: true },
            InjectedCall::Key { key: key(0x41), down: false },
            InjectedCall::Wheel { delta: -120, horizontal: false },
            InjectedCall::Unicode { utf16: b'h' as u16, down: true },
            InjectedCall::Unicode { utf16: b'h' as u16, down: false },
            InjectedCall::Unicode { utf16: b'i' as u16, down: true },
            InjectedCall::Unicode { utf16: b'i' as u16, down: false },
        ]
    );
    assert_eq!(h.sleeper.total_slept(), Duration::from_millis(30 + 30 + 50 + 5));
    assert_eq!(h.progress(), vec![(0, 1), (1, 1), (2, 1), (3, 1), (4, 1)]);
}

#[test]
fn double_speed_halves_the_slept_time() {
    let items = vec![wait(100.0), Action::KeyPress { key: key(0x41) }, wait(100.0)];
    let normal = Harness::new();
    assert_eq!(normal.run(&macro_of(items.clone())), PlaybackOutcome::Completed);
    assert_eq!(normal.sleeper.total_slept(), Duration::from_millis(230));

    let fast = Harness::new();
    let settings = MacroSettings { speed_percent: 200, ..Default::default() };
    assert_eq!(fast.run(&macro_with(settings, items)), PlaybackOutcome::Completed);
    assert_eq!(fast.sleeper.total_slept(), Duration::from_millis(115));
}

#[test]
fn disabled_items_and_start_index_are_honoured() {
    let h = Harness::new();
    let mut m = macro_of(vec![
        Action::KeyPress { key: key(0x41) },
        Action::KeyPress { key: key(0x42) },
        Action::KeyPress { key: key(0x43) },
    ]);
    m.items[1].enabled = false;
    assert_eq!(h.run_from(&m, 1), PlaybackOutcome::Completed);
    assert_eq!(
        h.calls(),
        vec![
            InjectedCall::Key { key: key(0x43), down: true },
            InjectedCall::Key { key: key(0x43), down: false },
        ]
    );
    assert_eq!(h.progress(), vec![(2, 1)]);
}

#[test]
fn repeat_count_runs_every_iteration() {
    let h = Harness::new();
    let settings = MacroSettings { repeat: Repeat::Count(3), ..Default::default() };
    let m = macro_with(settings, vec![Action::KeyPress { key: key(0x41) }, wait(10.0)]);
    assert_eq!(h.run(&m), PlaybackOutcome::Completed);
    assert_eq!(h.progress(), vec![(0, 1), (1, 1), (0, 2), (1, 2), (0, 3), (1, 3)]);
    assert_eq!(h.sleeper.total_slept(), Duration::from_millis(120));
}

#[test]
fn infinite_repeat_stops_when_another_thread_requests_it() {
    let h = Harness::new();
    let settings = MacroSettings { repeat: Repeat::Infinite, ..Default::default() };
    let m = macro_with(settings, vec![Action::KeyPress { key: key(0x41) }]);

    let (tx, rx) = crossbeam_channel::bounded::<(usize, u32)>(0);
    let ctl = h.ctl.clone();
    let watcher = std::thread::spawn(move || {
        let mut seen = Vec::new();
        while let Ok(step) = rx.recv() {
            seen.push(step);
            if step.1 >= 3 {
                ctl.request_stop();
            }
        }
        seen
    });

    let mut player = Player::new(
        h.deps(),
        h.ctl.clone(),
        Box::new(move |index, iteration| {
            let _ = tx.send((index, iteration));
        }),
    );
    let outcome = player.run(&m, 0);
    drop(player);
    let seen = watcher.join().unwrap();

    assert_eq!(outcome, PlaybackOutcome::StoppedByUser);
    assert_eq!(seen[..3], [(0, 1), (0, 2), (0, 3)]);
    assert!(seen.len() <= 6, "{seen:?}");
}

#[test]
fn stop_during_a_long_wait_returns_promptly() {
    let h = Harness::new();
    let deps = PlayerDeps { sleeper: Arc::new(RealSleeper::default()), ..h.deps() };
    let m = macro_of(vec![Action::Wait { duration: 10.0, unit: TimeUnit::S }]);
    let ctl = h.ctl.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        ctl.request_stop();
    });
    let started = Instant::now();
    let outcome = Player::new(deps, h.ctl.clone(), h.progress_fn()).run(&m, 0);
    assert_eq!(outcome, PlaybackOutcome::StoppedByUser);
    assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
}

#[test]
fn user_input_interrupt_releases_keys_and_buttons() {
    let h = Harness::new();
    let deps = PlayerDeps { sleeper: Arc::new(RealSleeper::default()), ..h.deps() };
    let m = macro_of(vec![
        Action::KeyDown { key: key(0x41) },
        Action::MouseButton { button: MouseButton::Right, event: ButtonEvent::Down, pos: None },
        Action::Wait { duration: 10.0, unit: TimeUnit::S },
    ]);
    let ctl = h.ctl.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        ctl.interrupt();
    });
    let outcome = Player::new(deps, h.ctl.clone(), h.progress_fn()).run(&m, 0);
    assert_eq!(outcome, PlaybackOutcome::InterruptedByUserInput);
    assert_eq!(
        h.calls(),
        vec![
            InjectedCall::Key { key: key(0x41), down: true },
            InjectedCall::Button { button: MouseButton::Right, down: true },
            InjectedCall::Button { button: MouseButton::Right, down: false },
            InjectedCall::Key { key: key(0x41), down: false },
        ]
    );
}

#[test]
fn mouse_paths_follow_the_path_mode() {
    let path = vec![
        PathPoint { x: 0, y: 0, dt_ms: 0 },
        PathPoint { x: 10, y: 10, dt_ms: 16 },
        PathPoint { x: 20, y: 20, dt_ms: 16 },
    ];
    let recorded = Harness::new();
    assert_eq!(
        recorded.run(&macro_of(vec![Action::MouseMove { path: path.clone() }])),
        PlaybackOutcome::Completed
    );
    assert_eq!(
        recorded.calls(),
        vec![
            InjectedCall::MoveAbs(Point::new(0, 0)),
            InjectedCall::MoveAbs(Point::new(10, 10)),
            InjectedCall::MoveAbs(Point::new(20, 20)),
        ]
    );
    assert_eq!(recorded.sleeper.total_slept(), Duration::from_millis(32));

    let straight = Harness::new();
    let settings = MacroSettings { mouse_path: MousePathMode::Straight, ..Default::default() };
    assert_eq!(
        straight.run(&macro_with(settings, vec![Action::MouseMove { path }])),
        PlaybackOutcome::Completed
    );
    assert_eq!(straight.calls(), vec![InjectedCall::MoveAbs(Point::new(20, 20))]);
    assert_eq!(straight.sleeper.total_slept(), Duration::ZERO);
}

#[test]
fn window_activate_finds_or_fails() {
    let h = Harness::new();
    h.windows.windows.lock().unwrap().push(WindowInfo {
        handle: WindowRef(42),
        title: "Untitled - Notepad".into(),
        process_name: "notepad.exe".into(),
    });
    let found = macro_of(vec![Action::WindowActivate {
        title_contains: "Notepad".into(),
        process_name: String::new(),
        timeout_ms: 5_000,
    }]);
    assert_eq!(h.run(&found), PlaybackOutcome::Completed);
    assert_eq!(*h.windows.activated.lock().unwrap(), vec![WindowRef(42)]);

    let missing = macro_of(vec![
        Action::Comment { text: "before".into() },
        Action::WindowActivate {
            title_contains: "Calculator".into(),
            process_name: String::new(),
            timeout_ms: 1_000,
        },
    ]);
    match h.run(&missing) {
        PlaybackOutcome::Failed { index, error } => {
            assert_eq!(index, 1);
            assert!(error.contains("Calculator"), "{error}");
        }
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn wait_for_image_matches_and_times_out() {
    let mut screen = RgbaImage::from_pixel(64, 64, Rgba([20, 30, 40, 255]));
    for y in 8..24 {
        for x in 8..24 {
            screen.put_pixel(x, y, Rgba([(x * 8) as u8, (y * 8) as u8, 90, 255]));
        }
    }
    let region = Rect::new(8, 8, 16, 16);
    let template = image::imageops::crop_imm(&screen, 8, 8, 16, 16).to_image();

    let h = Harness::new();
    *h.capture.screen.lock().unwrap() = screen;
    let matching = macro_of(vec![Action::WaitForImage {
        region,
        template_png: png_of(&template),
        similarity: 0.99,
        poll_ms: 25,
        timeout_ms: 1_000,
        mode: ImageMatchMode::Exact,
    }]);
    assert_eq!(h.run(&matching), PlaybackOutcome::Completed);
    assert_eq!(h.sleeper.total_slept(), Duration::ZERO);

    let never = macro_of(vec![Action::WaitForImage {
        region,
        template_png: png_of(&RgbaImage::from_pixel(16, 16, Rgba([255, 255, 255, 255]))),
        similarity: 0.99,
        poll_ms: 25,
        timeout_ms: 100,
        mode: ImageMatchMode::Exact,
    }]);
    match h.run(&never) {
        PlaybackOutcome::Failed { index, error } => {
            assert_eq!(index, 0);
            assert!(error.contains("image not found"), "{error}");
        }
        other => panic!("expected a timeout failure, got {other:?}"),
    }
    assert!(h.sleeper.total_slept() >= Duration::from_millis(100));
}

#[test]
fn search_mode_finds_a_template_inside_the_region() {
    let mut screen = RgbaImage::from_pixel(64, 64, Rgba([10, 10, 10, 255]));
    for y in 20..40 {
        for x in 20..40 {
            screen.put_pixel(x, y, Rgba([(x * 6) as u8, 200 - (y * 3) as u8, 30, 255]));
        }
    }
    let template = image::imageops::crop_imm(&screen, 24, 26, 12, 12).to_image();
    let h = Harness::new();
    *h.capture.screen.lock().unwrap() = screen;
    let m = macro_of(vec![Action::WaitForImage {
        region: Rect::new(0, 0, 64, 64),
        template_png: png_of(&template),
        similarity: 0.95,
        poll_ms: 25,
        timeout_ms: 500,
        mode: ImageMatchMode::Search,
    }]);
    assert_eq!(h.run(&m), PlaybackOutcome::Completed);
}

#[test]
fn wait_for_text_matches_and_quotes_the_read_text_on_timeout() {
    let region = Rect::new(0, 0, 48, 16);
    let h = Harness::new();
    h.reads(vec![ocr_line(
        "Export finished",
        &[("Export", Rect::new(2, 2, 20, 8)), ("finished", Rect::new(24, 2, 22, 8))],
    )]);
    let found = macro_of(vec![Action::WaitForText {
        region,
        text: "export FINISHED".into(),
        case_sensitive: false,
        match_mode: TextMatch::Contains,
        poll_ms: 50,
        timeout_ms: 1_000,
    }]);
    assert_eq!(h.run(&found), PlaybackOutcome::Completed);
    assert_eq!(h.sleeper.total_slept(), Duration::ZERO);
    assert_eq!(*h.ocr.calls.lock().unwrap(), 1);

    let never = macro_of(vec![Action::WaitForText {
        region,
        text: "cancelled".into(),
        case_sensitive: false,
        match_mode: TextMatch::Contains,
        poll_ms: 50,
        timeout_ms: 200,
    }]);
    match h.run(&never) {
        PlaybackOutcome::Failed { index, error } => {
            assert_eq!(index, 0);
            assert!(error.contains("cancelled"), "{error}");
            assert!(error.contains("Export finished"), "{error}");
        }
        other => panic!("expected a timeout failure, got {other:?}"),
    }
    assert!(h.sleeper.total_slept() >= Duration::from_millis(200));
}

#[test]
fn a_long_read_is_truncated_in_the_timeout_error() {
    let h = Harness::new();
    let long = "z".repeat(500);
    h.reads(vec![ocr_line(&long, &[(&long, Rect::new(0, 0, 40, 8))])]);
    let m = macro_of(vec![Action::WaitForText {
        region: Rect::new(0, 0, 48, 16),
        text: "ready".into(),
        case_sensitive: false,
        match_mode: TextMatch::Contains,
        poll_ms: 50,
        timeout_ms: 50,
    }]);
    match h.run(&m) {
        PlaybackOutcome::Failed { error, .. } => {
            assert!(error.contains("..."), "{error}");
            assert!(!error.contains(&"z".repeat(220)), "{error}");
        }
        other => panic!("expected a timeout failure, got {other:?}"),
    }
}

#[test]
fn click_on_text_clicks_the_centre_of_the_match_in_screen_pixels() {
    let region = Rect::new(100, 50, 200, 40);
    let h = Harness::new().with_screen(400, 200);
    h.reads(vec![ocr_line(
        "Save As",
        &[("Save", Rect::new(10, 4, 30, 12)), ("As", Rect::new(46, 4, 14, 12))],
    )]);
    let m = macro_of(vec![Action::ClickOnText {
        region,
        text: "save as".into(),
        case_sensitive: false,
        match_mode: TextMatch::Contains,
        button: MouseButton::Right,
        poll_ms: 50,
        timeout_ms: 1_000,
    }]);
    assert_eq!(h.run(&m), PlaybackOutcome::Completed);
    assert_eq!(
        h.calls(),
        vec![
            InjectedCall::MoveAbs(Point::new(135, 60)),
            InjectedCall::Button { button: MouseButton::Right, down: true },
            InjectedCall::Button { button: MouseButton::Right, down: false },
        ]
    );
    assert_eq!(h.sleeper.total_slept(), Duration::from_millis(30));
}

#[test]
fn wait_for_file_sees_an_existing_path_and_times_out_on_a_missing_one() {
    let dir = unique_temp_dir("wait-for-file");
    let present = dir.join("ready.txt");
    std::fs::write(&present, b"ok").unwrap();

    let h = Harness::new();
    let found = macro_of(vec![Action::WaitForFile {
        path: present.to_string_lossy().into_owned(),
        timeout_ms: 1_000,
    }]);
    let found_outcome = h.run(&found);
    let never = macro_of(vec![Action::WaitForFile {
        path: dir.join("never.txt").to_string_lossy().into_owned(),
        timeout_ms: 600,
    }]);
    let never_outcome = h.run(&never);
    let slept = h.sleeper.total_slept();
    std::fs::remove_dir_all(&dir).unwrap();

    assert_eq!(found_outcome, PlaybackOutcome::Completed);
    match never_outcome {
        PlaybackOutcome::Failed { index, error } => {
            assert_eq!(index, 0);
            assert!(error.contains("never.txt"), "{error}");
        }
        other => panic!("expected a timeout failure, got {other:?}"),
    }
    assert_eq!(slept, Duration::from_millis(750));
}

#[test]
fn wait_for_text_in_regex_mode_matches_a_pattern() {
    let h = Harness::new();
    h.reads(vec![ocr_line(
        "Copied 1024 files",
        &[
            ("Copied", Rect::new(2, 2, 24, 8)),
            ("1024", Rect::new(28, 2, 18, 8)),
            ("files", Rect::new(48, 2, 20, 8)),
        ],
    )]);
    let m = macro_of(vec![Action::WaitForText {
        region: Rect::new(0, 0, 48, 16),
        text: r"^Copied \d{3,} files$".into(),
        case_sensitive: false,
        match_mode: TextMatch::Regex,
        poll_ms: 50,
        timeout_ms: 1_000,
    }]);
    assert_eq!(h.run(&m), PlaybackOutcome::Completed);
    assert_eq!(h.sleeper.total_slept(), Duration::ZERO);
    assert_eq!(*h.ocr.calls.lock().unwrap(), 1);

    let never = macro_of(vec![Action::WaitForText {
        region: Rect::new(0, 0, 48, 16),
        text: r"\d{5,}".into(),
        case_sensitive: false,
        match_mode: TextMatch::Regex,
        poll_ms: 50,
        timeout_ms: 200,
    }]);
    match h.run(&never) {
        PlaybackOutcome::Failed { index, error } => {
            assert_eq!(index, 0);
            assert!(error.contains(r"\\d{5,}"), "{error}");
            assert!(error.contains("Copied 1024 files"), "{error}");
        }
        other => panic!("expected a timeout failure, got {other:?}"),
    }
}

#[test]
fn an_invalid_regex_fails_the_action_before_any_polling() {
    let h = Harness::new();
    h.reads(vec![ocr_line("ready", &[("ready", Rect::new(0, 0, 20, 8))])]);
    let m = macro_of(vec![Action::WaitForText {
        region: Rect::new(0, 0, 48, 16),
        text: "(unclosed".into(),
        case_sensitive: false,
        match_mode: TextMatch::Regex,
        poll_ms: 50,
        timeout_ms: 5_000,
    }]);
    match h.run(&m) {
        PlaybackOutcome::Failed { index, error } => {
            assert_eq!(index, 0);
            assert!(error.contains("(unclosed"), "{error}");
        }
        other => panic!("expected an immediate failure, got {other:?}"),
    }
    assert_eq!(h.sleeper.total_slept(), Duration::ZERO);
    assert_eq!(*h.ocr.calls.lock().unwrap(), 0);
}

#[test]
fn click_on_text_in_regex_mode_clicks_the_matched_part_of_the_line() {
    let region = Rect::new(100, 50, 200, 40);
    let h = Harness::new().with_screen(400, 200);
    h.reads(vec![ocr_line(
        "Total: 42 EUR",
        &[
            ("Total:", Rect::new(10, 4, 40, 12)),
            ("42", Rect::new(54, 4, 16, 12)),
            ("EUR", Rect::new(74, 4, 26, 12)),
        ],
    )]);
    let m = macro_of(vec![Action::ClickOnText {
        region,
        text: r"\d+".into(),
        case_sensitive: true,
        match_mode: TextMatch::Regex,
        button: MouseButton::Left,
        poll_ms: 50,
        timeout_ms: 1_000,
    }]);
    assert_eq!(h.run(&m), PlaybackOutcome::Completed);
    assert_eq!(
        h.calls(),
        vec![
            InjectedCall::MoveAbs(Point::new(162, 60)),
            InjectedCall::Button { button: MouseButton::Left, down: true },
            InjectedCall::Button { button: MouseButton::Left, down: false },
        ]
    );
}

#[test]
fn wait_for_file_accepts_a_glob_pattern() {
    let dir = unique_temp_dir("wait-for-file-glob");
    std::fs::write(dir.join("report-2026-09.csv"), b"ok").unwrap();

    let h = Harness::new();
    let found = macro_of(vec![Action::WaitForFile {
        path: dir.join("report-*.csv").to_string_lossy().into_owned(),
        timeout_ms: 1_000,
    }]);
    let found_outcome = h.run(&found);
    let found_slept = h.sleeper.total_slept();

    let never = macro_of(vec![Action::WaitForFile {
        path: dir.join("missing-*.csv").to_string_lossy().into_owned(),
        timeout_ms: 600,
    }]);
    let never_outcome = h.run(&never);
    let slept = h.sleeper.total_slept();
    std::fs::remove_dir_all(&dir).unwrap();

    assert_eq!(found_outcome, PlaybackOutcome::Completed);
    assert_eq!(found_slept, Duration::ZERO);
    match never_outcome {
        PlaybackOutcome::Failed { index, error } => {
            assert_eq!(index, 0);
            assert!(error.contains("missing-*.csv"), "{error}");
        }
        other => panic!("expected a timeout failure, got {other:?}"),
    }
    assert!(slept >= Duration::from_millis(600), "{slept:?}");
}

#[test]
fn an_invalid_glob_pattern_fails_without_waiting() {
    let dir = unique_temp_dir("wait-for-file-bad-glob");
    let h = Harness::new();
    let m = macro_of(vec![Action::WaitForFile {
        path: dir.join("[unterminated").to_string_lossy().into_owned(),
        timeout_ms: 5_000,
    }]);
    let outcome = h.run(&m);
    let slept = h.sleeper.total_slept();
    std::fs::remove_dir_all(&dir).unwrap();

    match outcome {
        PlaybackOutcome::Failed { index, error } => {
            assert_eq!(index, 0);
            assert!(error.contains("[unterminated"), "{error}");
        }
        other => panic!("expected an immediate failure, got {other:?}"),
    }
    assert_eq!(slept, Duration::ZERO);
}

#[test]
fn relative_moves_scale_and_carry_the_rounding_remainder() {
    let steps = vec![
        PathPoint { x: 1, y: 1, dt_ms: 0 },
        PathPoint { x: 1, y: 1, dt_ms: 10 },
        PathPoint { x: 1, y: 1, dt_ms: 10 },
        PathPoint { x: 1, y: 1, dt_ms: 10 },
    ];
    let plain = Harness::new();
    let m = macro_of(vec![Action::MouseMoveRelative { steps: steps.clone(), scale: 1.0 }]);
    assert_eq!(plain.run(&m), PlaybackOutcome::Completed);
    assert_eq!(plain.calls(), vec![InjectedCall::MoveRel { dx: 1, dy: 1 }; 4]);
    assert_eq!(plain.sleeper.total_slept(), Duration::from_millis(30));
    assert_eq!(plain.injector.cursor_pos().unwrap(), Point::new(4, 4));

    let scaled = Harness::new();
    let m = macro_of(vec![Action::MouseMoveRelative { steps, scale: 1.5 }]);
    assert_eq!(scaled.run(&m), PlaybackOutcome::Completed);
    assert_eq!(
        scaled.calls(),
        vec![
            InjectedCall::MoveRel { dx: 2, dy: 2 },
            InjectedCall::MoveRel { dx: 1, dy: 1 },
            InjectedCall::MoveRel { dx: 2, dy: 2 },
            InjectedCall::MoveRel { dx: 1, dy: 1 },
        ]
    );
    assert_eq!(scaled.injector.cursor_pos().unwrap(), Point::new(6, 6));
}

#[test]
fn relative_steps_that_round_to_nothing_are_skipped() {
    let h = Harness::new();
    let steps = vec![
        PathPoint { x: 0, y: 0, dt_ms: 0 },
        PathPoint { x: 0, y: 0, dt_ms: 15 },
        PathPoint { x: -3, y: 4, dt_ms: 15 },
    ];
    let m = macro_of(vec![Action::MouseMoveRelative { steps, scale: 1.0 }]);
    assert_eq!(h.run(&m), PlaybackOutcome::Completed);
    assert_eq!(h.calls(), vec![InjectedCall::MoveRel { dx: -3, dy: 4 }]);
    assert_eq!(h.sleeper.total_slept(), Duration::from_millis(30));

    let small = Harness::new();
    let steps = vec![
        PathPoint { x: 1, y: 0, dt_ms: 0 },
        PathPoint { x: 1, y: 0, dt_ms: 5 },
        PathPoint { x: 1, y: 0, dt_ms: 5 },
    ];
    let m = macro_of(vec![Action::MouseMoveRelative { steps, scale: 0.4 }]);
    assert_eq!(small.run(&m), PlaybackOutcome::Completed);
    assert_eq!(small.calls(), vec![InjectedCall::MoveRel { dx: 1, dy: 0 }]);
}

/// Injector with a keyboard layout, so scan-code typing and its fallback are both exercised.
struct LayoutInjector {
    inner: MockInjector,
}

impl InputInjector for LayoutInjector {
    fn key(&self, k: Key, down: bool) -> Result<()> {
        self.inner.key(k, down)
    }

    fn unicode(&self, utf16: u16, down: bool) -> Result<()> {
        self.inner.unicode(utf16, down)
    }

    fn mouse_move_abs(&self, pos: Point) -> Result<()> {
        self.inner.mouse_move_abs(pos)
    }

    fn mouse_move_rel(&self, dx: i32, dy: i32) -> Result<()> {
        self.inner.mouse_move_rel(dx, dy)
    }

    fn mouse_button(&self, button: MouseButton, down: bool) -> Result<()> {
        self.inner.mouse_button(button, down)
    }

    fn mouse_wheel(&self, delta: i32, horizontal: bool) -> Result<()> {
        self.inner.mouse_wheel(delta, horizontal)
    }

    fn cursor_pos(&self) -> Result<Point> {
        self.inner.cursor_pos()
    }

    fn key_for_char(&self, ch: char) -> Option<CharKey> {
        let a = Key { vk: 0x41, scancode: 0x1E, extended: false };
        match ch {
            'a' => Some(CharKey { key: a, shift: false, ctrl: false, alt: false }),
            'A' => Some(CharKey { key: a, shift: true, ctrl: false, alt: false }),
            _ => None,
        }
    }
}

#[test]
fn scan_code_typing_holds_modifiers_and_falls_back_to_unicode() {
    let injector = Arc::new(LayoutInjector { inner: MockInjector::default() });
    let sleeper: Arc<dyn Sleeper> = Arc::new(MockSleeper::default());
    let deps = PlayerDeps {
        injector: injector.clone(),
        capture: Arc::new(MockCapture::new(RgbaImage::new(8, 8))),
        windows: Arc::new(MockWindowManager::default()),
        sleeper,
        ocr: Arc::new(MockOcr::default()),
    };
    let m =
        macro_of(vec![Action::TypeText { text: "aA?".into(), mode: TextMode::ScanCodes, char_delay_ms: 0 }]);
    let outcome = Player::new(deps, PlayerControl::new(), Box::new(|_, _| {})).run(&m, 0);
    assert_eq!(outcome, PlaybackOutcome::Completed);

    let a = Key { vk: 0x41, scancode: 0x1E, extended: false };
    let shift = Key { vk: 0x10, scancode: 0, extended: false };
    assert_eq!(
        injector.inner.calls(),
        vec![
            InjectedCall::Key { key: a, down: true },
            InjectedCall::Key { key: a, down: false },
            InjectedCall::Key { key: shift, down: true },
            InjectedCall::Key { key: a, down: true },
            InjectedCall::Key { key: a, down: false },
            InjectedCall::Key { key: shift, down: false },
            InjectedCall::Unicode { utf16: b'?' as u16, down: true },
            InjectedCall::Unicode { utf16: b'?' as u16, down: false },
        ]
    );
}

#[test]
fn comments_and_labels_do_nothing() {
    let h = Harness::new();
    let m = macro_of(vec![Action::Label { name: "start".into() }, Action::Comment { text: "note".into() }]);
    assert_eq!(h.run(&m), PlaybackOutcome::Completed);
    assert!(h.calls().is_empty());
    assert_eq!(h.sleeper.total_slept(), Duration::ZERO);
}
