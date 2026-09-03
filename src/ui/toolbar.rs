use egui::{Align, Button, Color32, DragValue, Layout, RichText, Vec2};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons;

use super::{App, Mode, files, style};
use crate::model::{
    Action, ButtonEvent, ImageMatchMode, Key, MouseButton, MousePathMode, PathPoint, Rect, Repeat, TextMode,
    TimeUnit, vk,
};

/// Top panel: file menu, transport buttons, add-action menus, edit buttons and the playback strip.
pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        file_menu(app, ui);
        ui.separator();
        transport(app, ui);
        ui.separator();
        add_menus(app, ui);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| edit_buttons(app, ui));
    });
    ui.add_space(8.0);
    playback_strip(app, ui);
    ui.add_space(6.0);
}

fn file_menu(app: &mut App, ui: &mut egui::Ui) {
    ui.menu_button((icon(icons::ICON_MENU, 17.0), "File"), |ui| {
        ui.set_min_width(190.0);
        if ui.add(Button::new("New").shortcut_text("Ctrl+N")).clicked() {
            files::new_doc(app);
        }
        if ui.add(Button::new("Open...").shortcut_text("Ctrl+O")).clicked() {
            files::open(app);
        }
        if ui.add(Button::new("Save").shortcut_text("Ctrl+S")).clicked() {
            files::save(app);
        }
        if ui.add(Button::new("Save as...").shortcut_text("Ctrl+Shift+S")).clicked() {
            files::save_as(app);
        }
        let recent = app.settings.recent_files.clone();
        ui.menu_button("Recent", |ui| {
            ui.set_min_width(260.0);
            if recent.is_empty() {
                ui.weak("Nothing yet");
            }
            for path in recent {
                let label = path
                    .file_name()
                    .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().to_string());
                if ui.button(label).on_hover_text(path.display().to_string()).clicked() {
                    files::open_path(app, path);
                }
            }
        });
        ui.separator();
        if ui.button("Settings...").clicked() {
            app.settings_open = true;
        }
        if ui.button("Exit").clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    });
}

fn transport(app: &mut App, ui: &mut egui::Ui) {
    let visuals = ui.visuals();
    let red = style::record_red(visuals);
    let green = style::play_green(visuals);
    let accent = style::accent(visuals);

    let recording = app.mode == Mode::Recording;
    let record_label = if recording { "Stop rec" } else { "Record" };
    if big_button(ui, icons::ICON_FIBER_MANUAL_RECORD, record_label, red, app.mode != Mode::Playing)
        .on_hover_text(hotkey_hint(app, crate::model::HotkeyAction::ToggleRecord, "Record"))
        .clicked()
    {
        app.toggle_record();
    }

    let play = big_button(
        ui,
        icons::ICON_PLAY_ARROW,
        "Play",
        green,
        app.mode == Mode::Idle && !app.doc.items.is_empty(),
    );
    if play
        .on_hover_text(format!(
            "{}\nHold Shift to play from the selected action",
            hotkey_hint(app, crate::model::HotkeyAction::TogglePlay, "Play")
        ))
        .clicked()
    {
        let from_selection = ui.input(|i| i.modifiers.shift);
        app.toggle_play(from_selection);
    }

    if big_button(ui, icons::ICON_STOP, "Stop", accent, app.mode.is_busy())
        .on_hover_text(hotkey_hint(app, crate::model::HotkeyAction::Stop, "Stop"))
        .clicked()
    {
        app.stop();
    }
}

/// Add-action menus, folded into a single Add menu when the toolbar runs out of room.
fn add_menus(app: &mut App, ui: &mut egui::Ui) {
    let enabled = !app.mode.is_busy();
    let compact = ui.available_width() < 480.0;
    ui.add_enabled_ui(enabled, |ui| {
        if compact {
            menu(ui, icons::ICON_PLAYLIST_ADD, "Add", |ui| {
                menu(ui, icons::ICON_MOUSE, "Mouse", |ui| mouse_items(app, ui));
                menu(ui, icons::ICON_KEYBOARD, "Key", |ui| key_items(app, ui));
                if ui.button("Wait").clicked() {
                    app.add_action(wait_action());
                }
                menu(ui, icons::ICON_IMAGE, "Image", |ui| image_items(app, ui));
                menu(ui, icons::ICON_MORE_HORIZ, "Misc", |ui| misc_items(app, ui));
            });
            return;
        }
        menu(ui, icons::ICON_MOUSE, "Mouse", |ui| mouse_items(app, ui));
        menu(ui, icons::ICON_KEYBOARD, "Key", |ui| key_items(app, ui));
        if ui
            .add(Button::new((icon(icons::ICON_SCHEDULE, 16.0), "Wait")))
            .on_hover_text("Insert a wait")
            .clicked()
        {
            app.add_action(wait_action());
        }
        menu(ui, icons::ICON_IMAGE, "Image", |ui| image_items(app, ui));
        menu(ui, icons::ICON_MORE_HORIZ, "Misc", |ui| misc_items(app, ui));
    });
}

fn mouse_items(app: &mut App, ui: &mut egui::Ui) {
    if ui.button("Click").clicked() {
        app.add_action(Action::MouseButton {
            button: MouseButton::Left,
            event: ButtonEvent::Click,
            pos: None,
        });
    }
    if ui.button("Move").clicked() {
        app.add_action(Action::MouseMove { path: vec![PathPoint::default()] });
    }
    if ui.button("Wheel").clicked() {
        app.add_action(Action::MouseWheel { delta: -120, horizontal: false, pos: None });
    }
}

