use anyhow::{Context, Result};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};

use crate::model::{OverlayScene, OverlayShape, Point, Rect};

/// Padding around the scene bounds so strokes, arrow heads and anti-aliasing are never clipped.
pub const MARGIN: i32 = 24;

const GLYPH_W: usize = 5;
const GLYPH_H: usize = 7;
const GLYPH_SCALE: f32 = 2.0;
const LABEL_PAD_X: i32 = 8;
const LABEL_PAD_Y: i32 = 5;

const TEXT_COLOR: [u8; 4] = [12, 12, 16, 255];

/// Scene bounds plus [`MARGIN`], widened to fit label pills and clamped to the virtual screen.
pub fn window_rect(scene: &OverlayScene, screen: Rect) -> Option<Rect> {
    let bounds = scene.bounds()?;
    let mut left = bounds.x - MARGIN;
    let mut top = bounds.y - MARGIN;
    let mut right = bounds.right() + MARGIN;
    let mut bottom = bounds.bottom() + MARGIN;
    for shape in &scene.shapes {
        if let OverlayShape::Label { at, text, .. } = shape {
            let pill = label_rect(*at, text);
            left = left.min(pill.x - MARGIN);
            top = top.min(pill.y - MARGIN);
            right = right.max(pill.right() + MARGIN);
            bottom = bottom.max(pill.bottom() + MARGIN);
        }
    }
    let left = left.max(screen.x);
    let top = top.max(screen.y);
    let right = right.min(screen.right());
    let bottom = bottom.min(screen.bottom());
    (right > left && bottom > top).then(|| Rect::new(left, top, right - left, bottom - top))
}

pub fn render(scene: &OverlayScene, rect: Rect) -> Result<Pixmap> {
    let mut pixmap = Pixmap::new(rect.w as u32, rect.h as u32)
        .with_context(|| format!("overlay pixmap {}x{} could not be allocated", rect.w, rect.h))?;
    let transform = Transform::from_translate(-rect.x as f32, -rect.y as f32);
    for shape in &scene.shapes {
        draw_shape(&mut pixmap, shape, transform);
    }
    Ok(pixmap)
}

fn draw_shape(pixmap: &mut Pixmap, shape: &OverlayShape, transform: Transform) {
    match shape {
        OverlayShape::Polyline { points, color, width } => {
            if let Some(path) = polyline_path(points) {
                stroke(pixmap, &path, *color, *width, transform);
            }
            if let Some(head) = arrow_head(points, *width) {
                fill(pixmap, &head, *color, transform);
            }
        }
        OverlayShape::Circle { center, radius, color, filled } => {
            if let Some(path) = PathBuilder::from_circle(px(center.x), px(center.y), radius.max(0.5)) {
                if *filled {
                    fill(pixmap, &path, *color, transform);
                } else {
                    stroke(pixmap, &path, *color, 2.0, transform);
                }
            }
        }
        OverlayShape::Crosshair { center, size, color } => {
            draw_crosshair(pixmap, *center, *size, *color, transform);
        }
        OverlayShape::Rect { rect, color, width } => {
            if let Some(path) = rounded_rect(*rect, 4.0) {
                stroke(pixmap, &path, *color, *width, transform);
            }
        }
        OverlayShape::Label { at, text, color } => draw_label(pixmap, *at, text, *color, transform),
    }
}

/// Pixel centre of an integer screen coordinate, so thin strokes stay crisp.
fn px(value: i32) -> f32 {
    value as f32 + 0.5
}

fn paint(color: [u8; 4]) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color[0], color[1], color[2], color[3]);
    paint
}

fn stroke_of(width: f32) -> Stroke {
    Stroke {
        width: width.max(0.5),
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    }
}

/// Dark halo colour that keeps a shape readable on light and dark backgrounds.
fn shadow(color: [u8; 4]) -> [u8; 4] {
    [0, 0, 0, (color[3] as u16 * 200 / 255) as u8]
}

