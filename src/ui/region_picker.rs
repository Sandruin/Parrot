use std::sync::Arc;

use anyhow::{Context as _, Result};
use egui::{Color32, FontId, Pos2, Sense, Stroke, StrokeKind, ViewportBuilder, ViewportId};
use image::RgbaImage;

use super::{App, UiServices, style};
use crate::model::{ActionId, Rect};

/// Alpha of the dimming veil over the parts of the screenshot outside the selection.
const DIM_ALPHA: u8 = 102;
/// Smallest selection in physical pixels that counts as a real drag.
const MIN_DRAG: i32 = 4;

/// Frozen screenshot of one monitor that the user drags a `WaitForImage` region on.
pub struct Picker {
    /// Action whose open properties dialog receives the result.
    target: ActionId,
    /// The picked monitor in physical virtual-screen pixels.
    monitor: Rect,
    shot: Arc<RgbaImage>,
    texture: Option<egui::TextureHandle>,
    /// Drag corners in viewport points, `None` until the first drag.
    anchor: Option<Pos2>,
    cursor: Option<Pos2>,
}

enum Outcome {
    Picking,
    Confirmed(Rect),
    Cancelled,
}

/// Screenshots the monitor under the mouse and arms the picker for the given action.
pub fn open(app: &mut App, target: ActionId) {
    match capture_monitor(&app.services) {
        Ok((monitor, shot)) => {
            app.region_picker = Some(Picker {
                target,
                monitor,
                shot: Arc::new(shot),
                texture: None,
                anchor: None,
                cursor: None,
            });
            app.info("Drag a region, Enter confirms, Escape cancels");
        }
        Err(e) => app.error(format!("Could not capture the screen: {e:#}")),
    }
}

/// Shows the picker viewport while one is armed and writes the result back into the dialog.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut picker) = app.region_picker.take() else {
        return;
    };
    if picker.texture.is_none() {
        picker.texture = Some(load_texture(ctx, &picker.shot));
    }

    let points = ctx.pixels_per_point();
    let position = egui::pos2(picker.monitor.x as f32 / points, picker.monitor.y as f32 / points);
    let size = egui::vec2(picker.monitor.w as f32 / points, picker.monitor.h as f32 / points);
    let builder = ViewportBuilder::default()
        .with_title("Pick a region")
        .with_decorations(false)
        .with_resizable(false)
        .with_taskbar(false)
        .with_always_on_top()
        .with_position(position)
        .with_inner_size(size);

    let id = ViewportId::from_hash_of("region_picker");
    let outcome = ctx.show_viewport_immediate(id, builder, |ui, _class| picker_ui(ui, &mut picker));

    match outcome {
        Outcome::Picking => app.region_picker = Some(picker),
        Outcome::Cancelled => app.info("Region picking cancelled"),
        Outcome::Confirmed(region) => match encode_crop(&picker.shot, picker.monitor, region) {
            Ok(png) => {
                let applied =
                    app.dialog.as_mut().is_some_and(|dialog| dialog.apply_region(picker.target, region, png));
                if applied {
                    app.info(format!(
                        "Captured {} x {} px at {}, {}",
                        region.w, region.h, region.x, region.y
                    ));
                } else {
                    app.error("The properties dialog closed before the region arrived");
                }
            }
            Err(e) => app.error(format!("Could not encode the template: {e:#}")),
        },
    }
}

fn picker_ui(ui: &mut egui::Ui, picker: &mut Picker) -> Outcome {
    let full = ui.max_rect();
    let response = ui.interact(full, ui.id().with("region"), Sense::click_and_drag());
    if response.drag_started() {
        picker.anchor = ui.input(|i| i.pointer.press_origin()).or(response.interact_pointer_pos());
        picker.cursor = picker.anchor;
    }
    if response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
    {
        picker.cursor = Some(pos);
    }

    let accent = style::accent(ui.visuals());
    let texture = match &picker.texture {
        Some(texture) => texture.id(),
        None => return Outcome::Cancelled,
    };
    let uv_full = egui::Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0));
    let painter = ui.painter().clone();
    painter.image(texture, full, uv_full, Color32::WHITE);
    painter.rect_filled(full, 0, Color32::from_black_alpha(DIM_ALPHA));

    let points = ui.ctx().pixels_per_point();
    let selection = match (picker.anchor, picker.cursor) {
        (Some(a), Some(b)) => Some(egui::Rect::from_two_pos(a, b)),
        _ => None,
    };
    if let Some(selection) = selection.filter(|s| s.width() >= 1.0 && s.height() >= 1.0) {
        let uv = egui::Rect::from_min_max(uv_at(full, selection.min), uv_at(full, selection.max));
        painter.image(texture, selection, uv, Color32::WHITE);
        painter.rect_stroke(selection, 0, Stroke::new(1.0, accent), StrokeKind::Outside);
        let physical = to_physical(picker.monitor, full, selection, points);
        label(&painter, selection, format!("{} x {} px", physical.w, physical.h), accent);
    }
    hint(&painter, full);

    let (escape, enter) = ui.input(|i| (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::Enter)));
    let picked = selection.map(|s| to_physical(picker.monitor, full, s, points));
    let big_enough = picked.is_some_and(|r| r.w >= MIN_DRAG && r.h >= MIN_DRAG);

    if escape {
        return Outcome::Cancelled;
    }
    if response.drag_stopped() && !big_enough {
        picker.anchor = None;
        picker.cursor = None;
        return Outcome::Picking;
    }
    if (enter || response.drag_stopped()) && big_enough {
        return Outcome::Confirmed(picked.expect("a big enough selection has a rectangle"));
    }
    if enter {
        return Outcome::Cancelled;
    }
    Outcome::Picking
}

