use anyhow::Result;
use wayland_client::{Connection, Dispatch, QueueHandle};

use super::hyprland::Hyprland;
use super::keys;
use super::protocols::hyprland_global_shortcut_v1::{self, HyprlandGlobalShortcutV1};
use super::wayland::{State, Wayland};
use crate::model::{Hotkey, HotkeyAction, HotkeyConfig, modifiers, vk};

/// `app_id` this backend registers with the global-shortcuts protocol and every Hyprland bind.
const APP_ID: &str = "macro-recorder";
/// `trigger_description` handed to the protocol; the actual chord lives only in `HotkeyConfig`.
const TRIGGER_DESCRIPTION: &str = "configured in the Macro Recorder settings";

/// Shortcut ids the Wayland dispatcher saw pressed since the last drain.
#[derive(Default)]
pub struct WlState {
    pub pressed: Vec<String>,
}

/// Global hotkeys: compositor binds where available, software chord matching on the evdev stream otherwise.
pub struct Hotkeys {
    hyprland: Option<Hyprland>,
    /// Global-shortcuts protocol objects, registered once per action and kept alive for the process.
    shortcuts: Vec<(HotkeyAction, HyprlandGlobalShortcutV1)>,
    /// Chords currently bound through Hyprland.
    binds: Vec<(HotkeyAction, Hotkey)>,
    /// Chords matched in `on_key` because Hyprland refused them or a bind was not attempted.
    fallback: Vec<(Hotkey, HotkeyAction)>,
    /// Modifier state tracked from the raw vk stream, for matching fallback chords.
    mods: u8,
    /// Trigger key currently held for a fired fallback chord, so a stray repeat does not refire it.
    firing_vk: Option<u16>,
}

impl Hotkeys {
    pub fn new(hyprland: Option<Hyprland>) -> Self {
        Self {
            hyprland,
            shortcuts: Vec::new(),
            binds: Vec::new(),
            fallback: Vec::new(),
            mods: 0,
            firing_vk: None,
        }
    }

    /// Registers the chords for the busy state; the Stop chord only while `busy`.
    pub fn set(&mut self, wl: &mut Wayland, config: &HotkeyConfig, busy: bool) -> Result<()> {
        self.ensure_registered(wl);
        let protocol_available = wl.state.global_shortcuts.is_some();
        let active = active_chords(config, busy);

        let mut kept = Vec::new();
        for (action, hotkey) in std::mem::take(&mut self.binds) {
            if active.iter().any(|(a, h)| *a == action && *h == hotkey) {
                kept.push((action, hotkey));
            } else if let Some(hyprland) = &self.hyprland {
                unbind_hotkey(hyprland, hotkey);
            }
        }
        self.binds = kept;

        let mut fallback = Vec::new();
        for (action, hotkey) in active {
            if self.binds.iter().any(|(a, h)| *a == action && *h == hotkey) {
                continue;
            }
            let bound = self.try_bind(hotkey, action, protocol_available);
            if bound {
                self.binds.push((action, hotkey));
            } else {
                fallback.push((hotkey, action));
            }
        }
        self.fallback = fallback;
        Ok(())
    }

    /// Attempts a compositor bind for one chord, logging and returning `false` when it must fall back.
    fn try_bind(&self, hotkey: Hotkey, action: HotkeyAction, protocol_available: bool) -> bool {
        let Some(hyprland) = &self.hyprland else {
            log::info!(
                "not running under hyprland; hotkey {hotkey} uses the fallback matcher, which cannot \
                 swallow it from other applications"
            );
            return false;
        };
        if !protocol_available {
            log::info!(
                "the hyprland global-shortcuts protocol is unavailable; hotkey {hotkey} uses the \
                 fallback matcher, which cannot swallow it from other applications"
            );
            return false;
        }
        let Some(name) = key_name(hotkey.vk) else {
            log::info!(
                "hotkey {hotkey} has no compositor key name; using the fallback matcher, which cannot \
                 swallow it from other applications"
            );
            return false;
        };
        match bind_hotkey(hyprland, action, hotkey, &name) {
            Ok(()) => true,
            Err(e) => {
                log::info!(
                    "hyprland refused hotkey {hotkey} ({e:#}); using the fallback matcher, which cannot \
                     swallow it from other applications"
                );
                false
            }
        }
    }

