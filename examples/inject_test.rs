use std::time::Duration;

use anyhow::Result;
use macro_recorder::model::Point;
use macro_recorder::platform::native;

const SIDE: i32 = 80;
const STEPS: i32 = 20;

fn main() -> Result<()> {
    native::init();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let type_text = std::env::args().any(|a| a == "--type");

    println!("starting in 3 seconds, focus a harmless window");
    std::thread::sleep(Duration::from_secs(3));

    let injector = native::services()?.injector;
    let start = injector.cursor_pos()?;
    println!("cursor before: {}, {}", start.x, start.y);

    for (leg, (dx, dy)) in [(1, 0), (0, 1), (-1, 0), (0, -1)].into_iter().enumerate() {
        for step in 1..=STEPS {
            let travelled = SIDE * step / STEPS;
            let corner = corner_offset(dx, dy);
            let target = Point::new(start.x + corner.x + dx * travelled, start.y + corner.y + dy * travelled);
            injector.mouse_move_abs(target)?;
            std::thread::sleep(Duration::from_millis(8));
        }
        let at = injector.cursor_pos()?;
        println!("corner {}: {}, {}", leg + 1, at.x, at.y);
    }
    injector.mouse_move_abs(start)?;
    let end = injector.cursor_pos()?;
    println!("cursor after: {}, {}", end.x, end.y);

    if type_text {
        println!("typing \"hello\"");
        for ch in "hello".chars() {
            let Some(chord) = injector.key_for_char(ch) else {
                println!("  {ch:?} is not on the current layout");
                continue;
            };
            let shift = native::keys::key_from_vk(macro_recorder::model::vk::LSHIFT);
            if chord.shift {
                injector.key(shift, true)?;
            }
            injector.key(chord.key, true)?;
            injector.key(chord.key, false)?;
            if chord.shift {
                injector.key(shift, false)?;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    } else {
        println!("run with --type to also type \"hello\" into the focused window");
    }
    Ok(())
}

/// Where the given leg of the square starts, relative to the initial cursor position.
fn corner_offset(dx: i32, dy: i32) -> Point {
    match (dx, dy) {
        (1, 0) => Point::new(0, 0),
        (0, 1) => Point::new(SIDE, 0),
        (-1, 0) => Point::new(SIDE, SIDE),
        _ => Point::new(0, SIDE),
    }
}
