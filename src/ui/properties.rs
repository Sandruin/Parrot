use egui::{Button, DragValue, Modal, RichText, Slider, TextEdit, Vec2};

use super::{App, action_list, keymap, style};
use crate::model::{
    Action, ActionId, ActionItem, ButtonEvent, ImageMatchMode, Key, MouseButton, PathPoint, Point, Rect,
    TextMode, TimeUnit,
};

/// Modal editor for one action: edits a clone, commits on OK, discards on Cancel or Escape.
pub struct Dialog {
    id: ActionId,
    action: Action,
    comment: String,
    /// Set while the key picker waits for the next key press.
    capture_key: bool,
    preview: Option<Preview>,
}

enum Preview {
    Image(egui::TextureHandle),
    Failed,
}

impl Dialog {
    pub fn new(item: &ActionItem) -> Self {
        Self {
            id: item.id,
            action: item.action.clone(),
            comment: item.comment.clone(),
            capture_key: false,
            preview: None,
        }
    }
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut dialog) = app.dialog.take() else {
        return;
    };
    let captured = capture_pressed_key(&mut dialog, ctx);
    let mut commit = false;
    let mut cancel = false;

    let response = Modal::new(egui::Id::new("action_properties")).show(ctx, |ui| {
        ui.set_min_width(470.0);
        ui.horizontal(|ui| {
            ui.label(
                action_list::icon_for(&dialog.action)
                    .rich_text()
                    .size(20.0)
                    .color(style::accent(ui.visuals())),
            );
            ui.heading(dialog.action.kind_name());
        });
        ui.add_space(8.0);

        let Dialog { action, comment, capture_key, preview, .. } = &mut dialog;
        egui::Grid::new("action_fields").num_columns(2).spacing([14.0, 8.0]).min_col_width(104.0).show(
            ui,
            |ui| {
                fields(ui, ctx, action, capture_key, preview);
                row(ui, "Comment", |ui| {
                    ui.add(TextEdit::singleline(comment).desired_width(280.0));
                });
            },
        );

        ui.add_space(12.0);
        ui.separator();
        ui.horizontal(|ui| {
            let accent = style::accent(ui.visuals());
            let ok = Button::new(RichText::new("OK").color(egui::Color32::WHITE))
                .fill(accent)
                .min_size(Vec2::new(76.0, 0.0));
            if ui.add(ok).clicked() {
                commit = true;
            }
            if ui.add(Button::new("Cancel").min_size(Vec2::new(76.0, 0.0))).clicked() {
                cancel = true;
            }
        });
    });

    if !captured && response.should_close() {
        cancel = true;
    }
    if commit {
        if let Some(item) = app.doc.item_mut(dialog.id) {
            item.action = dialog.action.clone();
            item.comment = dialog.comment.clone();
            app.dirty = true;
        }
        return;
    }
    if !cancel {
        app.dialog = Some(dialog);
    }
}