fn key_items(app: &mut App, ui: &mut egui::Ui) {
    if ui.button("Key down").clicked() {
        app.add_action(Action::KeyDown { key: default_key() });
    }
    if ui.button("Key up").clicked() {
        app.add_action(Action::KeyUp { key: default_key() });
    }
    if ui.button("Key press").clicked() {
        app.add_action(Action::KeyPress { key: default_key() });
    }
    if ui.button("Type text").clicked() {
        app.add_action(Action::TypeText { text: String::new(), mode: TextMode::Unicode, char_delay_ms: 10 });
    }
}

fn image_items(app: &mut App, ui: &mut egui::Ui) {
    if ui.button("Wait for image").clicked() {
        app.add_action(Action::WaitForImage {
            region: Rect::new(0, 0, 200, 200),
            template_png: Vec::new(),
            similarity: 0.9,
            poll_ms: 250,
            timeout_ms: 10_000,
            mode: ImageMatchMode::Search,
        });
    }
}

fn misc_items(app: &mut App, ui: &mut egui::Ui) {
    if ui.button("Window activate").clicked() {
        app.add_action(Action::WindowActivate {
            title_contains: String::new(),
            process_name: String::new(),
            timeout_ms: 5_000,
        });
    }
    if ui.button("Comment").clicked() {
        app.add_action(Action::Comment { text: String::new() });
    }
    if ui.button("Label").clicked() {
        app.add_action(Action::Label { name: String::new() });
    }
}

fn edit_buttons(app: &mut App, ui: &mut egui::Ui) {
    let enabled = app.selected.is_some() && !app.mode.is_busy();
    let buttons = [
        (icons::ICON_DELETE, "Delete the selected action (Del)"),
        (icons::ICON_CONTENT_COPY, "Duplicate the selected action (Ctrl+D)"),
        (icons::ICON_EDIT, "Edit the selected action (Enter)"),
    ];
    for (index, (glyph, hint)) in buttons.into_iter().enumerate() {
        let button = Button::new(icon(glyph, 17.0)).min_size(Vec2::new(34.0, 30.0));
        if ui.add_enabled(enabled, button).on_hover_text(hint).clicked() {
            match index {
                0 => app.delete_selected(),
                1 => app.duplicate_selected(),
                _ => app.open_properties(),
            }
        }
    }
}

fn wait_action() -> Action {
    Action::Wait { duration: 100.0, unit: TimeUnit::Ms }
}

fn playback_strip(app: &mut App, ui: &mut egui::Ui) {
    let enabled = !app.mode.is_busy();
    ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| {
            ui.label(caption("Speed"));
            let speed = ui.add(
                DragValue::new(&mut app.doc.settings.speed_percent).range(1..=2000).speed(1.0).suffix(" %"),
            );
            ui.separator();

            ui.label(caption("Repeat"));
            let mut infinite = matches!(app.doc.settings.repeat, Repeat::Infinite);
            let mut count = match app.doc.settings.repeat {
                Repeat::Count(n) => n,
                Repeat::Infinite => 1,
            };
            let count_changed =
                ui.add_enabled(!infinite, DragValue::new(&mut count).range(1..=1_000_000)).changed();
            let infinite_changed = ui.checkbox(&mut infinite, "Infinite").changed();
            if count_changed || infinite_changed {
                app.doc.settings.repeat =
                    if infinite { Repeat::Infinite } else { Repeat::Count(count.max(1)) };
                app.dirty = true;
            }
            ui.separator();

            ui.label(caption("Mouse path"));
            let mut path_mode = app.doc.settings.mouse_path;
            egui::ComboBox::from_id_salt("mouse_path").selected_text(path_mode_label(path_mode)).show_ui(
                ui,
                |ui| {
                    for mode in [MousePathMode::AsRecorded, MousePathMode::Straight] {
                        ui.selectable_value(&mut path_mode, mode, path_mode_label(mode));
                    }
                },
            );
            if path_mode != app.doc.settings.mouse_path {
                app.doc.settings.mouse_path = path_mode;
                app.dirty = true;
            }
            ui.separator();

            let stop_on_input = ui
                .checkbox(&mut app.doc.settings.stop_on_user_input, "Stop on user input")
                .on_hover_text("Any real key or mouse button stops playback");
            if speed.changed() || stop_on_input.changed() {
                app.dirty = true;
            }
        });
    });
}

fn menu(ui: &mut egui::Ui, glyph: MaterialIcon, label: &str, contents: impl FnOnce(&mut egui::Ui)) {
    ui.menu_button((icon(glyph, 16.0), label), |ui| {
        ui.set_min_width(160.0);
        contents(ui);
    });
}

fn big_button(
    ui: &mut egui::Ui,
    glyph: MaterialIcon,
    label: &str,
    tint: Color32,
    enabled: bool,
) -> egui::Response {
    let tint = if enabled { tint } else { ui.visuals().weak_text_color() };
    let atoms = (glyph.rich_text().size(20.0).color(tint), RichText::new(label).font(style::medium(13.5)));
    ui.add_enabled(enabled, Button::new(atoms).min_size(Vec2::new(84.0, 34.0)))
}

fn icon(glyph: MaterialIcon, size: f32) -> RichText {
    glyph.rich_text().size(size)
}

fn caption(text: &str) -> RichText {
    RichText::new(text).font(style::medium(12.5))
}

fn default_key() -> Key {
    Key::from_vk(vk::RETURN)
}

fn path_mode_label(mode: MousePathMode) -> &'static str {
    match mode {
        MousePathMode::AsRecorded => "As recorded",
        MousePathMode::Straight => "Straight",
    }
}

fn hotkey_hint(app: &App, action: crate::model::HotkeyAction, label: &str) -> String {
    match app.settings.hotkeys.get(action) {
        Some(hotkey) => format!("{label} ({hotkey})"),
        None => label.to_string(),
    }
}
