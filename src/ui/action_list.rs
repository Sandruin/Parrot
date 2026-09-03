use egui::{Align, Color32, Layout, Rect, RichText, Sense, UiBuilder, Vec2};
use egui_material_icons::MaterialIcon;
use egui_material_icons::icons;

use super::{App, style};
use crate::model::{Action, ActionId};

const ROW_HEIGHT: f32 = 26.0;
const W_DRAG: f32 = 20.0;
const W_ENABLED: f32 = 24.0;
const W_INDEX: f32 = 34.0;
const W_ACTION: f32 = 168.0;
const W_COMMENT: f32 = 190.0;

/// Width of the value column, which takes whatever the fixed columns leave.
fn value_width(total: f32, spacing: f32) -> f32 {
    let fixed = W_DRAG + W_ENABLED + W_INDEX + W_ACTION + W_COMMENT + 5.0 * spacing;
    (total - fixed).max(110.0)
}

/// Central action list: header, rows with drag reorder, selection and inline comment editing.
pub fn show(app: &mut App, ui: &mut egui::Ui) {
    shortcuts(app, ui.ctx());
    header(ui);

    if app.doc.items.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.weak("No actions yet. Record, or add one from the toolbar.");
        });
        return;
    }

    let selection = app.selection.clone();
    let focused = app.selected;
    let running = app.running;
    let editing = app.editing_comment;
    let scroll_to = app.scroll_to.take();
    let accent = style::accent(ui.visuals());
    let running_tint = style::play_green(ui.visuals());

    let can_paste = app.can_paste();
    let mut clicked = None;
    let mut open = None;
    let mut edit_comment = None;
    let mut command = None;
    let mut changed = false;

    let mods = ui.input(|i| i.modifiers);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let spacing = ui.spacing().item_spacing.x;
        let value_w = value_width(ui.available_width(), spacing);
        let response =
            egui_dnd::dnd(ui, "actions").show_vec(&mut app.doc.items, |ui, item, handle, state| {
                let is_selected = selection.contains(&item.id);
                let is_focused = focused == Some(item.id);
                let is_running = running == Some(item.id);
                let (rect, row) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::click());

                let bg = if is_running {
                    running_tint.gamma_multiply(0.35)
                } else if is_focused {
                    ui.visuals().selection.bg_fill
                } else if is_selected {
                    ui.visuals().selection.bg_fill.gamma_multiply(0.55)
                } else if state.index % 2 == 1 {
                    ui.visuals().faint_bg_color
                } else {
                    Color32::TRANSPARENT
                };
                if bg != Color32::TRANSPARENT {
                    ui.painter().rect_filled(rect, 4.0, bg);
                }
                if is_running {
                    let bar = Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height()));
                    ui.painter().rect_filled(bar, 1.0, running_tint);
                }
                if Some(item.id) == scroll_to {
                    ui.scroll_to_rect(rect, Some(Align::Center));
                }

                let dim = !item.enabled;
                let mut cells = ui.new_child(
                    UiBuilder::new()
                        .max_rect(rect.shrink2(Vec2::new(6.0, 2.0)))
                        .layout(Layout::left_to_right(Align::Center)),
                );
                let ui = &mut cells;

                let row_h = ui.available_height();
                cell(ui, W_DRAG, row_h, |ui| {
                    handle.ui(ui, |ui| {
                        let glyph = icons::ICON_DRAG_INDICATOR
                            .rich_text()
                            .size(14.0)
                            .color(ui.visuals().weak_text_color());
                        ui.add_sized(Vec2::new(W_DRAG, row_h), egui::Label::new(glyph).selectable(false));
                    });
                });
                cell(ui, W_ENABLED, row_h, |ui| {
                    if ui
                        .checkbox(&mut item.enabled, "")
                        .on_hover_text("Include this action in playback")
                        .changed()
                    {
                        changed = true;
                    }
                });
                cell(ui, W_INDEX, row_h, |ui| {
                    ui.label(
                        RichText::new(format!("{}", state.index + 1))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                });
                cell(ui, W_ACTION, row_h, |ui| {
                    let tint = if dim { ui.visuals().weak_text_color() } else { accent };
                    ui.label(icon_for(&item.action).rich_text().size(15.0).color(tint));
                    label(ui, item.action.kind_name(), dim, true);
                });
                cell(ui, value_w, row_h, |ui| label(ui, item.action.value_text(), dim, false));

                let (comment_rect, comment_response) =
                    ui.allocate_exact_size(Vec2::new(W_COMMENT, row_h), Sense::click());
                let mut comment_ui = ui.new_child(
                    UiBuilder::new().max_rect(comment_rect).layout(Layout::left_to_right(Align::Center)),
                );
                if editing == Some(item.id) {
                    let edit = comment_ui.add(
                        egui::TextEdit::singleline(&mut item.comment)
                            .id(comment_edit_id(item.id))
                            .hint_text("comment")
                            .desired_width(f32::INFINITY),
                    );
                    if edit.changed() {
                        changed = true;
                    }
                    if edit.lost_focus() {
                        edit_comment = Some(None);
                    }
                } else if !item.comment.is_empty() {
                    label(&mut comment_ui, item.comment.clone(), dim, false);
                }
                if comment_response.double_clicked() {
                    edit_comment = Some(Some(item.id));
                    comment_ui.ctx().memory_mut(|m| m.request_focus(comment_edit_id(item.id)));
                } else if comment_response.clicked() {
                    clicked = Some(item.id);
                }

                if row.clicked() {
                    clicked = Some(item.id);
                }
                if row.double_clicked() {
                    open = Some(item.id);
                }
                if row.secondary_clicked() && !is_selected {
                    clicked = Some(item.id);
                }
                row.context_menu(|ui| {
                    ui.set_min_width(170.0);
                    for (label, shortcut, wanted, enabled) in [
                        ("Cut", "Ctrl+X", RowCommand::Cut, true),
                        ("Copy", "Ctrl+C", RowCommand::Copy, true),
                        ("Paste", "Ctrl+V", RowCommand::Paste, can_paste),
                        ("Duplicate", "Ctrl+D", RowCommand::Duplicate, true),
                        ("Delete", "Del", RowCommand::Delete, true),
                        ("Properties", "Enter", RowCommand::Properties, true),
                    ] {
                        let button = egui::Button::new(label).shortcut_text(shortcut);
                        if ui.add_enabled(enabled, button).clicked() {
                            command = Some(wanted);
                            ui.close();
                        }
                    }
                });
            });
        if response.is_drag_finished() {
            changed = true;
        }
    });

    if let Some(id) = clicked {
        if mods.command {
            app.toggle_select(id);
        } else if mods.shift {
            app.extend_select(id);
        } else {
            app.select(Some(id));
        }
    }
    if let Some(target) = edit_comment {
        app.editing_comment = target;
    }
    if let Some(id) = open {
        app.select(Some(id));
        app.open_properties();
    }
    match command {
        Some(RowCommand::Cut) => app.cut_selected(ui.ctx()),
        Some(RowCommand::Copy) => app.copy_selected(ui.ctx()),
        Some(RowCommand::Paste) => app.paste_clipboard(),
        Some(RowCommand::Duplicate) => app.duplicate_selected(),
        Some(RowCommand::Delete) => app.delete_selected(),
        Some(RowCommand::Properties) => app.open_properties(),
        None => {}
    }
    if changed {
        app.dirty = true;
    }
}