fn hint(painter: &egui::Painter, full: egui::Rect) {
    let text = "Drag to pick a region     Enter confirms     Escape cancels";
    let font = FontId::new(15.0, egui::FontFamily::Name(style::MEDIUM_FAMILY.into()));
    let at = egui::pos2(full.center().x, full.top() + 34.0);
    let galley = painter.layout_no_wrap(text.to_owned(), font, Color32::WHITE);
    let rect = egui::Align2::CENTER_CENTER.anchor_size(at, galley.size()).expand(10.0);
    painter.rect_filled(rect, 6, Color32::from_black_alpha(190));
    painter.galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
}

fn label(painter: &egui::Painter, selection: egui::Rect, text: String, accent: Color32) {
    let font = FontId::proportional(13.0);
    let galley = painter.layout_no_wrap(text, font, Color32::WHITE);
    let above = selection.top() - galley.size().y - 12.0;
    let at = if above > 0.0 {
        egui::pos2(selection.left(), above)
    } else {
        egui::pos2(selection.left(), selection.top() + 6.0)
    };
    let rect = egui::Rect::from_min_size(at, galley.size()).expand(5.0);
    painter.rect_filled(rect, 4, accent);
    painter.galley(at, galley, Color32::WHITE);
}

fn uv_at(full: egui::Rect, pos: Pos2) -> Pos2 {
    egui::pos2((pos.x - full.left()) / full.width(), (pos.y - full.top()) / full.height())
}

/// Maps a selection in viewport points onto physical virtual-screen pixels, clamped to the monitor.
fn to_physical(monitor: Rect, full: egui::Rect, selection: egui::Rect, points: f32) -> Rect {
    let left = ((selection.left() - full.left()) * points).round() as i32;
    let top = ((selection.top() - full.top()) * points).round() as i32;
    let right = ((selection.right() - full.left()) * points).round() as i32;
    let bottom = ((selection.bottom() - full.top()) * points).round() as i32;
    let left = left.clamp(0, monitor.w);
    let top = top.clamp(0, monitor.h);
    let right = right.clamp(left, monitor.w);
    let bottom = bottom.clamp(top, monitor.h);
    Rect::new(monitor.x + left, monitor.y + top, right - left, bottom - top)
}

fn load_texture(ctx: &egui::Context, shot: &RgbaImage) -> egui::TextureHandle {
    let size = [shot.width() as usize, shot.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, shot.as_raw());
    ctx.load_texture("region_picker_shot", color, egui::TextureOptions::LINEAR)
}

/// Cuts `region` out of the monitor screenshot and encodes it as PNG.
fn encode_crop(shot: &RgbaImage, monitor: Rect, region: Rect) -> Result<Vec<u8>> {
    use image::ImageEncoder as _;

    let x = (region.x - monitor.x).max(0) as u32;
    let y = (region.y - monitor.y).max(0) as u32;
    let w = (region.w.max(1) as u32).min(shot.width().saturating_sub(x));
    let h = (region.h.max(1) as u32).min(shot.height().saturating_sub(y));
    if w == 0 || h == 0 {
        anyhow::bail!("selection {region:?} lies outside the screenshot");
    }
    let crop = image::imageops::crop_imm(shot, x, y, w, h).to_image();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(crop.as_raw(), w, h, image::ExtendedColorType::Rgba8)
        .context("encoding the template as PNG")?;
    Ok(png)
}

fn capture_monitor(services: &UiServices) -> Result<(Rect, RgbaImage)> {
    let monitor = monitor_under_cursor(services);
    let shot = services.capture.capture(monitor).with_context(|| format!("capturing {monitor:?}"))?;
    Ok((monitor, shot))
}

/// Bounds of the monitor the mouse cursor sits on, or the whole virtual screen when none contains it.
fn monitor_under_cursor(services: &UiServices) -> Rect {
    let cursor = services.cursor_pos();
    let monitors = services.capture.monitors();
    monitors.iter().copied().find(|m| m.contains(cursor)).unwrap_or_else(|| services.capture.virtual_screen())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport(w: f32, h: f32) -> egui::Rect {
        egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(w, h))
    }

    #[test]
    fn points_map_back_onto_physical_monitor_pixels() {
        let monitor = Rect::new(-1920, 120, 1920, 1080);
        let full = viewport(960.0, 540.0);
        let selection = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(60.0, 70.0));
        let physical = to_physical(monitor, full, selection, 2.0);
        assert_eq!(physical, Rect::new(-1900, 160, 100, 100));
    }

    #[test]
    fn selections_are_clamped_to_the_monitor() {
        let monitor = Rect::new(0, 0, 800, 600);
        let full = viewport(800.0, 600.0);
        let selection = egui::Rect::from_min_max(egui::pos2(-50.0, -50.0), egui::pos2(900.0, 900.0));
        assert_eq!(to_physical(monitor, full, selection, 1.0), Rect::new(0, 0, 800, 600));
    }

    #[test]
    fn cropping_keeps_the_selected_pixels() {
        let mut shot = RgbaImage::new(20, 10);
        shot.put_pixel(5, 5, image::Rgba([1, 2, 3, 255]));
        let monitor = Rect::new(100, 200, 20, 10);
        let png = encode_crop(&shot, monitor, Rect::new(104, 204, 4, 4)).expect("encoding");
        let decoded = image::load_from_memory(&png).expect("decoding").to_rgba8();
        assert_eq!((decoded.width(), decoded.height()), (4, 4));
        assert_eq!(decoded.get_pixel(1, 1), &image::Rgba([1, 2, 3, 255]));
    }
}
