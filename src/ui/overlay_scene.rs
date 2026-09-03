use crate::model::{Action, OverlayScene, OverlayShape, PathPoint, Point};

/// Path and region colour, an accent blue.
const PATH: [u8; 4] = [96, 165, 250, 220];
/// Button event colour, red.
const CLICK: [u8; 4] = [239, 68, 68, 220];
/// Wait-for-image and wait-for-text region colour, amber.
const REGION: [u8; 4] = [250, 204, 21, 220];

/// Longest text drawn next to an OCR region before it is cut off.
const LABEL_CHARS: usize = 24;

/// Shapes that visualize where an action acts on screen; empty for actions without a position.
pub fn for_action(action: &Action) -> OverlayScene {
    for_action_from(action, cursor_pos())
}

/// Same as `for_action`, with the cursor position that relative moves start from passed in.
pub fn for_action_from(action: &Action, cursor: Point) -> OverlayScene {
    let mut shapes = Vec::new();
    match action {
        Action::MouseMove { path } => {
            let Some(first) = path.first() else {
                return OverlayScene::default();
            };
            let last = path.last().copied().unwrap_or(*first);
            if path.len() > 1 {
                shapes.push(OverlayShape::Polyline {
                    points: path.iter().map(|p| p.pos()).collect(),
                    color: PATH,
                    width: 2.0,
                });
            }
            shapes.push(OverlayShape::Circle { center: first.pos(), radius: 5.0, color: PATH, filled: true });
            shapes.push(OverlayShape::Crosshair { center: last.pos(), size: 14, color: PATH });
        }
        Action::MouseMoveRelative { steps, scale } => {
            let points = relative_path(cursor, steps, *scale);
            let last = points.last().copied().unwrap_or(cursor);
            if points.len() > 1 {
                shapes.push(OverlayShape::Polyline { points, color: PATH, width: 2.0 });
            }
            shapes.push(OverlayShape::Circle { center: cursor, radius: 5.0, color: PATH, filled: true });
            shapes.push(OverlayShape::Crosshair { center: last, size: 14, color: PATH });
        }
        Action::MouseButton { button, event, pos: Some(pos) } => {
            shapes.push(OverlayShape::Crosshair { center: *pos, size: 16, color: CLICK });
            shapes.push(OverlayShape::Label {
                at: Point::new(pos.x + 20, pos.y + 8),
                text: format!("{} {}", button.label(), event.label()),
                color: CLICK,
            });
        }
        Action::MouseWheel { pos: Some(pos), .. } => {
            shapes.push(OverlayShape::Crosshair { center: *pos, size: 14, color: PATH });
        }
        Action::WaitForImage { region, .. } => {
            shapes.push(OverlayShape::Rect { rect: *region, color: REGION, width: 2.0 });
        }
        Action::WaitForText { region, text, .. } | Action::ClickOnText { region, text, .. } => {
            shapes.push(OverlayShape::Rect { rect: *region, color: REGION, width: 2.0 });
            shapes.push(OverlayShape::Label {
                at: Point::new(region.x, region.bottom() + 6),
                text: clip(text, LABEL_CHARS),
                color: REGION,
            });
        }
        _ => {}
    }
    OverlayScene { shapes }
}

/// Screen positions the cursor passes through when the scaled steps are applied from `start`.
fn relative_path(start: Point, steps: &[PathPoint], scale: f32) -> Vec<Point> {
    let mut points = Vec::with_capacity(steps.len() + 1);
    points.push(start);
    let (mut x, mut y) = (start.x as f32, start.y as f32);
    for step in steps {
        x += step.x as f32 * scale;
        y += step.y as f32 * scale;
        points.push(Point::new(x.round() as i32, y.round() as i32));
    }
    points
}

/// Single-line text of at most `max_chars` characters, with an ellipsis when it was cut.
fn clip(text: &str, max_chars: usize) -> String {
    let single_line = text.replace(['\r', '\n'], " ");
    if single_line.chars().count() <= max_chars {
        return single_line;
    }
    let kept: String = single_line.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{kept}...")
}

/// Where a relative move starts, which is wherever the cursor happens to be right now.
#[cfg(windows)]
fn cursor_pos() -> Point {
    use crate::platform::InputInjector as _;

    crate::platform::win32::injector::Win32Injector.cursor_pos().unwrap_or_default()
}