fn stroke(pixmap: &mut Pixmap, path: &Path, color: [u8; 4], width: f32, transform: Transform) {
    pixmap.stroke_path(path, &paint(shadow(color)), &stroke_of(width + 2.0), transform, None);
    pixmap.stroke_path(path, &paint(color), &stroke_of(width), transform, None);
}

fn fill(pixmap: &mut Pixmap, path: &Path, color: [u8; 4], transform: Transform) {
    pixmap.stroke_path(path, &paint(shadow(color)), &stroke_of(2.0), transform, None);
    pixmap.fill_path(path, &paint(color), FillRule::Winding, transform, None);
}

fn polyline_path(points: &[Point]) -> Option<Path> {
    let (first, rest) = points.split_first()?;
    let mut builder = PathBuilder::with_capacity(points.len(), points.len());
    builder.move_to(px(first.x), px(first.y));
    for point in rest {
        builder.line_to(px(point.x), px(point.y));
    }
    builder.finish()
}

/// Filled triangle pointing along the last segment that is long enough to give a direction.
fn arrow_head(points: &[Point], width: f32) -> Option<Path> {
    if points.len() < 2 {
        return None;
    }
    let tip = *points.last()?;
    let (dx, dy) = points.iter().rev().skip(1).find_map(|p| {
        let (dx, dy) = ((tip.x - p.x) as f32, (tip.y - p.y) as f32);
        let len = (dx * dx + dy * dy).sqrt();
        (len >= 1.0).then_some((dx / len, dy / len))
    })?;
    let length = (width * 4.0).max(12.0);
    let half = length * 0.45;
    let (bx, by) = (px(tip.x) - dx * length, px(tip.y) - dy * length);
    let mut builder = PathBuilder::with_capacity(4, 3);
    builder.move_to(px(tip.x), px(tip.y));
    builder.line_to(bx - dy * half, by + dx * half);
    builder.line_to(bx + dy * half, by - dx * half);
    builder.close();
    builder.finish()
}

fn draw_crosshair(pixmap: &mut Pixmap, center: Point, size: i32, color: [u8; 4], transform: Transform) {
    let size = size.max(6) as f32;
    let ring = size * 0.42;
    let gap = ring + 2.0;
    let (cx, cy) = (px(center.x), px(center.y));
    let mut builder = PathBuilder::with_capacity(8, 8);
    builder.move_to(cx - size, cy);
    builder.line_to(cx - gap, cy);
    builder.move_to(cx + gap, cy);
    builder.line_to(cx + size, cy);
    builder.move_to(cx, cy - size);
    builder.line_to(cx, cy - gap);
    builder.move_to(cx, cy + gap);
    builder.line_to(cx, cy + size);
    if let Some(path) = builder.finish() {
        stroke(pixmap, &path, color, 2.0, transform);
    }
    if let Some(path) = PathBuilder::from_circle(cx, cy, ring) {
        stroke(pixmap, &path, color, 1.5, transform);
    }
}

fn rounded_rect(rect: Rect, radius: f32) -> Option<Path> {
    let left = px(rect.x);
    let top = px(rect.y);
    let right = (rect.right() as f32 - 0.5).max(left);
    let bottom = (rect.bottom() as f32 - 0.5).max(top);
    let r = radius.min((right - left) / 3.0).min((bottom - top) / 3.0).max(0.0);
    let mut builder = PathBuilder::with_capacity(10, 12);
    builder.move_to(left + r, top);
    builder.line_to(right - r, top);
    builder.quad_to(right, top, right, top + r);
    builder.line_to(right, bottom - r);
    builder.quad_to(right, bottom, right - r, bottom);
    builder.line_to(left + r, bottom);
    builder.quad_to(left, bottom, left, bottom - r);
    builder.line_to(left, top + r);
    builder.quad_to(left, top, left + r, top);
    builder.close();
    builder.finish()
}