    /// Re-applies the current binds after the compositor reloaded its configuration.
    pub fn reapply(&mut self, wl: &mut Wayland) {
        self.ensure_registered(wl);
        let Some(hyprland) = &self.hyprland else { return };
        let current = std::mem::take(&mut self.binds);
        for (action, hotkey) in current {
            let Some(name) = key_name(hotkey.vk) else { continue };
            match bind_hotkey(hyprland, action, hotkey, &name) {
                Ok(()) => self.binds.push((action, hotkey)),
                Err(e) => {
                    log::info!(
                        "re-binding hotkey {hotkey} after a hyprland config reload failed ({e:#}); using \
                         the fallback matcher, which cannot swallow it from other applications"
                    );
                    self.fallback.push((hotkey, action));
                }
            }
        }
    }

    /// Removes every bind this process added.
    pub fn clear(&mut self) {
        if let Some(hyprland) = &self.hyprland {
            for (_, hotkey) in self.binds.drain(..) {
                unbind_hotkey(hyprland, hotkey);
            }
        } else {
            self.binds.clear();
        }
        self.fallback.clear();
        for (_, shortcut) in self.shortcuts.drain(..) {
            shortcut.destroy();
        }
    }

    /// Feeds one physical key to the software matcher; returns the action of a completed fallback chord.
    pub fn on_key(&mut self, vk: u16, down: bool) -> Option<HotkeyAction> {
        self.track_modifier(vk, down);
        self.handle_fallback(vk, down)
    }

    fn track_modifier(&mut self, code: u16, down: bool) {
        let flag = match code {
            vk::SHIFT | vk::LSHIFT | vk::RSHIFT => modifiers::SHIFT,
            vk::CONTROL | vk::LCONTROL | vk::RCONTROL => modifiers::CONTROL,
            vk::MENU | vk::LMENU | vk::RMENU => modifiers::ALT,
            vk::LWIN | vk::RWIN => modifiers::WIN,
            _ => return,
        };
        if down {
            self.mods |= flag;
        } else {
            self.mods &= !flag;
        }
    }

    fn handle_fallback(&mut self, code: u16, down: bool) -> Option<HotkeyAction> {
        if !down {
            if self.firing_vk == Some(code) {
                self.firing_vk = None;
            }
            return None;
        }
        if self.firing_vk.is_some() {
            return None;
        }
        let mods = self.mods;
        let action = self.fallback.iter().find(|(h, _)| h.vk == code && h.modifiers == mods).map(|(_, a)| *a);
        if action.is_some() {
            self.firing_vk = Some(code);
        }
        action
    }

    /// Maps a shortcut id from the global shortcuts protocol to its action.
    pub fn action_for_id(&self, id: &str) -> Option<HotkeyAction> {
        action_for_id(id)
    }

    /// Registers one global-shortcuts protocol object per action, once for the life of this process.
    fn ensure_registered(&mut self, wl: &Wayland) {
        if !self.shortcuts.is_empty() {
            return;
        }
        let Some(manager) = wl.state.global_shortcuts.as_ref() else { return };
        for action in HotkeyAction::ALL {
            let id = action_id(action);
            let shortcut = manager.register_shortcut(
                id.to_string(),
                APP_ID.to_string(),
                action.label().to_string(),
                TRIGGER_DESCRIPTION.to_string(),
                &wl.qh,
                id.to_string(),
            );
            self.shortcuts.push((action, shortcut));
        }
    }
}

/// Configured chords active for the busy state; the Stop chord is dropped while idle.
fn active_chords(config: &HotkeyConfig, busy: bool) -> Vec<(HotkeyAction, Hotkey)> {
    config.bindings().filter(|(action, _)| busy || *action != HotkeyAction::Stop).collect()
}

fn action_id(action: HotkeyAction) -> &'static str {
    match action {
        HotkeyAction::ToggleRecord => "toggle_record",
        HotkeyAction::TogglePlay => "toggle_play",
        HotkeyAction::Stop => "stop",
    }
}

fn action_for_id(id: &str) -> Option<HotkeyAction> {
    match id {
        "toggle_record" => Some(HotkeyAction::ToggleRecord),
        "toggle_play" => Some(HotkeyAction::TogglePlay),
        "stop" => Some(HotkeyAction::Stop),
        _ => None,
    }
}