fn fields(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    action: &mut Action,
    capture_key: &mut bool,
    preview: &mut Option<Preview>,
) {
    match action {
        Action::KeyDown { .. } | Action::KeyUp { .. } | Action::KeyPress { .. } => {
            key_fields(ui, action, capture_key);
        }
        Action::Wait { duration, unit } => {
            row(ui, "Duration", |ui| {
                ui.add(DragValue::new(duration).range(0.0..=3_600_000.0).speed(1.0));
                combo(ui, "wait_unit", unit, &TimeUnit::ALL, |u| u.label().to_string());
            });
        }
        Action::TypeText { text, mode, char_delay_ms } => {
            row(ui, "Text", |ui| {
                ui.add(TextEdit::multiline(text).desired_rows(4).desired_width(300.0));
            });
            row(ui, "Mode", |ui| {
                combo(ui, "text_mode", mode, &[TextMode::Unicode, TextMode::ScanCodes], |m| match m {
                    TextMode::Unicode => "Unicode".into(),
                    TextMode::ScanCodes => "Scan codes (games)".into(),
                });
            });
            row(ui, "Per character", |ui| {
                ui.add(DragValue::new(char_delay_ms).range(0..=5_000).suffix(" ms"));
            });
        }
        Action::MouseMove { path } => {
            let mut end = path.last().copied().unwrap_or_default();
            row(ui, "Endpoint", |ui| {
                ui.add(DragValue::new(&mut end.x).prefix("x "));
                ui.add(DragValue::new(&mut end.y).prefix("y "));
            });
            let mut straighten = false;
            row(ui, "Path", |ui| {
                ui.label(format!("{} points", path.len()));
                if ui
                    .add_enabled(path.len() > 1, Button::new("Straighten"))
                    .on_hover_text("Drop the recorded path and jump straight to the endpoint")
                    .clicked()
                {
                    straighten = true;
                }
            });
            if straighten {
                *path = vec![PathPoint { x: end.x, y: end.y, dt_ms: 0 }];
            } else if let Some(last) = path.last_mut() {
                last.x = end.x;
                last.y = end.y;
            } else {
                path.push(PathPoint { x: end.x, y: end.y, dt_ms: 0 });
            }
        }
        Action::MouseButton { button, event, pos } => {
            row(ui, "Button", |ui| {
                combo(ui, "mouse_button", button, &MouseButton::ALL, |b| b.label().to_string());
            });
            row(ui, "Event", |ui| {
                combo(ui, "button_event", event, &ButtonEvent::ALL, |e| e.label().to_string());
            });
            row(ui, "Position", |ui| position(ui, pos));
        }
        Action::MouseWheel { delta, horizontal, pos } => {
            let mut notches = *delta / 120;
            row(ui, "Notches", |ui| {
                ui.add(DragValue::new(&mut notches).range(-50..=50));
                ui.weak(format!("delta {}", notches * 120));
            });
            *delta = notches * 120;
            row(ui, "Axis", |ui| {
                ui.checkbox(horizontal, "Horizontal");
            });
            row(ui, "Position", |ui| position(ui, pos));
        }
        Action::WindowActivate { title_contains, process_name, timeout_ms } => {
            row(ui, "Title contains", |ui| {
                ui.add(TextEdit::singleline(title_contains).desired_width(280.0));
            });
            row(ui, "Process name", |ui| {
                ui.add(TextEdit::singleline(process_name).hint_text("notepad.exe").desired_width(280.0));
            });
            millis(ui, "Timeout", timeout_ms);
            row(ui, "", |ui| {
                ui.add_enabled(false, Button::new("Use current foreground"))
                    .on_disabled_hover_text("Picking the foreground window comes in a later phase");
            });
        }
        Action::WaitForImage { region, template_png, similarity, poll_ms, timeout_ms, mode } => {
            region_rows(ui, region);
            row(ui, "Similarity", |ui| {
                ui.add(Slider::new(similarity, 0.5..=1.0).fixed_decimals(2));
            });
            millis(ui, "Poll every", poll_ms);
            millis(ui, "Timeout", timeout_ms);
            row(ui, "Mode", |ui| {
                let modes = [ImageMatchMode::Exact, ImageMatchMode::Search];
                combo(ui, "image_mode", mode, &modes, |m| match m {
                    ImageMatchMode::Exact => "Exact region".into(),
                    ImageMatchMode::Search => "Search in region".into(),
                });
            });
            row(ui, "Template", |ui| template(ui, ctx, template_png, preview));
        }
        Action::WaitForText { region, text, case_sensitive, poll_ms, timeout_ms } => {
            region_rows(ui, region);
            row(ui, "Text", |ui| {
                ui.add(TextEdit::singleline(text).desired_width(280.0));
            });
            row(ui, "Case", |ui| {
                ui.checkbox(case_sensitive, "Case sensitive");
            });
            millis(ui, "Poll every", poll_ms);
            millis(ui, "Timeout", timeout_ms);
        }
        Action::WaitForFile { path, timeout_ms } => {
            row(ui, "Path", |ui| {
                ui.add(TextEdit::singleline(path).desired_width(280.0));
            });
            millis(ui, "Timeout", timeout_ms);
        }
        Action::Comment { text } => {
            row(ui, "Text", |ui| {
                ui.add(TextEdit::multiline(text).desired_rows(3).desired_width(300.0));
            });
        }
        Action::Label { name } => {
            row(ui, "Name", |ui| {
                ui.add(TextEdit::singleline(name).desired_width(280.0));
            });
        }
    }
}

fn key_fields(ui: &mut egui::Ui, action: &mut Action, capture_key: &mut bool) {
    let Some(key) = key_of(action) else {
        return;
    };
    let mut kind = KeyKind::of(action);
    row(ui, "Key", |ui| {
        let text = if *capture_key {
            RichText::new("Press any key...").color(style::accent(ui.visuals()))
        } else {
            RichText::new(key.name())
        };
        if ui
            .add(Button::new(text).selected(*capture_key).min_size(Vec2::new(170.0, 0.0)))
            .on_hover_text("Click, then press the key you want")
            .clicked()
        {
            *capture_key = true;
        }
        ui.weak(format!("vk 0x{:02X}", key.vk));
    });
    row(ui, "Event", |ui| {
        combo(ui, "key_kind", &mut kind, &KeyKind::ALL, |k| k.label().to_string());
    });
    *action = kind.build(key);
}

fn position(ui: &mut egui::Ui, pos: &mut Option<Point>) {
    let mut at_cursor = pos.is_none();
    let mut point = pos.unwrap_or_default();
    ui.checkbox(&mut at_cursor, "At cursor");
    ui.add_enabled_ui(!at_cursor, |ui| {
        ui.add(DragValue::new(&mut point.x).prefix("x "));
        ui.add(DragValue::new(&mut point.y).prefix("y "));
    });
    *pos = if at_cursor { None } else { Some(point) };
}

