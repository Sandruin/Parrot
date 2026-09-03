use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

const STYLE_UNKNOWN: u8 = 0;
const STYLE_LEGACY: u8 = 1;
const STYLE_LUA: u8 = 2;

/// Client for Hyprland's IPC sockets: requests go to `.socket.sock`, events arrive on `.socket2.sock`.
#[derive(Clone, Debug)]
pub struct Hyprland {
    dir: PathBuf,
    /// Whether the running instance parses `keyword` lines or wants Lua through `eval`.
    style: Arc<AtomicU8>,
}

/// One top-level window as reported by `j/clients` and `j/activewindow`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Client {
    pub address: String,
    pub mapped: bool,
    pub hidden: bool,
    pub title: String,
    pub class: String,
    #[serde(rename = "initialClass")]
    pub initial_class: String,
    pub pid: i32,
    pub workspace: Workspace,
    #[serde(rename = "focusHistoryID")]
    pub focus_history_id: i32,
    pub floating: bool,
    pub xwayland: bool,
}

impl Client {
    /// What the macros call the process name: the window class, which is the stable id on Wayland.
    pub fn process_name(&self) -> String {
        if !self.class.is_empty() {
            return self.class.clone();
        }
        u32::try_from(self.pid).ok().and_then(process_name).unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Workspace {
    pub id: i32,
    pub name: String,
}

/// One monitor as reported by `j/monitors`; `x` and `y` are logical, `width` and `height` are pixels.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct MonitorInfo {
    pub id: i32,
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    pub transform: i32,
    pub focused: bool,
    pub disabled: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
struct CursorPos {
    x: f64,
    y: f64,
}

impl Hyprland {
    /// The running Hyprland instance, if this process was started inside one.
    pub fn detect() -> Option<Self> {
        let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
        let candidates = [runtime.map(|r| r.join("hypr")), Some(PathBuf::from("/tmp/hypr"))];
        let dir = candidates.into_iter().flatten().map(|d| d.join(&signature)).find(|d| d.exists())?;
        Some(Self { dir, style: Arc::new(AtomicU8::new(STYLE_UNKNOWN)) })
    }

    pub fn request_socket(&self) -> PathBuf {
        self.dir.join(".socket.sock")
    }

    /// Sends one request and returns the raw reply.
    pub fn request(&self, command: &str) -> Result<String> {
        let mut stream = UnixStream::connect(self.request_socket())
            .with_context(|| format!("connecting to {}", self.request_socket().display()))?;
        stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
        stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;
        stream.write_all(command.as_bytes()).with_context(|| format!("sending {command:?} to hyprland"))?;
        let mut reply = String::new();
        stream.read_to_string(&mut reply).with_context(|| format!("reading the reply to {command:?}"))?;
        Ok(reply)
    }

    /// Sends a request with the JSON flag and parses the reply.
    pub fn json<T: DeserializeOwned>(&self, command: &str) -> Result<T> {
        let reply = self.request(&format!("j/{command}"))?;
        serde_json::from_str(&reply)
            .with_context(|| format!("parsing the reply to {command:?}: {reply:.200}"))
    }

    /// Cursor position in logical compositor coordinates.
    pub fn cursor_pos(&self) -> Result<(f64, f64)> {
        let pos: CursorPos = self.json("cursorpos")?;
        Ok((pos.x, pos.y))
    }

    pub fn clients(&self) -> Result<Vec<Client>> {
        self.json("clients")
    }

    /// The focused window, `None` when nothing has focus.
    pub fn active_window(&self) -> Result<Option<Client>> {
        let reply = self.request("j/activewindow")?;
        let trimmed = reply.trim();
        if trimmed.is_empty() || trimmed == "{}" {
            return Ok(None);
        }
        let client: Client = serde_json::from_str(trimmed)
            .with_context(|| format!("parsing the active window: {trimmed:.200}"))?;
        Ok((!client.address.is_empty()).then_some(client))
    }

    pub fn monitors(&self) -> Result<Vec<MonitorInfo>> {
        self.json("monitors")
    }

    /// Runs a dispatcher given as classic text such as `focuswindow address:0x...` and as a Lua
    /// dispatcher expression such as `hl.dsp.focus({ window = "address:0x..." })`.
    pub fn dispatch(&self, legacy: &str, lua: &str) -> Result<()> {
        self.styled(&format!("dispatch {legacy}"), "hl.dispatch", &format!("eval return hl.dispatch({lua})"))
    }

    /// Applies a config line: `legacy` through `keyword` on classic configs, `lua` through `eval` otherwise.
    pub fn configure(&self, legacy: &str, lua: &str) -> Result<()> {
        self.styled(&format!("keyword {legacy}"), "non-legacy", &format!("eval {lua}"))
    }

    /// Sends the classic request unless the instance is known to parse Lua; an error reply containing
    /// `marker` reveals the Lua parser and switches every later call to the Lua form.
    fn styled(&self, classic: &str, marker: &str, lua: &str) -> Result<()> {
        if self.style.load(Ordering::Relaxed) != STYLE_LUA {
            let reply = self.request(classic)?;
            if reply.trim() == "ok" {
                self.style.store(STYLE_LEGACY, Ordering::Relaxed);
                return Ok(());
            }
            if !reply.contains(marker) {
                bail!("hyprctl {classic}: {}", reply.trim());
            }
            self.style.store(STYLE_LUA, Ordering::Relaxed);
        }
        let reply = self.request(lua)?;
        expect_ok(&reply, lua)
    }

    /// Connects to the event socket; lines of the form `name>>data` arrive as things happen.
    pub fn events(&self) -> Result<UnixStream> {
        let path = self.dir.join(".socket2.sock");
        UnixStream::connect(&path).with_context(|| format!("connecting to {}", path.display()))
    }
}

fn expect_ok(reply: &str, what: &str) -> Result<()> {
    if reply.trim() == "ok" { Ok(()) } else { bail!("hyprctl {what}: {}", reply.trim()) }
}

/// Splits one event line into its name and payload.
pub fn parse_event(line: &str) -> Option<(&str, &str)> {
    line.trim_end_matches(['\r', '\n']).split_once(">>")
}

/// Numeric window handle for a Hyprland address such as `0x55a7ca346e10`.
pub fn parse_address(address: &str) -> Option<isize> {
    let hex = address.trim().trim_start_matches("0x");
    isize::from_str_radix(hex, 16).ok().filter(|handle| *handle != 0)
}

pub fn format_address(handle: isize) -> String {
    format!("0x{handle:x}")
}

/// Executable file name of a process, from `/proc`.
pub fn process_name(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok();
    let from_exe = exe.and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()));
    from_exe
        .or_else(|| std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|s| s.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clients_with_unknown_fields() {
        let json = r#"[{"address":"0x55a7ca346e10","mapped":true,"hidden":false,"at":[5589,139],
            "size":[1600,958],"workspace":{"id":1,"name":"1"},"class":"Alacritty","title":"main | ~",
            "initialClass":"Alacritty","pid":2326,"xwayland":false,"focusHistoryID":0,"extra":42}]"#;
        let clients: Vec<Client> = serde_json::from_str(json).unwrap();
        assert_eq!(clients[0].pid, 2326);
        assert_eq!(clients[0].class, "Alacritty");
        assert_eq!(clients[0].workspace.id, 1);
        assert_eq!(parse_address(&clients[0].address), Some(0x55a7ca346e10));
        assert_eq!(format_address(0x55a7ca346e10), "0x55a7ca346e10");
        assert_eq!(parse_address("0x0"), None);
    }

    #[test]
    fn splits_event_lines() {
        assert_eq!(
            parse_event("activewindowv2>>0x55a7ca346e10\n"),
            Some(("activewindowv2", "0x55a7ca346e10"))
        );
        assert_eq!(
            parse_event("activewindow>>Alacritty,main | ~"),
            Some(("activewindow", "Alacritty,main | ~"))
        );
        assert_eq!(parse_event("configreloaded>>"), Some(("configreloaded", "")));
        assert_eq!(parse_event("garbage"), None);
    }

    #[test]
    fn own_process_has_a_name() {
        assert!(process_name(std::process::id()).is_some_and(|n| !n.is_empty()));
        assert_eq!(process_name(0), None);
    }
}