/// Takes the clipboard events off the queue; egui turns Ctrl+X, Ctrl+C and Ctrl+V into these.
fn clipboard_events(ctx: &egui::Context) -> (bool, bool, Option<String>) {
    let mut cut = false;
    let mut copy = false;
    let mut paste = None;
    ctx.input_mut(|i| {
        i.events.retain(|event| match event {
            egui::Event::Cut => {
                cut = true;
                false
            }
            egui::Event::Copy => {
                copy = true;
                false
            }
            egui::Event::Paste(text) => {
                paste = Some(text.clone());
                false
            }
            _ => true,
        });
    });
    (cut, copy, paste)
}

/// What a row's context menu asked for, applied once the list is drawn.
#[derive(Clone, Copy)]
enum RowCommand {
    Cut,
    Copy,
    Paste,
    Duplicate,
    Delete,
    Properties,
}

fn header(ui: &mut egui::Ui) {
    let spacing = ui.spacing().item_spacing.x;
    let value_w = value_width(ui.available_width(), spacing);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        let columns = [
            (W_DRAG, ""),
            (W_ENABLED, ""),
            (W_INDEX, ""),
            (W_ACTION, "Action"),
            (value_w, "Value"),
            (W_COMMENT, "Comment"),
        ];
        for (width, text) in columns {
            cell(ui, width, 18.0, |ui| {
                if !text.is_empty() {
                    ui.label(
                        RichText::new(text).font(style::medium(12.0)).color(ui.visuals().weak_text_color()),
                    );
                }
            });
        }
    });
    ui.add_space(2.0);
    ui.separator();
}

