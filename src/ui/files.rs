use std::path::{Path, PathBuf};

use egui::{Button, Key, KeyboardShortcut, Modifiers, Vec2};

use super::App;
use crate::model::Macro;

/// An action that waits for the user to decide what to do with unsaved changes.
#[derive(Clone, Debug)]
pub enum Pending {
    New,
    Open,
    OpenPath(PathBuf),
    Close,
}

pub fn new_doc(app: &mut App) {
    if app.dirty {
        app.confirm = Some(Pending::New);
    } else {
        reset(app);
    }
}

pub fn open(app: &mut App) {
    if app.dirty {
        app.confirm = Some(Pending::Open);
    } else {
        pick_and_open(app);
    }
}

pub fn open_path(app: &mut App, path: PathBuf) {
    if app.dirty {
        app.confirm = Some(Pending::OpenPath(path));
    } else {
        load(app, &path);
    }
}

pub fn save(app: &mut App) -> bool {
    match app.path.clone() {
        Some(path) => write(app, &path),
        None => save_as(app),
    }
}

pub fn save_as(app: &mut App) -> bool {
    let suggested = app
        .path
        .as_ref()
        .and_then(|p| p.file_name())
        .map_or_else(|| "macro.json".to_string(), |n| n.to_string_lossy().to_string());
    let Some(path) = dialog(app).set_file_name(suggested).save_file() else {
        return false;
    };
    write(app, &path)
}

/// Ctrl+N, Ctrl+O, Ctrl+S and Ctrl+Shift+S.
pub fn shortcuts(app: &mut App, ctx: &egui::Context) {
    if app.dialog.is_some() || app.confirm.is_some() {
        return;
    }
    let hits = ctx.input_mut(|i| {
        [
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::S)),
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::S)),
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::N)),
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::O)),
        ]
    });
    let [save_as_hit, save_hit, new_hit, open_hit] = hits;
    if save_as_hit {
        save_as(app);
    } else if save_hit {
        save(app);
    }
    if new_hit {
        new_doc(app);
    }
    if open_hit {
        open(app);
    }
}

/// Intercepts a close request while the macro has unsaved changes.
pub fn confirm_close(app: &mut App, ctx: &egui::Context) {
    if app.closing || !ctx.input(|i| i.viewport().close_requested()) {
        return;
    }
    if !app.dirty {
        app.closing = true;
        return;
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
    if app.confirm.is_none() {
        app.confirm = Some(Pending::Close);
    }
}

/// Modal asking what to do with unsaved changes before a destructive action.
pub fn show_confirm(app: &mut App, ctx: &egui::Context) {
    let Some(pending) = app.confirm.take() else {
        return;
    };
    let mut proceed = false;
    let mut cancel = false;
    let mut save_first = false;

    let closing = matches!(pending, Pending::Close);
    let response = egui::Modal::new(egui::Id::new("confirm_discard")).show(ctx, |ui| {
        ui.set_min_width(340.0);
        ui.heading("Unsaved changes");
        ui.add_space(6.0);
        ui.label(if closing {
            "This macro has changes that are not saved yet. Close anyway?"
        } else {
            "This macro has changes that are not saved yet."
        });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.add(Button::new("Save").min_size(Vec2::new(84.0, 0.0))).clicked() {
                save_first = true;
            }
            let discard = if closing { "Close" } else { "Discard" };
            if ui.add(Button::new(discard).min_size(Vec2::new(84.0, 0.0))).clicked() {
                proceed = true;
            }
            if ui.add(Button::new("Cancel").min_size(Vec2::new(84.0, 0.0))).clicked() {
                cancel = true;
            }
        });
    });
    if response.should_close() {
        cancel = true;
    }
    if save_first {
        if save(app) {
            proceed = true;
        } else {
            app.confirm = Some(pending.clone());
            return;
        }
    }
    if proceed {
        match pending {
            Pending::New => reset(app),
            Pending::Open => pick_and_open(app),
            Pending::OpenPath(path) => load(app, &path),
            Pending::Close => {
                app.closing = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    } else if !cancel {
        app.confirm = Some(pending);
    }
}

fn reset(app: &mut App) {
    app.doc = Macro::default();
    app.path = None;
    app.dirty = false;
    app.select(None);
    app.info("New macro");
}

fn pick_and_open(app: &mut App) {
    if let Some(path) = dialog(app).pick_file() {
        load(app, &path);
    }
}

fn load(app: &mut App, path: &Path) {
    match Macro::load(path) {
        Ok(doc) => {
            let count = doc.items.len();
            app.doc = doc;
            app.path = Some(path.to_path_buf());
            app.dirty = false;
            app.select(None);
            remember(app, path);
            app.info(format!("Opened {count} actions from {}", path.display()));
        }
        Err(e) => app.error(format!("Cannot open {}: {e:#}", path.display())),
    }
}

fn write(app: &mut App, path: &Path) -> bool {
    if app.doc.name.is_empty() {
        app.doc.name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    }
    match app.doc.save(path) {
        Ok(()) => {
            app.path = Some(path.to_path_buf());
            app.dirty = false;
            remember(app, path);
            app.info(format!("Saved {}", path.display()));
            true
        }
        Err(e) => {
            app.error(format!("Cannot save {}: {e:#}", path.display()));
            false
        }
    }
}

fn remember(app: &mut App, path: &Path) {
    app.settings.push_recent(path.to_path_buf());
    if let Err(e) = app.settings.save_default() {
        log::warn!("cannot save settings: {e:#}");
    }
}

/// Folder macro files live in by default, `Documents/Parrot`.
pub fn default_dir_path() -> Option<PathBuf> {
    dirs::document_dir().map(|d| d.join("Parrot"))
}

/// The default folder, created if it does not exist yet.
pub fn default_dir() -> Option<PathBuf> {
    let dir = default_dir_path()?;
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("cannot create {}: {e:#}", dir.display());
        return None;
    }
    Some(dir)
}

/// Where the file dialogs open: the current file's folder, else the default one.
fn start_dir(app: &App) -> Option<PathBuf> {
    app.path
        .as_ref()
        .and_then(|p| p.parent())
        .filter(|d| d.is_dir())
        .map(Path::to_path_buf)
        .or_else(default_dir)
}

fn dialog(app: &App) -> rfd::FileDialog {
    let mut dialog = rfd::FileDialog::new().add_filter("Macro", &["json"]).set_title("Macro file");
    if let Some(dir) = start_dir(app) {
        dialog = dialog.set_directory(dir);
    }
    dialog
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dir_sits_in_documents() {
        let Some(dir) = default_dir_path() else { return };
        assert_eq!(dir.file_name().unwrap(), "Parrot");
        assert_eq!(dir.parent(), dirs::document_dir().as_deref());
    }
}