fn text_size(text: &str) -> (i32, i32) {
    let scale = GLYPH_SCALE as i32;
    let glyphs = text.chars().count() as i32;
    let width = if glyphs == 0 { 0 } else { glyphs * (GLYPH_W as i32 + 1) * scale - scale };
    (width, GLYPH_H as i32 * scale)
}

/// Pill that carries the label text, anchored at the scene position.
fn label_rect(at: Point, text: &str) -> Rect {
    let (width, height) = text_size(text);
    Rect::new(at.x, at.y, width + 2 * LABEL_PAD_X, height + 2 * LABEL_PAD_Y)
}

fn draw_label(pixmap: &mut Pixmap, at: Point, text: &str, color: [u8; 4], transform: Transform) {
    let pill = label_rect(at, text);
    if let Some(path) = rounded_rect(pill, pill.h as f32 * 0.45) {
        fill(pixmap, &path, color, transform);
    }
    let (width, height) = text_size(text);
    let x = pill.x + (pill.w - width) / 2;
    let y = pill.y + (pill.h - height) / 2;
    if let Some(path) = text_path(text, x, y) {
        pixmap.fill_path(&path, &paint(TEXT_COLOR), FillRule::Winding, transform, None);
    }
}

/// Turns text into one path of scaled pixel rectangles, merging each glyph row into runs.
fn text_path(text: &str, x: i32, y: i32) -> Option<Path> {
    let advance = (GLYPH_W as f32 + 1.0) * GLYPH_SCALE;
    let mut builder = PathBuilder::new();
    for (index, ch) in text.chars().enumerate() {
        let origin = x as f32 + index as f32 * advance;
        for (row, bits) in glyph(ch).iter().enumerate() {
            let mut col = 0;
            while col < GLYPH_W {
                if bits & bit(col) == 0 {
                    col += 1;
                    continue;
                }
                let start = col;
                while col < GLYPH_W && bits & bit(col) != 0 {
                    col += 1;
                }
                let cell = tiny_skia::Rect::from_xywh(
                    origin + start as f32 * GLYPH_SCALE,
                    y as f32 + row as f32 * GLYPH_SCALE,
                    (col - start) as f32 * GLYPH_SCALE,
                    GLYPH_SCALE,
                );
                if let Some(cell) = cell {
                    builder.push_rect(cell);
                }
            }
        }
    }
    builder.finish()
}

fn bit(col: usize) -> u8 {
    1 << (GLYPH_W - 1 - col)
}

fn glyph(ch: char) -> [u8; GLYPH_H] {
    let upper = ch.to_ascii_uppercase();
    FONT.iter().find(|(c, _)| *c == upper).map(|(_, rows)| *rows).unwrap_or(UNKNOWN_GLYPH)
}

const UNKNOWN_GLYPH: [u8; GLYPH_H] = [0b11111, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11111];

