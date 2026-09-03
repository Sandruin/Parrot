use egui::{Button, DragValue, RichText, Vec2};

use super::{App, keymap, style};
use crate::model::{EngineCommand, Hotkey, HotkeyAction};

/// Settings window: hotkey editor, recording options and the overlay toggle.
pub fn show(app: &mut App, ctx: &egui::Context) {
    if !app.settings_open {
        return;
    }
    let mut changed = capture_hotkey(app, ctx);
    let mut open = true;

    egui::Window::new("Settings")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(440.0)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            ui.label(section("Hotkeys", ui));
            ui.add_space(4.0);
            egui::Grid::new("hotkeys").num_columns(3).spacing([12.0, 8.0]).show(ui, |ui| {
                for action in HotkeyAction::ALL {
                    ui.label(action.label());
                    let capturing = app.hotkey_capture == Some(action);
                    let text = if capturing {
                        RichText::new("Press a chord...").color(style::accent(ui.visuals()))
                    } else {
                        RichText::new(
                            app.settings
                                .hotkeys
                                .get(action)
                                .map_or_else(|| "not set".to_string(), |h| h.to_string()),
                        )
                    };
                    if ui
                        .add(Button::new(text).selected(capturing).min_size(Vec2::new(150.0, 0.0)))
                        .on_hover_text("Click, then press the key combination")
                        .clicked()
                    {
                        app.hotkey_capture = Some(action);
                    }
                    if ui.button("Clear").clicked() {
                        app.settings.hotkeys.set(action, None);
                        app.hotkey_capture = None;
                        changed = true;
                    }
                    ui.end_row();
                }
            });

            ui.add_space(12.0);
            ui.separator();
            ui.label(section("Recording", ui));
            ui.add_space(4.0);
            let record = &mut app.settings.record;
            changed |= ui.checkbox(&mut record.record_mouse_moves, "Record mouse moves").changed();
            changed |= ui
                .add_enabled(
                    record.record_mouse_moves,
                    egui::Checkbox::new(
                        &mut record.relative_mouse_moves,
                        "Record mouse moves as relative steps (raw input games)",
                    ),
                )
                .on_hover_text("Stores cursor deltas instead of absolute screen positions")
                .changed();
            changed |= ui.checkbox(&mut record.record_window_changes, "Record window changes").changed();
            changed |= ui.checkbox(&mut record.fold_clicks, "Fold button down and up into a click").changed();
            changed |=
                ui.checkbox(&mut record.fold_key_presses, "Fold key down and up into a key press").changed();
            ui.horizontal(|ui| {
                ui.label("Shortest recorded wait");
                changed |=
                    ui.add(DragValue::new(&mut record.min_wait_ms).range(0..=5_000).suffix(" ms")).changed();
            });

            ui.add_space(12.0);
            ui.separator();
            ui.label(section("Overlay", ui));
            ui.add_space(4.0);
            changed |= ui
                .checkbox(&mut app.settings.show_overlay, "Highlight the selected action on screen")
                .changed();
        });

    if !open {
        app.settings_open = false;
        app.hotkey_capture = None;
    }
    if changed {
        app.engine.send(EngineCommand::SetHotkeys(app.settings.hotkeys.clone()));
        if let Err(e) = app.settings.save_default() {
            app.error(format!("Cannot save settings: {e:#}"));
        }
    }
}

fn section(text: &str, ui: &egui::Ui) -> RichText {
    RichText::new(text).font(style::medium(13.0)).color(style::accent(ui.visuals()))
}

/// Takes the next key press as the hotkey chord for the armed action.
fn capture_hotkey(app: &mut App, ctx: &egui::Context) -> bool {
    let Some(action) = app.hotkey_capture else {
        return false;
    };
    let pressed = ctx.input_mut(|i| {
        let mut found = None;
        i.events.retain(|event| match event {
            egui::Event::Key { key, modifiers, pressed: true, .. }
                if found.is_none() && !keymap::is_modifier_key(*key) =>
            {
                found = Some((*key, *modifiers));
                false
            }
            _ => true,
        });
        found
    });
    let Some((key, modifiers)) = pressed else {
        return false;
    };
    app.hotkey_capture = None;
    if key == egui::Key::Escape && modifiers.is_none() {
        return false;
    }
    match keymap::vk_from_key(key) {
        Some(vk) => {
            app.settings.hotkeys.set(action, Some(Hotkey::new(keymap::modifier_flags(modifiers), vk)));
            true
        }
        None => {
            app.error("That key cannot be used as a hotkey");
            false
        }
    }
}