/// Adds a Hyprland bind for one chord, trying the classic `keyword` syntax and then Lua `eval`.
fn bind_hotkey(hyprland: &Hyprland, action: HotkeyAction, hotkey: Hotkey, name: &str) -> Result<()> {
    let mods = mod_names(hotkey.modifiers);
    let target = action_id(action);
    let legacy = format!("bind = {}, {name}, global, {APP_ID}:{target}", mods.join(" "));
    let lua = format!("hl.bind(\"{}\", hl.dsp.global(\"{APP_ID}:{target}\"))", lua_key(&mods, name));
    hyprland.configure(&legacy, &lua)
}

/// Removes a Hyprland bind previously added by [`bind_hotkey`].
fn unbind_hotkey(hyprland: &Hyprland, hotkey: Hotkey) {
    let Some(name) = key_name(hotkey.vk) else { return };
    let mods = mod_names(hotkey.modifiers);
    let legacy = format!("unbind = {}, {name}", mods.join(" "));
    let lua = format!("hl.unbind(\"{}\")", lua_key(&mods, &name));
    if let Err(e) = hyprland.configure(&legacy, &lua) {
        log::warn!("unbinding hotkey {hotkey} failed: {e:#}");
    }
}

/// Modifier names in Hyprland bind syntax, in the same fixed order as [`Hotkey`]'s `Display` impl.
fn mod_names(flags: u8) -> Vec<&'static str> {
    let mut out = Vec::new();
    if flags & modifiers::CONTROL != 0 {
        out.push("CTRL");
    }
    if flags & modifiers::ALT != 0 {
        out.push("ALT");
    }
    if flags & modifiers::SHIFT != 0 {
        out.push("SHIFT");
    }
    if flags & modifiers::WIN != 0 {
        out.push("SUPER");
    }
    out
}

/// The `MODS + KEY` string Hyprland's Lua `hl.bind`/`hl.unbind` expect.
fn lua_key(mods: &[&str], name: &str) -> String {
    let mut parts = mods.to_vec();
    parts.push(name);
    parts.join(" + ")
}

/// Hyprland key name for a virtual-key code: an xkb keysym name where one is defined, else `code:N`.
fn key_name(vk: u16) -> Option<String> {
    if let Some(name) = keysym_name(vk) {
        return Some(name);
    }
    keys::evdev_from_vk(vk).map(|code| format!("code:{}", code + 8))
}

/// The xkb keysym name Hyprland expects for keys with a stable, layout-independent name.
fn keysym_name(code: u16) -> Option<String> {
    Some(match code {
        0x30..=0x39 | 0x41..=0x5A => (code as u8 as char).to_string(),
        vk::F1..=vk::F24 => format!("F{}", code - vk::F1 + 1),
        vk::ESCAPE => "Escape".to_string(),
        vk::RETURN => "Return".to_string(),
        vk::SPACE => "space".to_string(),
        vk::TAB => "Tab".to_string(),
        vk::BACK => "BackSpace".to_string(),
        vk::DELETE => "Delete".to_string(),
        vk::INSERT => "Insert".to_string(),
        vk::HOME => "Home".to_string(),
        vk::END => "End".to_string(),
        vk::PRIOR => "Prior".to_string(),
        vk::NEXT => "Next".to_string(),
        vk::LEFT => "Left".to_string(),
        vk::RIGHT => "Right".to_string(),
        vk::UP => "Up".to_string(),
        vk::DOWN => "Down".to_string(),
        vk::PAUSE => "Pause".to_string(),
        vk::SNAPSHOT => "Print".to_string(),
        vk::NUMPAD0..=0x69 => format!("KP_{}", code - vk::NUMPAD0),
        vk::MULTIPLY => "KP_Multiply".to_string(),
        vk::ADD => "KP_Add".to_string(),
        vk::SUBTRACT => "KP_Subtract".to_string(),
        vk::DECIMAL => "KP_Decimal".to_string(),
        vk::DIVIDE => "KP_Divide".to_string(),
        _ => return None,
    })
}

impl Dispatch<HyprlandGlobalShortcutV1, String> for State {
    fn event(
        state: &mut Self,
        _shortcut: &HyprlandGlobalShortcutV1,
        event: hyprland_global_shortcut_v1::Event,
        id: &String,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let hyprland_global_shortcut_v1::Event::Pressed { .. } = event {
            state.shortcuts.pressed.push(id.clone());
        }
    }
}

/// Virtual-key codes taking part in any configured chord, including its modifier keys.
/// The service uses these to keep the play hotkey from triggering auto-stop.
pub fn chord_vks(config: &HotkeyConfig) -> Vec<u16> {
    let mut out = Vec::new();
    for (_, hotkey) in config.bindings() {
        push_unique(&mut out, hotkey.vk);
        for code in modifier_vks(hotkey.modifiers) {
            push_unique(&mut out, code);
        }
    }
    out
}

