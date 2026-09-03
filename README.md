# Macro Recorder

A small desktop macro recorder for Windows, written in Rust with egui. Record keyboard and mouse input, edit the resulting action list, and replay it into games and desktop apps. Built for personal automation: sandbox games and repetitive workflows.

![Macro Recorder with a demo macro loaded](docs/screenshot.png)

## Features

- [x] Record keyboard, mouse and window changes into editable actions
- [x] Replay with speed factor, repeat count or infinite loop
- [x] Auto-stop when you press a key or mouse button during playback
- [x] Global hotkeys (F9 record, F10 play, Esc stop, all configurable)
- [x] Drag to reorder, double-click to edit, comments per action
- [x] Scan-code key injection so DirectInput games see the keys
- [x] Relative mouse moves for raw-input games
- [x] Wait for an image region to match, with a screen region picker
- [x] Wait for text and click on text via Windows OCR, substring or regex
- [x] Wait for a file to appear, with wildcards
- [x] Window activation by title and process name
- [x] On-screen overlay showing where the selected action acts
- [x] JSON macro files, hand-editable, opened from the command line
- [x] Elevation warning and restart as administrator
- [ ] Linux Wayland backend (see [docs/porting-linux.md](docs/porting-linux.md))
- [ ] Variables and conditional branches
- [ ] Image search click, like click on text but for a template

## Build and run

```
cargo run --release
cargo run --release -- path\to\macro.json
```

Requires Rust 1.95 or newer. On Windows the build embeds a per-monitor DPI manifest so coordinates are physical pixels everywhere.

## Layout

- `src/model` shared action model, macro file format, settings
- `src/platform` platform traits, mocks, and the Win32 implementation (hooks, injector, capture, overlay, OCR)
- `src/engine` recorder, player, scheduler, image and text matching
- `src/ui` egui application
- `examples/` small binaries for smoke testing hooks, injection, OCR and the overlay
- `scripts/screenshot.ps1` launches the app and captures its window

## Development

```
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

Set `MACRO_FAKE_ENGINE=1` to run the GUI against a scripted engine and `MACRO_DEMO_DOC=1` to preload a macro with one action of every kind.