/// Uppercase letters, digits and a little punctuation in a 5x7 cell, one byte per row.
const FONT: &[(char, [u8; GLYPH_H])] = &[
    (' ', [0, 0, 0, 0, 0, 0, 0]),
    ('A', [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001]),
    ('B', [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110]),
    ('C', [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110]),
    ('D', [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110]),
    ('E', [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111]),
    ('F', [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000]),
    ('G', [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110]),
    ('H', [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001]),
    ('I', [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111]),
    ('J', [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100]),
    ('K', [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001]),
    ('L', [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111]),
    ('M', [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001]),
    ('N', [0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001]),
    ('O', [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110]),
    ('P', [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000]),
    ('Q', [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10011, 0b01101]),
    ('R', [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001]),
    ('S', [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110]),
    ('T', [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100]),
    ('U', [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110]),
    ('V', [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100]),
    ('W', [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001]),
    ('X', [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001]),
    ('Y', [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100]),
    ('Z', [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111]),
    ('0', [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110]),
    ('1', [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110]),
    ('2', [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111]),
    ('3', [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110]),
    ('4', [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010]),
    ('5', [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110]),
    ('6', [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110]),
    ('7', [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000]),
    ('8', [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110]),
    ('9', [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100]),
    ('.', [0, 0, 0, 0, 0, 0b01100, 0b01100]),
    (',', [0, 0, 0, 0, 0b01100, 0b00100, 0b01000]),
    (':', [0, 0b01100, 0b01100, 0, 0b01100, 0b01100, 0]),
    ('-', [0, 0, 0, 0b01110, 0, 0, 0]),
    ('+', [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0]),
    ('=', [0, 0, 0b11111, 0, 0b11111, 0, 0]),
    ('/', [0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000]),
    ('(', [0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010]),
    (')', [0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000]),
    ('!', [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0, 0b00100]),
    ('?', [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0, 0b00100]),
    ('%', [0b11000, 0b11001, 0b00010, 0b00100, 0b01000, 0b10011, 0b00011]),
];

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect::new(0, 0, 1920, 1080);

    fn scene(shapes: Vec<OverlayShape>) -> OverlayScene {
        OverlayScene { shapes }
    }

    #[test]
    fn window_rect_adds_the_margin_and_clamps_to_the_screen() {
        let far = scene(vec![OverlayShape::Crosshair {
            center: Point::new(500, 400),
            size: 10,
            color: [255, 0, 0, 255],
        }]);
        assert_eq!(window_rect(&far, SCREEN), Some(Rect::new(466, 366, 68, 68)));

        let corner = scene(vec![OverlayShape::Circle {
            center: Point::new(2, 2),
            radius: 4.0,
            color: [255, 0, 0, 255],
            filled: true,
        }]);
        assert_eq!(window_rect(&corner, SCREEN), Some(Rect::new(0, 0, 30, 30)));

        assert_eq!(window_rect(&OverlayScene::default(), SCREEN), None);
    }

    #[test]
    fn window_rect_covers_the_whole_label_pill() {
        let text = "right click";
        let label = scene(vec![OverlayShape::Label {
            at: Point::new(400, 300),
            text: text.into(),
            color: [255, 0, 0, 220],
        }]);
        let rect = window_rect(&label, SCREEN).expect("a label has bounds");
        let pill = label_rect(Point::new(400, 300), text);
        assert!(rect.contains(Point::new(pill.right(), pill.bottom())), "{rect:?} vs {pill:?}");
        assert!(rect.right() >= pill.right() + MARGIN);
    }

    #[test]
    fn rendering_paints_inside_the_window() {
        let center = Point::new(600, 500);
        let scene = scene(vec![
            OverlayShape::Polyline {
                points: vec![Point::new(560, 460), Point::new(580, 480), center],
                color: [96, 165, 250, 220],
                width: 2.0,
            },
            OverlayShape::Crosshair { center, size: 16, color: [239, 68, 68, 220] },
            OverlayShape::Label {
                at: Point::new(center.x + 20, center.y + 8),
                text: "left click".into(),
                color: [239, 68, 68, 220],
            },
        ]);
        let rect = window_rect(&scene, SCREEN).expect("bounds");
        let pixmap = render(&scene, rect).expect("render");
        let alpha = |p: Point| {
            let index = ((p.y - rect.y) as usize * rect.w as usize + (p.x - rect.x) as usize) * 4 + 3;
            pixmap.data()[index]
        };
        assert!(alpha(Point::new(center.x + 12, center.y)) > 0, "crosshair arm is missing");
        assert_eq!(alpha(Point::new(rect.x + 1, rect.y + 1)), 0, "the margin must stay transparent");
        assert!(pixmap.data().iter().skip(3).step_by(4).filter(|a| **a > 0).count() > 500);
    }

    #[test]
    fn the_font_covers_every_label_the_gui_produces() {
        for ch in "left right middle x1 x2 down up click 0123456789".chars() {
            assert_ne!(glyph(ch), UNKNOWN_GLYPH, "no glyph for {ch:?}");
        }
        assert_eq!(glyph('~'), UNKNOWN_GLYPH);
        assert!(text_path("AB", 0, 0).is_some());
        assert!(text_path("", 0, 0).is_none());
    }
}
