use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};

use super::hyprland::{self, Client, Hyprland};
use crate::platform::{WindowInfo, WindowManager, WindowRef};

const POLL_INTERVAL: Duration = Duration::from_millis(10);
/// How long a plain `focuswindow` gets before `activate` also tries `alterzorder top`.
const ESCALATE_AFTER: Duration = Duration::from_millis(150);

/// Finds and focuses windows through the Hyprland IPC socket.
pub struct HyprlandWindows {
    hyprland: Option<Hyprland>,
}

impl HyprlandWindows {
    pub fn new() -> Self {
        Self { hyprland: Hyprland::detect() }
    }
}

impl Default for HyprlandWindows {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowManager for HyprlandWindows {
    fn find(&self, title_contains: &str, process_name: &str) -> Option<WindowRef> {
        let hyprland = self.hyprland.as_ref()?;
        let clients = match hyprland.clients() {
            Ok(clients) => clients,
            Err(e) => {
                log::warn!("listing hyprland clients: {e:#}");
                return None;
            }
        };
        let own_pid = std::process::id() as i32;
        best_match(&clients, title_contains, process_name, own_pid, exe_name)
            .and_then(|client| hyprland::parse_address(&client.address))
            .map(WindowRef)
    }

    fn activate(&self, window: WindowRef, timeout: Duration) -> Result<()> {
        let Some(hyprland) = self.hyprland.as_ref() else {
            bail!("window management needs Hyprland");
        };
        let address = hyprland::format_address(window.0);
        let deadline = Instant::now() + timeout;

        hyprland
            .dispatch(
                &format!("focuswindow address:{address}"),
                &format!("hl.dsp.focus({{ window = \"address:{address}\" }})"),
            )
            .context("dispatching focuswindow")?;
        let first_step = (Instant::now() + ESCALATE_AFTER).min(deadline);
        if wait_for_focus(hyprland, window.0, first_step) {
            return Ok(());
        }
        if Instant::now() < deadline {
            let result = hyprland.dispatch(
                &format!("alterzorder top,address:{address}"),
                &format!("hl.dsp.window.alter_zorder({{ mode = \"top\", window = \"address:{address}\" }})"),
            );
            if let Err(e) = result {
                log::debug!("raising {address} to the top of the z-order: {e:#}");
            }
            if wait_for_focus(hyprland, window.0, deadline) {
                return Ok(());
            }
        }
        let title = hyprland
            .clients()
            .unwrap_or_default()
            .into_iter()
            .find(|client| client.address == address)
            .map(|client| client.title)
            .unwrap_or_default();
        bail!("could not activate window {address} ('{title}') within {timeout:?}");
    }

