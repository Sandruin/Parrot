# Porting to Linux (Wayland)

Everything the engine and GUI need from the operating system goes through the traits in `src/platform/traits.rs`. A Linux backend is a new module `src/platform/linux` implementing them, plus a service thread that feeds `RawInputEvent`s and handles `Win32Command`s (rename the enum when the second platform lands).

## Traits to implement

| Trait | Windows implementation | Wayland notes |
|---|---|---|
| `InputInjector` | `win32/injector.rs` on SendInput | libei through the RemoteDesktop portal (GNOME 46+, Plasma 6.1+), or uinput as a fallback that needs the input group |
| `ScreenCapture` | `win32/capture.rs` on GDI | xdg-desktop-portal Screenshot, or wlr-screencopy on wlroots compositors |
| `WindowManager` | `win32/window.rs` | no generic protocol; per compositor: hyprctl, swaymsg, KWin scripting, a GNOME shell extension |
| `Ocr` | `win32/ocr.rs` on Windows.Media.Ocr | tesseract-rs or ocrs |
| `Sleeper` | `platform/sleeper.rs`, portable | reuse as is |

The service thread (`win32/service_thread.rs`) also owns recording hooks, hotkeys and the overlay window:

- Recording needs global input events. On Wayland that means reading `/dev/input` with the evdev crate, which requires the user to be in the `input` group.
- Hotkeys: the GlobalShortcuts portal, or matching chords inside the evdev reader.
- Overlay: a layer-shell surface (wlr-layer-shell) with an input region of zero size, drawn with the same tiny-skia renderer used in `win32/overlay.rs`.

## Windows-only seams outside the platform module

- `src/main.rs` picks the platform services and currently has a `compile_error!` on non-Windows.
- `src/ui/mod.rs` elevation check, `src/ui/toolbar.rs` restart as administrator, `src/ui/overlay_scene.rs` cursor position for relative moves, `src/ui/region_picker.rs` monitor under the cursor. Each has a `#[cfg(not(windows))]` fallback; replace them with trait calls or Linux equivalents.

## Testing without a display

`src/platform/mock.rs` provides mock implementations with a virtual clock. The engine tests in `tests/` run against them and should pass unchanged on Linux.