fn region_rows(ui: &mut egui::Ui, region: &mut Rect) {
    row(ui, "Region", |ui| {
        ui.add(DragValue::new(&mut region.x).prefix("x "));
        ui.add(DragValue::new(&mut region.y).prefix("y "));
        ui.add(DragValue::new(&mut region.w).range(1..=32_768).prefix("w "));
        ui.add(DragValue::new(&mut region.h).range(1..=32_768).prefix("h "));
    });
    row(ui, "", |ui| {
        ui.add_enabled(false, Button::new("Capture region..."))
            .on_disabled_hover_text("The region picker comes in a later phase");
    });
}

fn template(ui: &mut egui::Ui, ctx: &egui::Context, png: &[u8], preview: &mut Option<Preview>) {
    if png.is_empty() {
        ui.weak("No template captured yet");
        return;
    }
    if preview.is_none() {
        *preview = Some(match decode(ctx, png) {
            Some(texture) => Preview::Image(texture),
            None => Preview::Failed,
        });
    }
    match preview {
        Some(Preview::Image(texture)) => {
            let sized = egui::load::SizedTexture::from_handle(texture);
            ui.add(egui::Image::new(sized).max_size(Vec2::new(180.0, 120.0)));
            ui.weak(format!("{} x {} px", sized.size.x, sized.size.y));
        }
        _ => {
            ui.colored_label(ui.visuals().error_fg_color, "Template image cannot be decoded");
        }
    }
}

fn decode(ctx: &egui::Context, png: &[u8]) -> Option<egui::TextureHandle> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    Some(ctx.load_texture("template_preview", color, egui::TextureOptions::NEAREST))
}

fn millis(ui: &mut egui::Ui, label: &str, value: &mut u32) {
    row(ui, label, |ui| {
        ui.add(DragValue::new(value).range(0..=3_600_000).speed(10.0).suffix(" ms"));
    });
}

fn row(ui: &mut egui::Ui, label: &str, contents: impl FnOnce(&mut egui::Ui)) {
    if label.is_empty() {
        ui.label("");
    } else {
        ui.label(RichText::new(label).font(style::medium(13.0)));
    }
    ui.horizontal(|ui| contents(ui));
    ui.end_row();
}

fn combo<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut T,
    options: &[T],
    label: impl Fn(T) -> String,
) {
    egui::ComboBox::from_id_salt(id).selected_text(label(*value)).show_ui(ui, |ui| {
        for option in options {
            ui.selectable_value(value, *option, label(*option));
        }
    });
}

fn key_of(action: &Action) -> Option<Key> {
    match action {
        Action::KeyDown { key } | Action::KeyUp { key } | Action::KeyPress { key } => Some(*key),
        _ => None,
    }
}

/// Replaces the key of a key action, keeping the event kind.
fn set_key(action: &mut Action, key: Key) {
    match action {
        Action::KeyDown { key: slot } | Action::KeyUp { key: slot } | Action::KeyPress { key: slot } => {
            *slot = key;
        }
        _ => {}
    }
}

/// Consumes the next key press while the picker is armed; unknown keys keep the previous value.
fn capture_pressed_key(dialog: &mut Dialog, ctx: &egui::Context) -> bool {
    if !dialog.capture_key {
        return false;
    }
    let pressed = ctx.input_mut(|i| {
        let mut found = None;
        i.events.retain(|event| match event {
            egui::Event::Key { key, pressed: true, .. } if found.is_none() => {
                found = Some(*key);
                false
            }
            _ => true,
        });
        found
    });
    let Some(key) = pressed else {
        return false;
    };
    dialog.capture_key = false;
    if let Some(vk) = keymap::vk_from_key(key) {
        set_key(&mut dialog.action, Key::from_vk(vk));
    }
    true
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyKind {
    Down,
    Up,
    Press,
}

impl KeyKind {
    const ALL: [KeyKind; 3] = [KeyKind::Down, KeyKind::Up, KeyKind::Press];

    fn of(action: &Action) -> Self {
        match action {
            Action::KeyDown { .. } => KeyKind::Down,
            Action::KeyUp { .. } => KeyKind::Up,
            _ => KeyKind::Press,
        }
    }

    fn label(self) -> &'static str {
        match self {
            KeyKind::Down => "Key down",
            KeyKind::Up => "Key up",
            KeyKind::Press => "Key press",
        }
    }

    fn build(self, key: Key) -> Action {
        match self {
            KeyKind::Down => Action::KeyDown { key },
            KeyKind::Up => Action::KeyUp { key },
            KeyKind::Press => Action::KeyPress { key },
        }
    }
}
