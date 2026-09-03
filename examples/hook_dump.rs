use std::time::{Duration, Instant};

use anyhow::Result;
use macro_recorder::model::{HotkeyConfig, RawInputEvent, Win32Command};
use macro_recorder::platform::win32;

/// Seconds to dump for, overridable with the first argument.
fn duration() -> Duration {
    let secs = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(10);
    Duration::from_secs(secs)
}

fn main() -> Result<()> {
    win32::dpi::ensure_per_monitor_v2();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let (raw_tx, raw_rx) = crossbeam_channel::unbounded();
    let service = win32::spawn_win32_service(raw_tx)?;
    service.send(Win32Command::SetHotkeys(HotkeyConfig::default()));
    service.send(Win32Command::EnableHooks(true));

    let duration = duration();
    println!("dumping raw input for {duration:?}, move the mouse and press a few keys");
    let start = Instant::now();
    let mut count = 0usize;
    while let Some(remaining) = duration.checked_sub(start.elapsed()) {
        match raw_rx.recv_timeout(remaining) {
            Ok(event) => {
                count += 1;
                println!("{:>7.3}s {}", start.elapsed().as_secs_f32(), describe(&event));
            }
            Err(_) => break,
        }
    }
    println!("{count} events in {:?}", start.elapsed());
    Ok(())
}

fn describe(event: &RawInputEvent) -> String {
    match event {
        RawInputEvent::Key { key, down, injected, own, .. } => format!(
            "Key    vk 0x{:02X} scan 0x{:02X}{} {} {}",
            key.vk,
            key.scancode,
            if key.extended { " ext" } else { "" },
            if *down { "down" } else { "up" },
            tags(*injected, *own)
        ),
        RawInputEvent::Move { pos, injected, own, .. } => {
            format!("Move   {}, {} {}", pos.x, pos.y, tags(*injected, *own))
        }
        RawInputEvent::Button { button, down, pos, injected, own, .. } => format!(
            "Button {} {} at {}, {} {}",
            button.label(),
            if *down { "down" } else { "up" },
            pos.x,
            pos.y,
            tags(*injected, *own)
        ),
        RawInputEvent::Wheel { delta, horizontal, pos, injected, own, .. } => format!(
            "Wheel  {delta}{} at {}, {} {}",
            if *horizontal { " horizontal" } else { "" },
            pos.x,
            pos.y,
            tags(*injected, *own)
        ),
        RawInputEvent::Foreground { hwnd, title, process_name, .. } => {
            format!("Front  {hwnd:#x} {process_name} \"{title}\"")
        }
        RawInputEvent::Hotkey(action) => format!("Hotkey {action:?}"),
    }
}

fn tags(injected: bool, own: bool) -> &'static str {
    match (injected, own) {
        (true, true) => "[own]",
        (true, false) => "[injected]",
        _ => "",
    }
}