fn modifier_vks(flags: u8) -> Vec<u16> {
    let mut out = Vec::new();
    if flags & modifiers::ALT != 0 {
        out.extend([vk::MENU, vk::LMENU, vk::RMENU]);
    }
    if flags & modifiers::CONTROL != 0 {
        out.extend([vk::CONTROL, vk::LCONTROL, vk::RCONTROL]);
    }
    if flags & modifiers::SHIFT != 0 {
        out.extend([vk::SHIFT, vk::LSHIFT, vk::RSHIFT]);
    }
    if flags & modifiers::WIN != 0 {
        out.extend([vk::LWIN, vk::RWIN]);
    }
    out
}

fn push_unique(list: &mut Vec<u16>, code: u16) {
    if !list.contains(&code) {
        list.push(code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keysym_names_cover_letters_digits_and_function_keys() {
        assert_eq!(key_name(0x41).as_deref(), Some("A"));
        assert_eq!(key_name(0x37).as_deref(), Some("7"));
        assert_eq!(key_name(vk::F1 + 8).as_deref(), Some("F9"));
        assert_eq!(key_name(vk::SPACE).as_deref(), Some("space"));
        assert_eq!(key_name(vk::BACK).as_deref(), Some("BackSpace"));
        assert_eq!(key_name(vk::NUMPAD0 + 5).as_deref(), Some("KP_5"));
        assert_eq!(key_name(vk::ADD).as_deref(), Some("KP_Add"));
    }

    #[test]
    fn keys_without_a_keysym_name_fall_back_to_evdev_codes() {
        let name = key_name(0xBA).expect("semicolon has an evdev mapping");
        assert!(name.starts_with("code:"), "{name}");
        let evdev = keys::evdev_from_vk(0xBA).unwrap();
        assert_eq!(name, format!("code:{}", evdev + 8));
    }

    #[test]
    fn modifier_names_and_chord_strings_match_both_syntaxes() {
        let flags = modifiers::CONTROL | modifiers::SHIFT;
        assert_eq!(mod_names(flags), vec!["CTRL", "SHIFT"]);
        assert_eq!(lua_key(&mod_names(flags), "A"), "CTRL + SHIFT + A");
        assert_eq!(mod_names(flags).join(" "), "CTRL SHIFT");
        assert_eq!(lua_key(&mod_names(0), "F9"), "F9");
        assert_eq!(mod_names(0).join(" "), "");
    }

    #[test]
    fn busy_state_filters_the_stop_chord() {
        let config = HotkeyConfig::default();
        let idle = active_chords(&config, false);
        assert_eq!(idle.len(), 2);
        assert!(!idle.iter().any(|(a, _)| *a == HotkeyAction::Stop));

        let busy = active_chords(&config, true);
        assert_eq!(busy.len(), 3);
        assert!(busy.iter().any(|(a, _)| *a == HotkeyAction::Stop));
    }

    #[test]
    fn matcher_requires_exact_modifiers_and_fires_once_per_press() {
        let mut hk = Hotkeys::new(None);
        hk.fallback = vec![(Hotkey::new(modifiers::CONTROL, vk::F1 + 8), HotkeyAction::ToggleRecord)];

        assert_eq!(hk.on_key(vk::F1 + 8, true), None, "modifier not held yet");

        assert_eq!(hk.on_key(vk::LCONTROL, true), None);
        assert_eq!(hk.on_key(vk::F1 + 8, true), Some(HotkeyAction::ToggleRecord));
        assert_eq!(hk.on_key(vk::F1 + 8, true), None, "held key must not refire");
        assert_eq!(hk.on_key(vk::F1 + 8, false), None);
        assert_eq!(hk.on_key(vk::F1 + 8, true), Some(HotkeyAction::ToggleRecord), "a new press fires again");

        hk.on_key(vk::F1 + 8, false);
        hk.on_key(vk::LCONTROL, false);
        assert_eq!(hk.on_key(vk::F1 + 8, true), None, "modifier released");
    }

    #[test]
    fn action_ids_round_trip() {
        for action in HotkeyAction::ALL {
            assert_eq!(action_for_id(action_id(action)), Some(action));
        }
        assert_eq!(action_for_id("unknown"), None);
    }
}
