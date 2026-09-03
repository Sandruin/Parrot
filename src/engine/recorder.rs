use std::time::{Duration, Instant};

use crate::model::{
    Action, ButtonEvent, Key, MouseButton, PathPoint, Point, RawInputEvent, RecordOptions, TimeUnit,
};

/// Longest gap that still folds a down/up pair into a click or key press.
const FOLD_WINDOW: Duration = Duration::from_millis(300);
/// A gap longer than this closes the open mouse path.
const PATH_GAP: Duration = Duration::from_millis(1_000);
const MIN_SAMPLE_DT_MS: u32 = 8;
const MIN_SAMPLE_DIST_SQ: i64 = 4;
const MAX_PATH_POINTS: usize = 5_000;
const CLICK_TOLERANCE_SQ: i64 = 9;

/// Turns a raw input stream into editable actions. Pure: all timing comes from the events.
pub struct Recorder {
    opts: RecordOptions,
    chord_vks: Vec<u16>,
    pending: Option<Pending>,
    open_path: Option<OpenPath>,
    last_emit: Option<Instant>,
    last_cursor: Option<Point>,
}

/// A down event held back until it is clear whether a matching up follows in time.
enum Pending {
    Key { key: Key, at: Instant },
    Button { button: MouseButton, pos: Point, at: Instant, held: Vec<(Point, Instant)> },
}

struct OpenPath {
    points: Vec<PathPoint>,
    start: Instant,
    last_pos: Point,
    last_at: Instant,
    /// Cursor position before the path started, the reference for the first relative step.
    origin: Option<Point>,
}

impl OpenPath {
    fn new(pos: Point, at: Instant, origin: Option<Point>) -> Self {
        Self {
            points: vec![PathPoint { x: pos.x, y: pos.y, dt_ms: 0 }],
            start: at,
            last_pos: pos,
            last_at: at,
            origin,
        }
    }
}

enum MoveStep {
    Dropped,
    Appended { full: bool },
    Start,
    Restart,
}

impl Recorder {
    pub fn new(opts: RecordOptions, chord_vks: Vec<u16>) -> Self {
        Self { opts, chord_vks, pending: None, open_path: None, last_emit: None, last_cursor: None }
    }

    pub fn options(&self) -> &RecordOptions {
        &self.opts
    }

    /// Consumes one raw event and returns the actions it completed.
    pub fn feed(&mut self, event: RawInputEvent) -> Vec<Action> {
        let mut out = Vec::new();
        if event.is_own() {
            return out;
        }
        match event {
            RawInputEvent::Hotkey(_) => {}
            RawInputEvent::Key { key, down, at, .. } => {
                if !self.chord_vks.contains(&key.vk) {
                    self.on_key(key, down, at, &mut out);
                }
            }
            RawInputEvent::Move { pos, at, .. } => {
                if self.opts.record_mouse_moves {
                    self.on_move(pos, at, &mut out);
                }
            }
            RawInputEvent::Button { button, down, pos, at, .. } => {
                self.on_button(button, down, pos, at, &mut out);
                self.last_cursor = Some(pos);
            }
            RawInputEvent::Wheel { delta, horizontal, pos, at, .. } => {
                self.flush_pending(&mut out);
                self.flush_path(&mut out);
                self.emit(&mut out, at, at, Action::MouseWheel { delta, horizontal, pos: Some(pos) });
            }
            RawInputEvent::Foreground { title, process_name, at, .. } => {
                if self.opts.record_window_changes {
                    self.flush_pending(&mut out);
                    self.flush_path(&mut out);
                    self.emit(
                        &mut out,
                        at,
                        at,
                        Action::WindowActivate { title_contains: title, process_name, timeout_ms: 5_000 },
                    );
                }
            }
        }
        out
    }

    /// Closes the recording: emits a held down event and the open mouse path.
    pub fn finish(&mut self) -> Vec<Action> {
        let mut out = Vec::new();
        self.flush_pending(&mut out);
        self.flush_path(&mut out);
        self.last_emit = None;
        self.last_cursor = None;
        out
    }