#[cfg(not(windows))]
fn cursor_pos() -> Point {
    Point::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ButtonEvent, ImageMatchMode, Key, MouseButton, Rect, TimeUnit};

    fn path(points: &[(i32, i32)]) -> Action {
        Action::MouseMove { path: points.iter().map(|&(x, y)| PathPoint { x, y, dt_ms: 8 }).collect() }
    }

    #[test]
    fn mouse_move_draws_polyline_start_dot_and_end_crosshair() {
        let scene = for_action(&path(&[(10, 10), (20, 30), (40, 50)]));
        assert_eq!(scene.shapes.len(), 3);
        assert!(matches!(&scene.shapes[0], OverlayShape::Polyline { points, .. } if points.len() == 3));
        assert!(
            matches!(scene.shapes[1], OverlayShape::Circle { center, .. } if center == Point::new(10, 10))
        );
        assert!(
            matches!(scene.shapes[2], OverlayShape::Crosshair { center, .. } if center == Point::new(40, 50))
        );
        assert_eq!(scene.bounds(), Some(Rect::new(5, 5, 49, 59)));
    }

    #[test]
    fn single_point_move_has_no_polyline() {
        let scene = for_action(&path(&[(7, 8)]));
        assert_eq!(scene.shapes.len(), 2);
        assert!(!scene.shapes.iter().any(|s| matches!(s, OverlayShape::Polyline { .. })));
        assert!(for_action(&path(&[])).shapes.is_empty());
    }

    #[test]
    fn button_at_position_gets_crosshair_and_label() {
        let action = Action::MouseButton {
            button: MouseButton::Right,
            event: ButtonEvent::Click,
            pos: Some(Point::new(100, 200)),
        };
        let scene = for_action(&action);
        assert!(
            matches!(scene.shapes[0], OverlayShape::Crosshair { center, color, .. } if center == Point::new(100, 200) && color == CLICK)
        );
        assert!(matches!(&scene.shapes[1], OverlayShape::Label { text, .. } if text == "right click"));
    }

    #[test]
    fn wheel_and_region_actions() {
        let wheel = Action::MouseWheel { delta: 120, horizontal: false, pos: Some(Point::new(5, 6)) };
        assert!(matches!(for_action(&wheel).shapes[0], OverlayShape::Crosshair { .. }));
        let image = Action::WaitForImage {
            region: Rect::new(1, 2, 30, 40),
            template_png: Vec::new(),
            similarity: 0.9,
            poll_ms: 250,
            timeout_ms: 5000,
            mode: ImageMatchMode::Exact,
        };
        assert!(
            matches!(for_action(&image).shapes[0], OverlayShape::Rect { rect, .. } if rect == Rect::new(1, 2, 30, 40))
        );
    }

    #[test]
    fn relative_steps_accumulate_from_the_start_and_scale() {
        let steps = [PathPoint { x: 10, y: -5, dt_ms: 4 }, PathPoint { x: 10, y: -5, dt_ms: 4 }];
        let points = relative_path(Point::new(100, 100), &steps, 2.0);
        assert_eq!(points, vec![Point::new(100, 100), Point::new(120, 90), Point::new(140, 80)]);
        assert_eq!(relative_path(Point::new(3, 4), &[], 1.0), vec![Point::new(3, 4)]);
    }

    #[test]
    fn relative_move_draws_path_from_the_cursor() {
        let action = Action::MouseMoveRelative {
            steps: vec![PathPoint { x: 40, y: 0, dt_ms: 0 }, PathPoint { x: 0, y: 40, dt_ms: 8 }],
            scale: 0.5,
        };
        let scene = for_action_from(&action, Point::new(500, 400));
        assert_eq!(scene.shapes.len(), 3);
        let expected = vec![Point::new(500, 400), Point::new(520, 400), Point::new(520, 420)];
        assert!(matches!(&scene.shapes[0], OverlayShape::Polyline { points, .. } if *points == expected));
        assert!(
            matches!(scene.shapes[1], OverlayShape::Circle { center, .. } if center == Point::new(500, 400))
        );
        assert!(
            matches!(scene.shapes[2], OverlayShape::Crosshair { center, .. } if center == Point::new(520, 420))
        );
    }

    #[test]
    fn a_single_relative_step_has_no_polyline() {
        let action = Action::MouseMoveRelative { steps: Vec::new(), scale: 1.0 };
        let scene = for_action_from(&action, Point::new(1, 2));
        assert_eq!(scene.shapes.len(), 2);
        assert!(!scene.shapes.iter().any(|s| matches!(s, OverlayShape::Polyline { .. })));
    }

    #[test]
    fn text_regions_get_a_rect_and_a_clipped_label() {
        let action = Action::ClickOnText {
            region: Rect::new(10, 20, 100, 30),
            text: "a very long caption that will not fit".into(),
            case_sensitive: false,
            button: MouseButton::Left,
            poll_ms: 500,
            timeout_ms: 10_000,
        };
        let scene = for_action_from(&action, Point::default());
        assert!(
            matches!(scene.shapes[0], OverlayShape::Rect { rect, .. } if rect == Rect::new(10, 20, 100, 30))
        );
        let OverlayShape::Label { at, text, .. } = &scene.shapes[1] else {
            panic!("expected a label, got {:?}", scene.shapes[1]);
        };
        assert_eq!(*at, Point::new(10, 56));
        assert_eq!(text, "a very long caption tha...");
        assert_eq!(text.chars().count(), LABEL_CHARS + 2);
    }

    #[test]
    fn short_text_is_kept_and_newlines_are_flattened() {
        assert_eq!(clip("Ready", LABEL_CHARS), "Ready");
        assert_eq!(clip("two\nlines", LABEL_CHARS), "two lines");
    }

    #[test]
    fn actions_without_a_position_produce_an_empty_scene() {
        let cases = [
            Action::Wait { duration: 1.0, unit: TimeUnit::S },
            Action::KeyPress { key: Key::from_vk(0x41) },
            Action::Comment { text: "x".into() },
            Action::MouseButton { button: MouseButton::Left, event: ButtonEvent::Click, pos: None },
            Action::MouseWheel { delta: 120, horizontal: true, pos: None },
        ];
        for action in cases {
            assert!(for_action(&action).shapes.is_empty(), "{action:?}");
        }
    }
}
