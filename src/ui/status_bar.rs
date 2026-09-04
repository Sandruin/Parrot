use egui::{Align, Layout, RichText};

use super::{App, Mode, style};
use crate::model::HotkeyAction;

/// Bottom panel: item count, mode, playback progress, last message and the hotkey hints.
pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.add_space(3.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} actions", app.doc.items.len())).font(style::medium(12.5)));
        if app.selection.len() > 1 {
            ui.label(
                RichText::new(format!("{} selected", app.selection.len()))
                    .font(style::medium(12.5))
                    .color(ui.visuals().weak_text_color()),
            );
        }
        ui.separator();

        let mode_color = match app.mode {
            Mode::Idle => ui.visuals().weak_text_color(),
            Mode::Recording => style::record_red(ui.visuals()),
            Mode::Playing => style::play_green(ui.visuals()),
        };
        ui.label(RichText::new(app.mode.label()).font(style::medium(12.5)).color(mode_color));

        // Windows hides keystrokes aimed at our own focused window from the low-level hook.
        if app.mode == Mode::Recording && ui.input(|i| i.focused) {
            ui.separator();
            ui.label(
                RichText::new("keys typed here are not captured")
                    .font(style::medium(12.5))
                    .color(style::record_red(ui.visuals())),
            )
            .on_hover_text(
                "Windows does not report keystrokes sent to the recorder's own window.\n\
                 Click into the program you are automating and they will be recorded.",
            );
        }

        if app.elevation_warning {
            ui.separator();
            ui.label(
                egui_material_icons::icons::ICON_WARNING
                    .rich_text()
                    .size(16.0)
                    .color(style::record_red(ui.visuals())),
            )
            .on_hover_text("The active window runs elevated; input will not reach it");
        }

        if let Some(progress) = app.progress {
            ui.separator();
            ui.label(format!("{} / {}", progress.index + 1, progress.total));
            if progress.iteration > 1 {
                ui.label(format!("iteration {}", progress.iteration));
            }
        }

        ui.separator();
        let color = if app.status.error { ui.visuals().error_fg_color } else { ui.visuals().text_color() };
        ui.add(egui::Label::new(RichText::new(&app.status.text).color(color)).truncate());

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(hotkey_hints(app)).small().color(ui.visuals().weak_text_color()));
        });
    });
    ui.add_space(3.0);
}

fn hotkey_hints(app: &App) -> String {
    let mut parts = Vec::new();
    for action in HotkeyAction::ALL {
        if let Some(hotkey) = app.settings.hotkeys.get(action) {
            let name = match action {
                HotkeyAction::ToggleRecord => "record",
                HotkeyAction::TogglePlay => "play",
                HotkeyAction::Stop => "stop",
            };
            parts.push(format!("{hotkey} {name}"));
        }
    }
    if parts.is_empty() { "no hotkeys configured".to_string() } else { parts.join("   ") }
}