    fn on_key(&mut self, key: Key, down: bool, at: Instant, out: &mut Vec<Action>) {
        if down {
            self.flush_pending(out);
            self.flush_path(out);
            if self.opts.fold_key_presses {
                self.pending = Some(Pending::Key { key, at });
            } else {
                self.emit(out, at, at, Action::KeyDown { key });
            }
            return;
        }
        if let Some(Pending::Key { key: held, at: held_at }) = &self.pending
            && held.vk == key.vk
            && at.saturating_duration_since(*held_at) < FOLD_WINDOW
        {
            let (held, held_at) = (*held, *held_at);
            self.pending = None;
            self.emit(out, held_at, at, Action::KeyPress { key: held });
            return;
        }
        self.flush_pending(out);
        self.flush_path(out);
        self.emit(out, at, at, Action::KeyUp { key });
    }

    fn on_button(&mut self, button: MouseButton, down: bool, pos: Point, at: Instant, out: &mut Vec<Action>) {
        if down {
            self.flush_pending(out);
            self.flush_path(out);
            if self.opts.fold_clicks {
                self.pending = Some(Pending::Button { button, pos, at, held: Vec::new() });
            } else {
                self.emit(
                    out,
                    at,
                    at,
                    Action::MouseButton { button, event: ButtonEvent::Down, pos: Some(pos) },
                );
            }
            return;
        }
        if let Some(Pending::Button { button: held, pos: held_pos, at: held_at, .. }) = &self.pending
            && *held == button
            && at.saturating_duration_since(*held_at) < FOLD_WINDOW
            && dist_sq(pos, *held_pos) <= CLICK_TOLERANCE_SQ
        {
            let (held, held_pos, held_at) = (*held, *held_pos, *held_at);
            self.pending = None;
            self.emit(
                out,
                held_at,
                at,
                Action::MouseButton { button: held, event: ButtonEvent::Click, pos: Some(held_pos) },
            );
            return;
        }
        self.flush_pending(out);
        self.flush_path(out);
        self.emit(out, at, at, Action::MouseButton { button, event: ButtonEvent::Up, pos: Some(pos) });
    }

    fn on_move(&mut self, pos: Point, at: Instant, out: &mut Vec<Action>) {
        if let Some(Pending::Button { pos: held_pos, at: held_at, held, .. }) = &mut self.pending
            && at.saturating_duration_since(*held_at) < FOLD_WINDOW
            && dist_sq(pos, *held_pos) <= CLICK_TOLERANCE_SQ
        {
            held.push((pos, at));
            return;
        }
        self.flush_pending(out);
        self.sample(pos, at, out);
    }

    fn sample(&mut self, pos: Point, at: Instant, out: &mut Vec<Action>) {
        let origin = self.last_cursor;
        self.last_cursor = Some(pos);
        let step = match self.open_path.as_mut() {
            None => MoveStep::Start,
            Some(path) if at.saturating_duration_since(path.last_at) > PATH_GAP => MoveStep::Restart,
            Some(path) => {
                let dt = at.saturating_duration_since(path.last_at).as_millis() as u32;
                if dt < MIN_SAMPLE_DT_MS && dist_sq(pos, path.last_pos) < MIN_SAMPLE_DIST_SQ {
                    MoveStep::Dropped
                } else {
                    path.points.push(PathPoint { x: pos.x, y: pos.y, dt_ms: dt });
                    path.last_pos = pos;
                    path.last_at = at;
                    MoveStep::Appended { full: path.points.len() >= MAX_PATH_POINTS }
                }
            }
        };
        match step {
            MoveStep::Dropped => {}
            MoveStep::Appended { full } => {
                if full {
                    self.flush_path(out);
                }
            }
            MoveStep::Start => self.open_path = Some(OpenPath::new(pos, at, origin)),
            MoveStep::Restart => {
                self.flush_path(out);
                self.open_path = Some(OpenPath::new(pos, at, origin));
            }
        }
    }

    fn flush_pending(&mut self, out: &mut Vec<Action>) {
        match self.pending.take() {
            None => {}
            Some(Pending::Key { key, at }) => self.emit(out, at, at, Action::KeyDown { key }),
            Some(Pending::Button { button, pos, at, held }) => {
                self.emit(
                    out,
                    at,
                    at,
                    Action::MouseButton { button, event: ButtonEvent::Down, pos: Some(pos) },
                );
                for (pos, at) in held {
                    self.sample(pos, at, out);
                }
            }
        }
    }

    fn flush_path(&mut self, out: &mut Vec<Action>) {
        let Some(path) = self.open_path.take() else {
            return;
        };
        let action = if self.opts.relative_mouse_moves {
            let steps = relative_steps(&path.points, path.origin);
            if steps.is_empty() {
                return;
            }
            Action::MouseMoveRelative { steps, scale: 1.0 }
        } else {
            Action::MouseMove { path: path.points }
        };
        self.emit(out, path.start, path.last_at, action);
    }