fn comment_edit_id(id: ActionId) -> egui::Id {
    egui::Id::new(("comment_edit", id))
}

fn cell(ui: &mut egui::Ui, width: f32, height: f32, contents: impl FnOnce(&mut egui::Ui)) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let mut child =
        ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)));
    child.spacing_mut().item_spacing.x = 6.0;
    contents(&mut child);
}

fn label(ui: &mut egui::Ui, text: String, dim: bool, medium: bool) {
    let mut rich = RichText::new(text);
    if medium {
        rich = rich.font(style::medium(13.5));
    }
    if dim {
        rich = rich.color(ui.visuals().weak_text_color());
    }
    ui.add(egui::Label::new(rich).truncate().selectable(false));
}

/// Row icon for an action kind.
pub fn icon_for(action: &Action) -> MaterialIcon {
    match action {
        Action::Wait { .. } => icons::ICON_SCHEDULE,
        Action::KeyDown { .. } | Action::KeyUp { .. } | Action::KeyPress { .. } => icons::ICON_KEYBOARD,
        Action::TypeText { .. } => icons::ICON_TEXT_FIELDS,
        Action::MouseMove { .. } => icons::ICON_OPEN_WITH,
        Action::MouseMoveRelative { .. } => icons::ICON_SPORTS_ESPORTS,
        Action::MouseButton { .. } => icons::ICON_ADS_CLICK,
        Action::MouseWheel { .. } => icons::ICON_MOUSE,
        Action::WindowActivate { .. } => icons::ICON_WINDOW,
        Action::WaitForImage { .. } => icons::ICON_IMAGE,
        Action::WaitForText { .. } => icons::ICON_FIND_IN_PAGE,
        Action::ClickOnText { .. } => icons::ICON_TOUCH_APP,
        Action::WaitForFile { .. } => icons::ICON_INSERT_DRIVE_FILE,
        Action::Comment { .. } => icons::ICON_COMMENT,
        Action::Label { .. } => icons::ICON_LABEL,
    }
}

fn shortcuts(app: &mut App, ctx: &egui::Context) {
    if app.keyboard_busy(ctx) {
        return;
    }
    use egui::{Key, Modifiers};
    let keys = ctx.input_mut(|i| {
        [
            i.consume_key(Modifiers::COMMAND, Key::ArrowUp),
            i.consume_key(Modifiers::COMMAND, Key::ArrowDown),
            i.consume_key(Modifiers::COMMAND, Key::D),
            i.consume_key(Modifiers::COMMAND, Key::A),
            i.consume_key(Modifiers::NONE, Key::Delete),
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::SHIFT, Key::ArrowUp),
            i.consume_key(Modifiers::SHIFT, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
        ]
    });
    let [
        move_up,
        move_down,
        duplicate,
        select_all,
        delete,
        enter,
        extend_up,
        extend_down,
        select_up,
        select_down,
    ] = keys;
    let (cut, copy, paste) = clipboard_events(ctx);

    if move_up {
        app.move_selected(-1);
    }
    if move_down {
        app.move_selected(1);
    }
    if duplicate {
        app.duplicate_selected();
    }
    if select_all {
        app.select_all();
    }
    if copy {
        app.copy_selected(ctx);
    }
    if cut {
        app.cut_selected(ctx);
    }
    if let Some(text) = paste {
        app.paste_text(&text);
    }
    if delete {
        app.delete_selected();
    }
    if enter {
        app.open_properties();
    }
    if extend_up {
        app.extend_by(-1);
    }
    if extend_down {
        app.extend_by(1);
    }
    if select_up {
        app.step_selection(-1);
    }
    if select_down {
        app.step_selection(1);
    }
}
