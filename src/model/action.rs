use serde::{Deserialize, Serialize};

pub type ActionId = u64;

/// Screen position in physical pixels, virtual-screen space.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// Axis-aligned rectangle in physical pixels, virtual-screen space.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.right() && p.y < self.bottom()
    }

    pub fn area(&self) -> i64 {
        self.w as i64 * self.h as i64
    }
}

/// One sample of a recorded mouse path; `dt_ms` is the delay since the previous sample.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PathPoint {
    pub x: i32,
    pub y: i32,
    pub dt_ms: u32,
}

impl PathPoint {
    pub fn pos(&self) -> Point {
        Point::new(self.x, self.y)
    }
}

/// A keyboard key identified by Windows virtual-key code and hardware scan code.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Key {
    pub vk: u16,
    pub scancode: u16,
    pub extended: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TimeUnit {
    #[default]
    Ms,
    S,
    Min,
}

impl TimeUnit {
    pub const ALL: [TimeUnit; 3] = [TimeUnit::Ms, TimeUnit::S, TimeUnit::Min];

    pub fn label(&self) -> &'static str {
        match self {
            TimeUnit::Ms => "ms",
            TimeUnit::S => "s",
            TimeUnit::Min => "min",
        }
    }

    pub fn to_millis(self, duration: f64) -> f64 {
        match self {
            TimeUnit::Ms => duration,
            TimeUnit::S => duration * 1_000.0,
            TimeUnit::Min => duration * 60_000.0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
    X1,
    X2,
}

impl MouseButton {
    pub const ALL: [MouseButton; 5] =
        [MouseButton::Left, MouseButton::Right, MouseButton::Middle, MouseButton::X1, MouseButton::X2];

    pub fn label(&self) -> &'static str {
        match self {
            MouseButton::Left => "left",
            MouseButton::Right => "right",
            MouseButton::Middle => "middle",
            MouseButton::X1 => "X1",
            MouseButton::X2 => "X2",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ButtonEvent {
    Down,
    Up,
    #[default]
    Click,
}

impl ButtonEvent {
    pub const ALL: [ButtonEvent; 3] = [ButtonEvent::Down, ButtonEvent::Up, ButtonEvent::Click];

    pub fn label(&self) -> &'static str {
        match self {
            ButtonEvent::Down => "down",
            ButtonEvent::Up => "up",
            ButtonEvent::Click => "click",
        }
    }
}

/// How `TypeText` is injected: Unicode events for normal apps, scan codes for games.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextMode {
    #[default]
    Unicode,
    ScanCodes,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ImageMatchMode {
    /// Region and template have the same size and are compared pixel by pixel.
    #[default]
    Exact,
    /// Template is searched anywhere inside the region.
    Search,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    Wait {
        duration: f64,
        unit: TimeUnit,
    },
    KeyDown {
        key: Key,
    },
    KeyUp {
        key: Key,
    },
    KeyPress {
        key: Key,
    },
    TypeText {
        text: String,
        mode: TextMode,
        char_delay_ms: u32,
    },
    /// Cursor path with per-sample delays; a single point is a plain jump.
    MouseMove {
        path: Vec<PathPoint>,
    },
    /// Button event at `pos`, or at the current cursor position when `pos` is `None`.
    MouseButton {
        button: MouseButton,
        event: ButtonEvent,
        pos: Option<Point>,
    },
    /// Wheel movement in multiples of 120 (one notch), positive is up or right.
    MouseWheel {
        delta: i32,
        horizontal: bool,
        pos: Option<Point>,
    },
    WindowActivate {
        title_contains: String,
        process_name: String,
        timeout_ms: u32,
    },
    WaitForImage {
        region: Rect,
        #[serde(with = "crate::model::b64")]
        template_png: Vec<u8>,
        similarity: f32,
        poll_ms: u32,
        timeout_ms: u32,
        mode: ImageMatchMode,
    },
    WaitForText {
        region: Rect,
        text: String,
        case_sensitive: bool,
        poll_ms: u32,
        timeout_ms: u32,
    },
    WaitForFile {
        path: String,
        timeout_ms: u32,
    },
    Comment {
        text: String,
    },
    Label {
        name: String,
    },
}

impl Action {
    /// Short human readable name for the Action column.
    pub fn kind_name(&self) -> String {
        match self {
            Action::Wait { .. } => "Wait".into(),
            Action::KeyDown { .. } => "Key down".into(),
            Action::KeyUp { .. } => "Key up".into(),
            Action::KeyPress { .. } => "Key press".into(),
            Action::TypeText { .. } => "Type text".into(),
            Action::MouseMove { .. } => "Mouse move".into(),
            Action::MouseButton { button, event, .. } => {
                format!("Mouse {} {}", button.label(), event.label())
            }
            Action::MouseWheel { horizontal, .. } => {
                if *horizontal {
                    "Mouse wheel horizontal".into()
                } else {
                    "Mouse wheel".into()
                }
            }
            Action::WindowActivate { .. } => "Window activate".into(),
            Action::WaitForImage { .. } => "Wait for image".into(),
            Action::WaitForText { .. } => "Wait for text".into(),
            Action::WaitForFile { .. } => "Wait for file".into(),
            Action::Comment { .. } => "Comment".into(),
            Action::Label { .. } => "Label".into(),
        }
    }

    /// Compact summary for the Value column.
    pub fn value_text(&self) -> String {
        match self {
            Action::Wait { duration, unit } => format!("{} {}", trim_float(*duration), unit.label()),
            Action::KeyDown { key } | Action::KeyUp { key } | Action::KeyPress { key } => key.name(),
            Action::TypeText { text, .. } => truncate(text, 40),
            Action::MouseMove { path } => match (path.first(), path.last()) {
                (Some(a), Some(b)) if path.len() > 1 => {
                    format!("{}, {} -> {}, {} ({} points)", a.x, a.y, b.x, b.y, path.len())
                }
                (Some(a), _) => format!("{}, {}", a.x, a.y),
                _ => String::new(),
            },
            Action::MouseButton { pos, .. } => match pos {
                Some(p) => format!("{}, {}", p.x, p.y),
                None => "at cursor".into(),
            },
            Action::MouseWheel { delta, .. } => format!("{delta}"),
            Action::WindowActivate { title_contains, process_name, .. } => {
                if title_contains.is_empty() {
                    process_name.clone()
                } else {
                    title_contains.clone()
                }
            }
            Action::WaitForImage { region, similarity, .. } => format!(
                "{}, {} {}x{} >= {}%",
                region.x,
                region.y,
                region.w,
                region.h,
                (similarity * 100.0).round()
            ),
            Action::WaitForText { text, .. } => truncate(text, 40),
            Action::WaitForFile { path, .. } => path.clone(),
            Action::Comment { text } => truncate(text, 60),
            Action::Label { name } => name.clone(),
        }
    }

    /// Duration a `Wait` action sleeps, `None` for every other action.
    pub fn wait_millis(&self) -> Option<f64> {
        match self {
            Action::Wait { duration, unit } => Some(unit.to_millis(*duration)),
            _ => None,
        }
    }

    /// Whether the action targets a screen position that the overlay can visualize.
    pub fn is_positional(&self) -> bool {
        matches!(
            self,
            Action::MouseMove { .. }
                | Action::MouseButton { pos: Some(_), .. }
                | Action::MouseWheel { pos: Some(_), .. }
                | Action::WaitForImage { .. }
                | Action::WaitForText { .. }
        )
    }
}

/// One row of the macro: an action plus user metadata.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ActionItem {
    pub id: ActionId,
    pub action: Action,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl ActionItem {
    pub fn new(id: ActionId, action: Action) -> Self {
        Self { id, action, comment: String::new(), enabled: true }
    }
}

impl std::hash::Hash for ActionItem {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

fn default_true() -> bool {
    true
}

fn trim_float(v: f64) -> String {
    if v.fract() == 0.0 { format!("{}", v as i64) } else { format!("{v:.2}") }
}

fn truncate(s: &str, max_chars: usize) -> String {
    let single_line = s.replace(['\r', '\n'], " ");
    if single_line.chars().count() <= max_chars {
        single_line
    } else {
        let cut: String = single_line.chars().take(max_chars - 1).collect();
        format!("{cut}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_millis_uses_unit() {
        let a = Action::Wait { duration: 1.5, unit: TimeUnit::S };
        assert_eq!(a.wait_millis(), Some(1500.0));
        assert_eq!(Action::Label { name: "x".into() }.wait_millis(), None);
    }

    #[test]
    fn value_text_summaries() {
        let mv = Action::MouseMove {
            path: vec![PathPoint { x: 1, y: 2, dt_ms: 0 }, PathPoint { x: 3, y: 4, dt_ms: 5 }],
        };
        assert_eq!(mv.value_text(), "1, 2 -> 3, 4 (2 points)");
        let click = Action::MouseButton { button: MouseButton::Left, event: ButtonEvent::Click, pos: None };
        assert_eq!(click.kind_name(), "Mouse left click");
        assert_eq!(click.value_text(), "at cursor");
        assert_eq!(Action::Wait { duration: 100.0, unit: TimeUnit::Ms }.value_text(), "100 ms");
    }

    #[test]
    fn rect_contains_is_half_open() {
        let r = Rect::new(10, 10, 5, 5);
        assert!(r.contains(Point::new(10, 10)));
        assert!(r.contains(Point::new(14, 14)));
        assert!(!r.contains(Point::new(15, 15)));
    }
}