    fn emit(&mut self, out: &mut Vec<Action>, start: Instant, end: Instant, action: Action) {
        if let Some(previous) = self.last_emit {
            let gap = start.saturating_duration_since(previous).as_millis() as u64;
            if gap >= self.opts.min_wait_ms as u64 {
                out.push(wait_action(gap));
            }
        }
        out.push(action);
        self.last_emit = Some(end.max(start));
    }
}

/// Turns absolute samples into per-step deltas; the first step is dropped without a known `origin`.
fn relative_steps(points: &[PathPoint], origin: Option<Point>) -> Vec<PathPoint> {
    let mut steps = Vec::with_capacity(points.len());
    let mut previous = origin;
    for point in points {
        if let Some(previous) = previous {
            steps.push(PathPoint { x: point.x - previous.x, y: point.y - previous.y, dt_ms: point.dt_ms });
        }
        previous = Some(point.pos());
    }
    steps
}

fn dist_sq(a: Point, b: Point) -> i64 {
    let dx = (a.x - b.x) as i64;
    let dy = (a.y - b.y) as i64;
    dx * dx + dy * dy
}

/// Picks the unit that keeps the number short: whole minutes, whole seconds, else milliseconds.
fn wait_action(ms: u64) -> Action {
    if ms >= 60_000 && ms.is_multiple_of(60_000) {
        Action::Wait { duration: (ms / 60_000) as f64, unit: TimeUnit::Min }
    } else if ms >= 1_000 && ms.is_multiple_of(1_000) {
        Action::Wait { duration: (ms / 1_000) as f64, unit: TimeUnit::S }
    } else {
        Action::Wait { duration: ms as f64, unit: TimeUnit::Ms }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HotkeyAction;

    const A: u16 = 0x41;
    const B: u16 = 0x42;
    const F9: u16 = 0x78;

    struct Stream {
        rec: Recorder,
        t0: Instant,
        out: Vec<Action>,
    }

    impl Stream {
        fn new(opts: RecordOptions) -> Self {
            Self { rec: Recorder::new(opts, vec![F9]), t0: Instant::now(), out: Vec::new() }
        }

        fn at(&self, ms: u64) -> Instant {
            self.t0 + Duration::from_millis(ms)
        }

        fn key(&mut self, vk: u16, down: bool, ms: u64) -> &mut Self {
            let key = Key { vk, scancode: 0, extended: false };
            let at = self.at(ms);
            self.feed(RawInputEvent::Key { key, down, injected: false, own: false, at })
        }

        fn own_key(&mut self, vk: u16, down: bool, ms: u64) -> &mut Self {
            let key = Key { vk, scancode: 0, extended: false };
            let at = self.at(ms);
            self.feed(RawInputEvent::Key { key, down, injected: true, own: true, at })
        }

        fn mv(&mut self, x: i32, y: i32, ms: u64) -> &mut Self {
            let at = self.at(ms);
            self.feed(RawInputEvent::Move { pos: Point::new(x, y), injected: false, own: false, at })
        }

        fn button(&mut self, down: bool, x: i32, y: i32, ms: u64) -> &mut Self {
            let at = self.at(ms);
            self.feed(RawInputEvent::Button {
                button: MouseButton::Left,
                down,
                pos: Point::new(x, y),
                injected: false,
                own: false,
                at,
            })
        }

        fn feed(&mut self, event: RawInputEvent) -> &mut Self {
            let actions = self.rec.feed(event);
            self.out.extend(actions);
            self
        }

        fn finish(&mut self) -> Vec<Action> {
            let tail = self.rec.finish();
            let mut out = std::mem::take(&mut self.out);
            out.extend(tail);
            out
        }
    }

    fn stream() -> Stream {
        Stream::new(RecordOptions::default())
    }

    fn key_of(vk: u16) -> Key {
        Key { vk, scancode: 0, extended: false }
    }

    fn press(vk: u16) -> Action {
        Action::KeyPress { key: key_of(vk) }
    }

    fn wait(duration: f64, unit: TimeUnit) -> Action {
        Action::Wait { duration, unit }
    }

    fn first_path(actions: &[Action]) -> Vec<PathPoint> {
        actions
            .iter()
            .find_map(|a| match a {
                Action::MouseMove { path } => Some(path.clone()),
                _ => None,
            })
            .expect("a mouse move action")
    }

    #[test]
    fn consecutive_moves_coalesce_into_one_path() {
        let mut s = stream();
        let out = s.mv(0, 0, 0).mv(10, 10, 10).mv(20, 20, 20).key(A, true, 100).key(A, false, 120).finish();
        assert_eq!(
            first_path(&out),
            vec![
                PathPoint { x: 0, y: 0, dt_ms: 0 },
                PathPoint { x: 10, y: 10, dt_ms: 10 },
                PathPoint { x: 20, y: 20, dt_ms: 10 },
            ]
        );
        assert_eq!(out[1], wait(80.0, TimeUnit::Ms));
        assert_eq!(out[2], press(A));
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn tiny_fast_samples_are_dropped() {
        let mut s = stream();
        let out = s.mv(0, 0, 0).mv(1, 0, 3).mv(5, 0, 6).mv(6, 0, 20).finish();
        assert_eq!(
            first_path(&out),
            vec![
                PathPoint { x: 0, y: 0, dt_ms: 0 },
                PathPoint { x: 5, y: 0, dt_ms: 6 },
                PathPoint { x: 6, y: 0, dt_ms: 14 },
            ]
        );
    }

    #[test]
    fn a_long_gap_splits_the_path() {
        let mut s = stream();
        let out = s.mv(0, 0, 0).mv(5, 5, 100).mv(9, 9, 1200).finish();
        assert_eq!(out.len(), 3);
        assert!(matches!(&out[0], Action::MouseMove { path } if path.len() == 2));
        assert_eq!(out[1], wait(1100.0, TimeUnit::Ms));
        assert!(matches!(&out[2], Action::MouseMove { path } if path.len() == 1));
    }

    #[test]
    fn paths_are_capped_at_five_thousand_points() {
        let mut s = stream();
        for i in 0..6_000u64 {
            s.mv(i as i32 * 5, 0, i * 10);
        }
        let out = s.finish();
        let lengths: Vec<usize> = out
            .iter()
            .filter_map(|a| match a {
                Action::MouseMove { path } => Some(path.len()),
                _ => None,
            })
            .collect();
        assert_eq!(lengths, vec![5_000, 1_000]);
    }

    #[test]
    fn wait_units_stay_readable() {
        let mut s = stream();
        let out = s
            .key(A, true, 0)
            .key(A, false, 10)
            .key(A, true, 2_010)
            .key(A, false, 2_020)
            .key(A, true, 62_020)
            .key(A, false, 62_030)
            .key(A, true, 62_080)
            .key(A, false, 62_090)
            .finish();
        assert_eq!(
            out,
            vec![
                press(A),
                wait(2.0, TimeUnit::S),
                press(A),
                wait(1.0, TimeUnit::Min),
                press(A),
                wait(50.0, TimeUnit::Ms),
                press(A),
            ]
        );
    }

    #[test]
    fn gaps_below_the_minimum_are_not_recorded() {
        let mut s = stream();
        let out = s.key(A, true, 0).key(A, false, 5).key(B, true, 15).key(B, false, 20).finish();
        assert_eq!(out, vec![press(A), press(B)]);
    }

    #[test]
    fn quick_stationary_pair_folds_into_a_click() {
        let mut s = stream();
        let out = s.button(true, 100, 100, 0).button(false, 101, 100, 50).finish();
        assert_eq!(
            out,
            vec![Action::MouseButton {
                button: MouseButton::Left,
                event: ButtonEvent::Click,
                pos: Some(Point::new(100, 100)),
            }]
        );
    }

    #[test]
    fn click_jitter_moves_are_swallowed() {
        let mut s = stream();
        let out = s.button(true, 100, 100, 0).mv(101, 100, 10).button(false, 101, 101, 20).finish();
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], Action::MouseButton { event: ButtonEvent::Click, .. }));
    }

    #[test]
    fn a_drag_keeps_down_move_and_up() {
        let mut s = stream();
        let out = s.button(true, 100, 100, 0).mv(150, 150, 10).button(false, 150, 150, 200).finish();
        assert_eq!(
            out,
            vec![
                Action::MouseButton {
                    button: MouseButton::Left,
                    event: ButtonEvent::Down,
                    pos: Some(Point::new(100, 100)),
                },
                Action::MouseMove { path: vec![PathPoint { x: 150, y: 150, dt_ms: 0 }] },
                wait(190.0, TimeUnit::Ms),
                Action::MouseButton {
                    button: MouseButton::Left,
                    event: ButtonEvent::Up,
                    pos: Some(Point::new(150, 150)),
                },
            ]
        );
    }

    #[test]
    fn a_late_up_leaves_the_down_unfolded() {
        let mut s = stream();
        let out = s.key(A, true, 0).key(A, false, 400).finish();
        assert_eq!(
            out,
            vec![
                Action::KeyDown { key: key_of(A) },
                wait(400.0, TimeUnit::Ms),
                Action::KeyUp { key: key_of(A) },
            ]
        );

        let mut s = stream();
        let out = s.button(true, 10, 10, 0).button(false, 10, 10, 400).finish();
        assert_eq!(out.len(), 3);
        assert!(matches!(out[0], Action::MouseButton { event: ButtonEvent::Down, .. }));
        assert!(matches!(out[2], Action::MouseButton { event: ButtonEvent::Up, .. }));
    }

    #[test]
    fn another_event_in_between_breaks_key_folding() {
        let mut s = stream();
        let out = s.key(A, true, 0).key(B, true, 10).key(A, false, 20).key(B, false, 30).finish();
        assert_eq!(
            out,
            vec![
                Action::KeyDown { key: key_of(A) },
                Action::KeyDown { key: key_of(B) },
                Action::KeyUp { key: key_of(A) },
                Action::KeyUp { key: key_of(B) },
            ]
        );
    }

    #[test]
    fn folding_can_be_switched_off() {
        let opts = RecordOptions { fold_clicks: false, fold_key_presses: false, ..Default::default() };
        let mut s = Stream::new(opts);
        let out = s.key(A, true, 0).key(A, false, 10).button(true, 5, 5, 20).button(false, 5, 5, 30).finish();
        assert_eq!(out.len(), 4);
        assert!(matches!(out[0], Action::KeyDown { .. }));
        assert!(matches!(out[1], Action::KeyUp { .. }));
        assert!(matches!(out[2], Action::MouseButton { event: ButtonEvent::Down, .. }));
        assert!(matches!(out[3], Action::MouseButton { event: ButtonEvent::Up, .. }));
    }

    #[test]
    fn own_input_is_dropped() {
        let mut s = stream();
        let own_move = RawInputEvent::Move { pos: Point::new(9, 9), injected: true, own: true, at: s.at(15) };
        let out = s
            .own_key(A, true, 0)
            .own_key(A, false, 10)
            .feed(own_move)
            .key(B, true, 20)
            .key(B, false, 25)
            .finish();
        assert_eq!(out, vec![press(B)]);
    }

    #[test]
    fn hotkey_chord_keys_and_hotkey_events_are_stripped() {
        let mut s = stream();
        let out = s
            .key(F9, true, 0)
            .key(F9, false, 10)
            .feed(RawInputEvent::Hotkey(HotkeyAction::TogglePlay))
            .key(A, true, 20)
            .key(A, false, 25)
            .finish();
        assert_eq!(out, vec![press(A)]);
    }

    #[test]
    fn foreground_change_becomes_window_activate() {
        let mut s = stream();
        let event = RawInputEvent::Foreground {
            hwnd: 7,
            title: "Untitled - Notepad".into(),
            process_name: "notepad.exe".into(),
            at: s.at(0),
        };
        let out = s.feed(event).finish();
        assert_eq!(
            out,
            vec![Action::WindowActivate {
                title_contains: "Untitled - Notepad".into(),
                process_name: "notepad.exe".into(),
                timeout_ms: 5_000,
            }]
        );

        let mut s = Stream::new(RecordOptions { record_window_changes: false, ..Default::default() });
        let event = RawInputEvent::Foreground {
            hwnd: 7,
            title: "x".into(),
            process_name: "x.exe".into(),
            at: s.at(0),
        };
        assert!(s.feed(event).finish().is_empty());
    }

    #[test]
    fn moves_are_skipped_when_not_recorded() {
        let mut s = Stream::new(RecordOptions { record_mouse_moves: false, ..Default::default() });
        let out = s.key(A, true, 0).mv(50, 50, 10).key(A, false, 20).mv(60, 60, 30).finish();
        assert_eq!(out, vec![press(A)]);
    }

    #[test]
    fn wheel_records_the_position() {
        let mut s = stream();
        let event = RawInputEvent::Wheel {
            delta: -120,
            horizontal: false,
            pos: Point::new(4, 5),
            injected: false,
            own: false,
            at: s.at(0),
        };
        let out = s.feed(event).finish();
        assert_eq!(
            out,
            vec![Action::MouseWheel { delta: -120, horizontal: false, pos: Some(Point::new(4, 5)) }]
        );
    }

    #[test]
    fn finish_flushes_the_open_path_and_a_held_down() {
        let mut s = stream();
        s.mv(1, 1, 0).mv(2, 2, 20);
        assert!(s.out.is_empty());
        let out = s.finish();
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], Action::MouseMove { path } if path.len() == 2));

        let mut s = stream();
        s.key(A, true, 0);
        assert_eq!(s.finish(), vec![Action::KeyDown { key: key_of(A) }]);
    }

    fn relative_stream() -> Stream {
        Stream::new(RecordOptions { relative_mouse_moves: true, ..Default::default() })
    }

    fn relative_steps_of(actions: &[Action]) -> Vec<Vec<PathPoint>> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::MouseMoveRelative { steps, .. } => Some(steps.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn relative_mode_records_deltas_between_samples() {
        let mut s = relative_stream();
        let out = s.mv(100, 100, 0).mv(110, 105, 20).mv(115, 105, 40).finish();
        assert_eq!(
            out,
            vec![Action::MouseMoveRelative {
                steps: vec![PathPoint { x: 10, y: 5, dt_ms: 20 }, PathPoint { x: 5, y: 0, dt_ms: 20 },],
                scale: 1.0,
            }]
        );
    }

    #[test]
    fn the_first_relative_step_comes_from_the_last_seen_cursor_position() {
        let mut s = relative_stream();
        let out = s
            .button(true, 100, 100, 0)
            .button(false, 100, 100, 50)
            .mv(110, 100, 200)
            .mv(120, 90, 220)
            .finish();
        assert_eq!(out.len(), 3);
        assert_eq!(out[1], wait(150.0, TimeUnit::Ms));
        assert_eq!(
            relative_steps_of(&out),
            vec![vec![PathPoint { x: 10, y: 0, dt_ms: 0 }, PathPoint { x: 10, y: -10, dt_ms: 20 }]]
        );
    }

    #[test]
    fn a_relative_path_without_a_previous_position_drops_its_first_step() {
        let mut s = relative_stream();
        let out = s.mv(50, 50, 0).key(A, true, 100).key(A, false, 110).finish();
        assert_eq!(out, vec![press(A)]);
    }

    #[test]
    fn relative_paths_keep_gap_splitting_and_downsampling() {
        let mut s = relative_stream();
        let out = s.mv(0, 0, 0).mv(1, 0, 3).mv(5, 0, 6).mv(6, 0, 20).mv(20, 0, 1_300).finish();
        assert_eq!(out.len(), 3);
        assert_eq!(out[1], wait(1_280.0, TimeUnit::Ms));
        assert_eq!(
            relative_steps_of(&out),
            vec![
                vec![PathPoint { x: 5, y: 0, dt_ms: 6 }, PathPoint { x: 1, y: 0, dt_ms: 14 }],
                vec![PathPoint { x: 14, y: 0, dt_ms: 0 }],
            ]
        );
    }

    #[test]
    fn relative_paths_are_capped_like_absolute_ones() {
        let mut s = relative_stream();
        for i in 0..6_000u64 {
            s.mv(i as i32 * 5, 0, i * 10);
        }
        let out = s.finish();
        let lengths: Vec<usize> = relative_steps_of(&out).iter().map(Vec::len).collect();
        assert_eq!(lengths, vec![4_999, 1_000]);
    }

    #[test]
    fn relative_mode_keeps_click_folding_and_drags() {
        let mut s = relative_stream();
        let out = s.button(true, 100, 100, 0).mv(101, 100, 10).button(false, 101, 101, 20).finish();
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], Action::MouseButton { event: ButtonEvent::Click, .. }));

        let mut s = relative_stream();
        let out = s.button(true, 100, 100, 0).mv(150, 150, 10).button(false, 150, 150, 200).finish();
        assert_eq!(
            out,
            vec![
                Action::MouseButton {
                    button: MouseButton::Left,
                    event: ButtonEvent::Down,
                    pos: Some(Point::new(100, 100)),
                },
                Action::MouseMoveRelative { steps: vec![PathPoint { x: 50, y: 50, dt_ms: 0 }], scale: 1.0 },
                wait(190.0, TimeUnit::Ms),
                Action::MouseButton {
                    button: MouseButton::Left,
                    event: ButtonEvent::Up,
                    pos: Some(Point::new(150, 150)),
                },
            ]
        );
    }
}
