# The Linux backend

Everything the engine and GUI need from the operating system goes through the traits in `src/platform/traits.rs`.
`src/platform/linux` implements them for Wayland compositors, with Hyprland specifics kept behind its IPC client.
`src/platform/mod.rs` re-exports the active backend as `platform::native`, which is all `main.rs` and the examples use.

## Pieces

| Trait or job | Module | How |
|---|---|---|
| `InputInjector` | `injector.rs`, `keymap.rs` | wlr-virtual-pointer (one pointer per output for absolute moves) and virtual-keyboard: a keyboard carrying the seat's xkb keymap for scan-code keys, a second one with a generated keymap for Unicode text |
| `ScreenCapture` | `capture.rs` | wlr-screencopy per output, stitched and cropped to the requested physical rectangle |
| `WindowManager` | `window.rs` | Hyprland IPC: `j/clients`, `j/activewindow`, `focuswindow` |
| `Ocr` | `ocr.rs` | system tesseract through the `tesseract` crate, word boxes from its TSV output |
| Recording | `service.rs`, `input.rs`, `keys.rs` | evdev devices from `/dev/input`, cursor position polled from `hyprctl cursorpos` at 60 Hz while recording, foreground changes from the Hyprland event socket |
| Hotkeys | `hotkeys.rs` | Hyprland binds with the `global` dispatcher plus the `hyprland-global-shortcuts-v1` protocol, so the chord never reaches the focused app; software matching on the evdev stream as fallback |
| Overlay | `overlay.rs` | wlr-layer-shell surfaces on the overlay layer with an empty input region, pixels from `platform/overlay_render.rs` through a viewport so fractional scales stay crisp |
| Sleeper | `platform/sleeper.rs` | portable |

`service.rs` is the counterpart of the Win32 service thread: a calloop event loop owning the Wayland connection for the overlay and shortcuts, the evdev readers, the Hyprland event socket and the command channel.
Injector and capture each keep their own Wayland connection so the player thread can use them synchronously.

## Coordinates

Macros store physical pixels.
Wayland lays outputs out in logical units, so `layout.rs` places every monitor at its logical position times the largest scale in use and keeps its pixel size; monitors never overlap and a single or uniformly scaled setup maps exactly.
Cursor positions from Hyprland are logical integers, which limits recorded positions to the scale's granularity (1.6 pixels at scale 1.6).

## Keys

`keys.rs` maps evdev codes to the Windows virtual-key code and set 1 scan code the model uses, matching the extended-key conventions of the Win32 backend so a macro recorded on one platform plays on the other.
Letters and digits follow the layout: the xkb keymap decides that the key at the position of US `Z` records as `Y` on a German layout, exactly like Windows does.
Injection prefers the scan code (position) and falls back to the virtual-key code through the layout.

## Hyprland specifics

The IPC client in `hyprland.rs` speaks both config styles: `keyword` and `dispatch <text>` on classic configs, `eval` with Lua (`hl.bind`, `hl.layer_rule`, `hl.dispatch(hl.dsp...)`) on the Lua parser, detected from the first refusal.
Binds are re-applied when the `configreloaded` event arrives since a reload drops dynamic ones.
A `no_screen_share` layer rule keeps the overlay out of captures unless `MACRO_OVERLAY_CAPTURABLE` is set.

## Limits

- The recorder sees hardware input only: taps on touchpads are synthesized by the compositor and are not recorded, physical buttons and mice are.
- Without Hyprland there is no cursor position, no window activation and no swallowed hotkey chords; fallback chords still trigger but also reach the focused application.
- Injected input is compositor level, so recordings never contain the macro's own playback and every physical key or button press interrupts playback when auto-stop is on.

## Other compositors

Cursor position, window list, focus and binds are the only compositor-specific parts and all live behind `hyprland.rs`, `window.rs` and `hotkeys.rs`.
A sway or KWin port would add an equivalent client (`swaymsg`, KWin scripting) and choose it in `service.rs` and `platform::linux::services`.
