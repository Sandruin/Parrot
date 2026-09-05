use std::time::Duration;

use anyhow::Result;
use parrot::model::{OverlayScene, OverlayShape, Point, Rect, Win32Command};
use parrot::platform::InputInjector;
use parrot::platform::win32;

const SHOWN: Duration = Duration::from_secs(4);

/// Path and region colour, matching the GUI accent blue.
const PATH: [u8; 4] = [96, 165, 250, 220];
/// Click colour, matching the GUI red.
const CLICK: [u8; 4] = [239, 68, 68, 220];

fn main() -> Result<()> {
    win32::dpi::ensure_per_monitor_v2();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let (raw_tx, _raw_rx) = crossbeam_channel::unbounded();
    let service = win32::spawn_win32_service(raw_tx)?;

    let cursor = win32::injector::Win32Injector.cursor_pos()?;
    println!("drawing the overlay around {}, {} for {SHOWN:?}", cursor.x, cursor.y);
    service.send(Win32Command::OverlayShow(demo_scene(cursor)));

    std::thread::sleep(SHOWN);
    println!("hiding the overlay");
    service.send(Win32Command::OverlayHide);
    std::thread::sleep(Duration::from_millis(300));
    Ok(())
}

/// A path leading into the cursor, a labelled crosshair on it and a region rectangle around it.
fn demo_scene(cursor: Point) -> OverlayScene {
    let start = Point::new(cursor.x - 260, cursor.y - 160);
    OverlayScene {
        shapes: vec![
            OverlayShape::Polyline {
                points: vec![
                    start,
                    Point::new(cursor.x - 200, cursor.y - 40),
                    Point::new(cursor.x - 110, cursor.y - 120),
                    Point::new(cursor.x - 30, cursor.y - 20),
                    cursor,
                ],
                color: PATH,
                width: 2.0,
            },
            OverlayShape::Circle { center: start, radius: 5.0, color: PATH, filled: true },
            OverlayShape::Rect {
                rect: Rect::new(cursor.x - 60, cursor.y - 45, 120, 90),
                color: PATH,
                width: 2.0,
            },
            OverlayShape::Crosshair { center: cursor, size: 16, color: CLICK },
            OverlayShape::Label {
                at: Point::new(cursor.x + 20, cursor.y + 8),
                text: "left click".into(),
                color: CLICK,
            },
        ],
    }
}