    fn foreground(&self) -> Option<WindowInfo> {
        let hyprland = self.hyprland.as_ref()?;
        let client = match hyprland.active_window() {
            Ok(Some(client)) => client,
            Ok(None) => return None,
            Err(e) => {
                log::debug!("querying the active window: {e:#}");
                return None;
            }
        };
        client_info(&client)
    }
}

/// Polls the active window every [`POLL_INTERVAL`] until it is `handle` or `deadline` passes.
fn wait_for_focus(hyprland: &Hyprland, handle: isize, deadline: Instant) -> bool {
    loop {
        match hyprland.active_window() {
            Ok(Some(client)) if hyprland::parse_address(&client.address) == Some(handle) => return true,
            Ok(_) => {}
            Err(e) => log::debug!("polling the active window: {e:#}"),
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Converts a client into the platform-independent `WindowInfo`, `None` when its address is unparsable.
fn client_info(client: &Client) -> Option<WindowInfo> {
    let handle = hyprland::parse_address(&client.address)?;
    Some(WindowInfo {
        handle: WindowRef(handle),
        title: client.title.clone(),
        process_name: client.process_name(),
    })
}

/// Executable name of a process, or empty when it cannot be resolved.
fn exe_name(pid: i32) -> String {
    u32::try_from(pid).ok().and_then(hyprland::process_name).unwrap_or_default()
}

/// Most recently focused mapped, titled window from another process that matches both filters.
fn best_match<'a>(
    clients: &'a [Client],
    title_contains: &str,
    process_filter: &str,
    own_pid: i32,
    exe_name_of: impl Fn(i32) -> String,
) -> Option<&'a Client> {
    clients
        .iter()
        .filter(|client| client.mapped && !client.hidden && !client.title.is_empty())
        .filter(|client| client.pid != own_pid)
        .filter(|client| {
            matches(
                &client.title,
                &client.process_name(),
                &exe_name_of(client.pid),
                title_contains,
                process_filter,
            )
        })
        .min_by_key(|client| client.focus_history_id)
}

/// Whether a window passes the user's filters; empty filters match anything and case is ignored.
/// The process filter matches either the window class or the executable name of its pid.
fn matches(title: &str, class: &str, exe_name: &str, title_contains: &str, process_filter: &str) -> bool {
    let title_ok = title_contains.is_empty() || title.to_lowercase().contains(&title_contains.to_lowercase());
    let filter = process_filter.to_lowercase();
    let process_ok =
        process_filter.is_empty() || class.to_lowercase() == filter || exe_name.to_lowercase() == filter;
    title_ok && process_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(title: &str, class: &str, pid: i32, focus_history_id: i32) -> Client {
        Client {
            address: format!("0x{:x}", pid.unsigned_abs() + 1),
            mapped: true,
            hidden: false,
            title: title.to_owned(),
            class: class.to_owned(),
            pid,
            focus_history_id,
            ..Default::default()
        }
    }

    #[test]
    fn empty_filters_match_anything() {
        assert!(matches("Mozilla Firefox", "firefox", "firefox", "", ""));
        assert!(matches("", "", "", "", ""));
    }

    #[test]
    fn title_filter_is_a_case_insensitive_substring() {
        assert!(matches("Mozilla Firefox", "firefox", "firefox", "FIREFOX", ""));
        assert!(matches("Mozilla Firefox", "firefox", "firefox", "mozilla", ""));
        assert!(!matches("Mozilla Firefox", "firefox", "firefox", "calculator", ""));
    }

    #[test]
    fn process_filter_matches_class_or_executable_name_case_insensitively() {
        assert!(matches("main | ~", "Alacritty", "alacritty", "", "ALACRITTY"));
        assert!(matches("Mozilla Firefox", "org.mozilla.firefox", "firefox", "", "firefox"));
        assert!(!matches("Mozilla Firefox", "org.mozilla.firefox", "firefox", "", "chrome"));
    }

    #[test]
    fn both_filters_must_match() {
        assert!(matches("main | ~", "Alacritty", "alacritty", "main", "alacritty"));
        assert!(!matches("main | ~", "Alacritty", "alacritty", "main", "firefox"));
        assert!(!matches("main | ~", "Alacritty", "alacritty", "other", "alacritty"));
    }

    #[test]
    fn best_match_ignores_unmapped_hidden_untitled_and_own_process_windows() {
        let clients = vec![
            Client { mapped: false, ..client("Hidden by mapped", "term", 1, 0) },
            Client { hidden: true, ..client("Hidden window", "term", 2, 0) },
            client("", "term", 3, 0),
            client("Ours", "term", 4, 0),
        ];
        assert!(best_match(&clients, "", "", 4, |_| String::new()).is_none());
    }

    #[test]
    fn best_match_prefers_the_most_recently_focused_window() {
        let clients = vec![client("term A", "Alacritty", 1, 5), client("term B", "Alacritty", 2, 1)];
        let found = best_match(&clients, "", "alacritty", 99, |_| String::new()).unwrap();
        assert_eq!(found.pid, 2);
    }

    #[test]
    fn best_match_falls_back_to_the_executable_name() {
        let clients = vec![client("Browser", "org.mozilla.firefox", 1, 0)];
        assert!(best_match(&clients, "", "firefox", 99, |_| "firefox".to_owned()).is_some());
        assert!(best_match(&clients, "", "org.mozilla.firefox", 99, |_| String::new()).is_some());
        assert!(best_match(&clients, "", "chrome", 99, |_| "firefox".to_owned()).is_none());
    }

    /// Exercises `find`, `activate` and `foreground` against a real compositor and an open Alacritty
    /// window, then restores whatever was focused beforehand; run with `cargo test -- --ignored`.
    #[test]
    #[ignore = "talks to a live Hyprland compositor"]
    fn live_hyprland_finds_and_activates_alacritty() {
        let windows = HyprlandWindows::new();
        let hyprland = windows.hyprland.as_ref().expect("no running Hyprland instance detected");
        let previous = windows.foreground().expect("a window should be focused to run this test");

        let by_class = windows.find("", "Alacritty").expect("Alacritty not found by class");
        let by_title = windows.find("main", "").expect("Alacritty not found by title");
        assert_eq!(by_class, by_title, "class and title search should find the same window");

        let other = hyprland
            .clients()
            .expect("listing clients")
            .into_iter()
            .find(|c| hyprland::parse_address(&c.address) != Some(by_class.0) && !c.title.is_empty())
            .expect("need a second window open to focus away from Alacritty");
        let other_handle = hyprland::parse_address(&other.address).expect("parsing the other address");
        hyprland
            .dispatch(
                &format!("focuswindow address:{}", other.address),
                &format!("hl.dsp.focus({{ window = \"address:{}\" }})", other.address),
            )
            .expect("focusing the other window");
        assert!(
            wait_for_focus(hyprland, other_handle, Instant::now() + Duration::from_secs(1)),
            "the other window never gained focus"
        );

        windows.activate(by_class, Duration::from_secs(2)).expect("activating Alacritty failed");
        let foreground = windows.foreground().expect("foreground should report a window after activation");
        assert_eq!(foreground.handle, by_class);
        assert_eq!(foreground.process_name, "Alacritty");

        windows
            .activate(previous.handle, Duration::from_secs(2))
            .expect("restoring the previous window failed");
    }
}
