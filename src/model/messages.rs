use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::{ActionItem, HotkeyAction, HotkeyConfig, Key, Macro, MouseButton, Point, Rect};

/// Raw input as seen by the low-level hooks, before the recorder turns it into actions.
#[derive(Clone, Debug)]
pub enum RawInputEvent {
    Key { key: Key, down: bool, injected: bool, own: bool, at: Instant },
    Move { pos: Point, injected: bool, own: bool, at: Instant },
    Button { button: MouseButton, down: bool, pos: Point, injected: bool, own: bool, at: Instant },
    Wheel { delta: i32, horizontal: bool, pos: Point, injected: bool, own: bool, at: Instant },
    Foreground { hwnd: isize, title: String, process_name: String, at: Instant },
    Hotkey(HotkeyAction),
}

impl RawInputEvent {
    pub fn at(&self) -> Option<Instant> {
        match self {
            RawInputEvent::Key { at, .. }
            | RawInputEvent::Move { at, .. }
            | RawInputEvent::Button { at, .. }
            | RawInputEvent::Wheel { at, .. }
            | RawInputEvent::Foreground { at, .. } => Some(*at),
            RawInputEvent::Hotkey(_) => None,
        }
    }

    /// True when the event was produced by this process's own injector.
    pub fn is_own(&self) -> bool {
        match self {
            RawInputEvent::Key { own, .. }
            | RawInputEvent::Move { own, .. }
            | RawInputEvent::Button { own, .. }
            | RawInputEvent::Wheel { own, .. } => *own,
            _ => false,
        }
    }
}

/// Options applied while turning raw input into actions.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct RecordOptions {
    pub record_mouse_moves: bool,
    pub record_window_changes: bool,
    /// Fold a quick stationary down/up pair into a single click action.
    pub fold_clicks: bool,
    /// Fold a quick key down/up pair into a single key press action.
    pub fold_key_presses: bool,
    /// Gaps shorter than this are not recorded as separate waits.
    pub min_wait_ms: u32,
}

impl Default for RecordOptions {
    fn default() -> Self {
        Self {
            record_mouse_moves: true,
            record_window_changes: true,
            fold_clicks: true,
            fold_key_presses: true,
            min_wait_ms: 20,
        }
    }
}

/// Commands from the engine or GUI to the Win32 service thread.
#[derive(Clone, Debug)]
pub enum Win32Command {
    EnableHooks(bool),
    SetHotkeys(HotkeyConfig),
    /// Marks the start of playback so the hooks can trigger auto-stop on foreign input.
    PlaybackStarted(Arc<PlayerControl>),
    PlaybackStopped,
    OverlayShow(OverlayScene),
    OverlayHide,
    Shutdown,
}

/// Commands from the GUI to the engine thread.
#[derive(Clone, Debug)]
pub enum EngineCommand {
    StartRecording(RecordOptions),
    StopRecording,
    Play { macro_: Arc<Macro>, start_index: usize },
    StopPlayback,
    SetHotkeys(HotkeyConfig),
    ShowOverlay(OverlayScene),
    HideOverlay,
    Shutdown,
}

/// Events from the engine thread to the GUI, drained once per frame.
#[derive(Clone, Debug, PartialEq)]
pub enum EngineEvent {
    RecordingStarted,
    Recorded(ActionItem),
    RecordingStopped,
    PlaybackStarted { total: usize },
    PlaybackProgress { index: usize, iteration: u32 },
    PlaybackFinished(PlaybackOutcome),
    HotkeyPressed(HotkeyAction),
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaybackOutcome {
    Completed,
    StoppedByUser,
    InterruptedByUserInput,
    Failed { index: usize, error: String },
}

/// Shared flags a running player polls; set from the GUI or directly from the hook thread.
#[derive(Debug, Default)]
pub struct PlayerControl {
    pub stop: std::sync::atomic::AtomicBool,
    pub interrupted: std::sync::atomic::AtomicBool,
    pub wake: (std::sync::Mutex<()>, std::sync::Condvar),
}

impl PlayerControl {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Requests a normal stop and wakes any pending wait.
    pub fn request_stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        self.notify();
    }

    /// Flags that the user interrupted playback with real input and stops it.
    pub fn interrupt(&self) {
        self.interrupted.store(true, std::sync::atomic::Ordering::SeqCst);
        self.request_stop();
    }

    pub fn is_stopped(&self) -> bool {
        self.stop.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn is_interrupted(&self) -> bool {
        self.interrupted.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn notify(&self) {
        let _guard = self.wake.0.lock().unwrap_or_else(|e| e.into_inner());
        self.wake.1.notify_all();
    }
}

/// Drawing instructions for the on-screen overlay, in physical virtual-screen pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverlayScene {
    pub shapes: Vec<OverlayShape>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OverlayShape {
    Polyline { points: Vec<Point>, color: [u8; 4], width: f32 },
    Circle { center: Point, radius: f32, color: [u8; 4], filled: bool },
    Crosshair { center: Point, size: i32, color: [u8; 4] },
    Rect { rect: Rect, color: [u8; 4], width: f32 },
    Label { at: Point, text: String, color: [u8; 4] },
}

impl OverlayScene {
    /// Bounding box of all shapes, or `None` for an empty scene.
    pub fn bounds(&self) -> Option<Rect> {
        let mut min = Point::new(i32::MAX, i32::MAX);
        let mut max = Point::new(i32::MIN, i32::MIN);
        let mut any = false;
        let mut extend = |x: i32, y: i32| {
            any = true;
            min.x = min.x.min(x);
            min.y = min.y.min(y);
            max.x = max.x.max(x);
            max.y = max.y.max(y);
        };
        for shape in &self.shapes {
            match shape {
                OverlayShape::Polyline { points, .. } => points.iter().for_each(|p| extend(p.x, p.y)),
                OverlayShape::Circle { center, radius, .. } => {
                    let r = radius.ceil() as i32;
                    extend(center.x - r, center.y - r);
                    extend(center.x + r, center.y + r);
                }
                OverlayShape::Crosshair { center, size, .. } => {
                    extend(center.x - size, center.y - size);
                    extend(center.x + size, center.y + size);
                }
                OverlayShape::Rect { rect, .. } => {
                    extend(rect.x, rect.y);
                    extend(rect.right(), rect.bottom());
                }
                OverlayShape::Label { at, text, .. } => {
                    extend(at.x, at.y);
                    extend(at.x + text.len() as i32 * 10, at.y + 20);
                }
            }
        }
        any.then(|| Rect::new(min.x, min.y, max.x - min.x, max.y - min.y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_bounds_cover_all_shapes() {
        let scene = OverlayScene {
            shapes: vec![
                OverlayShape::Crosshair { center: Point::new(100, 100), size: 10, color: [0; 4] },
                OverlayShape::Rect { rect: Rect::new(200, 50, 20, 20), color: [0; 4], width: 1.0 },
            ],
        };
        assert_eq!(scene.bounds(), Some(Rect::new(90, 50, 130, 60)));
        assert_eq!(OverlayScene::default().bounds(), None);
    }

    #[test]
    fn player_control_flags() {
        let ctl = PlayerControl::new();
        assert!(!ctl.is_stopped());
        ctl.interrupt();
        assert!(ctl.is_stopped());
        assert!(ctl.is_interrupted());
    }
}
