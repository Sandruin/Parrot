use crate::model::{Action, OverlayScene, OverlayShape, Point};

/// Path and region colour, an accent blue.
const PATH: [u8; 4] = [96, 165, 250, 220];
/// Button event colour, red.
const CLICK: [u8; 4] = [239, 68, 68, 220];
/// Wait-for-image and wait-for-text region colour, amber.
const REGION: [u8; 4] = [250, 204, 21, 220];

/// Shapes that visualize where an action acts on screen; empty for actions without a position.
pub fn for_action(action: &Action) -> OverlayScene {
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
        Action::WaitForImage { region, .. } | Action::WaitForText { region, .. } => {
            shapes.push(OverlayShape::Rect { rect: *region, color: REGION, width: 2.0 });
        }
        _ => {}
    }
    OverlayScene { shapes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ButtonEvent, ImageMatchMode, Key, MouseButton, PathPoint, Rect, TimeUnit};

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
