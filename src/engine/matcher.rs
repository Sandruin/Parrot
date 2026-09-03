use image::RgbaImage;

use crate::model::{ImageMatchMode, Point};

const EPS: f64 = 1e-6;
const PYRAMID_FACTOR: usize = 4;
const PYRAMID_MIN_TEMPLATE: usize = 16;
const REFINE_RADIUS: usize = 6;
const MAX_CANDIDATES: usize = 8;

/// Pixel-by-pixel similarity over RGB as `1 - mean(|a - b|) / 255`, or 0 when the sizes differ.
pub fn similarity_exact(a: &RgbaImage, b: &RgbaImage) -> f32 {
    if a.dimensions() != b.dimensions() {
        return 0.0;
    }
    let channels = a.width() as u64 * a.height() as u64 * 3;
    if channels == 0 {
        return 1.0;
    }
    let mut diff = 0u64;
    for (pa, pb) in a.pixels().zip(b.pixels()) {
        diff += pa.0[0].abs_diff(pb.0[0]) as u64;
        diff += pa.0[1].abs_diff(pb.0[1]) as u64;
        diff += pa.0[2].abs_diff(pb.0[2]) as u64;
    }
    (1.0 - diff as f64 / (channels as f64 * 255.0)) as f32
}

/// Best normalized cross-correlation position of `template` inside `haystack`,
/// or `None` when the template does not fit or carries no contrast.
pub fn search(haystack: &RgbaImage, template: &RgbaImage) -> Option<(Point, f32)> {
    let hay = Plane::from_rgba(haystack);
    let tpl = Plane::from_rgba(template);
    search_pyramid(&hay, &tpl)
}

/// Runs the mode's comparison and reports whether the score reaches `threshold`.
pub fn evaluate(
    mode: ImageMatchMode,
    captured: &RgbaImage,
    template: &RgbaImage,
    threshold: f32,
) -> (bool, f32) {
    let score = match mode {
        ImageMatchMode::Exact => similarity_exact(captured, template),
        ImageMatchMode::Search => search(captured, template).map_or(0.0, |(_, s)| s),
    };
    (score >= threshold, score)
}

/// Grayscale image as plain floats, the only form the correlation works on.
struct Plane {
    w: usize,
    h: usize,
    px: Vec<f64>,
}

impl Plane {
    fn from_rgba(img: &RgbaImage) -> Self {
        let px = img
            .pixels()
            .map(|p| 0.299 * p.0[0] as f64 + 0.587 * p.0[1] as f64 + 0.114 * p.0[2] as f64)
            .collect();
        Self { w: img.width() as usize, h: img.height() as usize, px }
    }

    fn downsample(&self, factor: usize) -> Plane {
        let w = self.w / factor;
        let h = self.h / factor;
        let mut px = Vec::with_capacity(w * h);
        let area = (factor * factor) as f64;
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0.0;
                for dy in 0..factor {
                    let row = (y * factor + dy) * self.w;
                    for dx in 0..factor {
                        sum += self.px[row + x * factor + dx];
                    }
                }
                px.push(sum / area);
            }
        }
        Plane { w, h, px }
    }
}

struct Stats {
    n: f64,
    sum: f64,
    var: f64,
}

fn stats(p: &Plane) -> Stats {
    let n = p.px.len() as f64;
    let sum: f64 = p.px.iter().sum();
    let sum_sq: f64 = p.px.iter().map(|v| v * v).sum();
    Stats { n, sum, var: (sum_sq - sum * sum / n).max(0.0) }
}

fn ncc_at(hay: &Plane, tpl: &Plane, st: &Stats, x: usize, y: usize) -> f64 {
    let mut sum_h = 0.0;
    let mut sum_hh = 0.0;
    let mut sum_ht = 0.0;
    for ty in 0..tpl.h {
        let hrow = (y + ty) * hay.w + x;
        let trow = ty * tpl.w;
        for tx in 0..tpl.w {
            let hv = hay.px[hrow + tx];
            let tv = tpl.px[trow + tx];
            sum_h += hv;
            sum_hh += hv * hv;
            sum_ht += hv * tv;
        }
    }
    let var_h = sum_hh - sum_h * sum_h / st.n;
    if var_h <= EPS {
        return 0.0;
    }
    (sum_ht - sum_h * st.sum / st.n) / (var_h * st.var).sqrt()
}

fn best_in_window(
    hay: &Plane,
    tpl: &Plane,
    st: &Stats,
    xs: (usize, usize),
    ys: (usize, usize),
) -> Option<(usize, usize, f64)> {
    let mut best: Option<(usize, usize, f64)> = None;
    for y in ys.0..=ys.1 {
        for x in xs.0..=xs.1 {
            let score = ncc_at(hay, tpl, st, x, y);
            if best.is_none_or(|(_, _, b)| score > b) {
                best = Some((x, y, score));
            }
        }
    }
    best
}

/// Best positions with a minimum separation, so several distinct peaks survive.
fn top_candidates(hay: &Plane, tpl: &Plane, st: &Stats, limit: usize) -> Vec<(usize, usize, f64)> {
    let mut scored = Vec::with_capacity((hay.w - tpl.w + 1) * (hay.h - tpl.h + 1));
    for y in 0..=hay.h - tpl.h {
        for x in 0..=hay.w - tpl.w {
            scored.push((x, y, ncc_at(hay, tpl, st, x, y)));
        }
    }
    scored.sort_by(|a, b| b.2.total_cmp(&a.2));
    let separation = (tpl.w.min(tpl.h) / 2).max(1) as i64;
    let mut picked: Vec<(usize, usize, f64)> = Vec::with_capacity(limit);
    for cand in scored {
        if picked.len() == limit {
            break;
        }
        let far_enough = picked.iter().all(|p| {
            (p.0 as i64 - cand.0 as i64).abs() >= separation
                || (p.1 as i64 - cand.1 as i64).abs() >= separation
        });
        if far_enough {
            picked.push(cand);
        }
    }
    picked
}

fn search_full(hay: &Plane, tpl: &Plane) -> Option<(Point, f32)> {
    let st = stats(tpl);
    if !fits(hay, tpl) || st.var <= EPS {
        return None;
    }
    best_in_window(hay, tpl, &st, (0, hay.w - tpl.w), (0, hay.h - tpl.h)).map(as_result)
}

fn search_pyramid(hay: &Plane, tpl: &Plane) -> Option<(Point, f32)> {
    let st = stats(tpl);
    if !fits(hay, tpl) || st.var <= EPS {
        return None;
    }
    if tpl.w >= PYRAMID_MIN_TEMPLATE && tpl.h >= PYRAMID_MIN_TEMPLATE {
        let coarse_hay = hay.downsample(PYRAMID_FACTOR);
        let coarse_tpl = tpl.downsample(PYRAMID_FACTOR);
        let coarse_st = stats(&coarse_tpl);
        if fits(&coarse_hay, &coarse_tpl) && coarse_st.var > EPS {
            let candidates = top_candidates(&coarse_hay, &coarse_tpl, &coarse_st, MAX_CANDIDATES);
            let mut best: Option<(usize, usize, f64)> = None;
            for (cx, cy, _) in candidates {
                let window = refine_window(hay, tpl, cx, cy);
                if let Some(found) = best_in_window(hay, tpl, &st, window.0, window.1)
                    && best.is_none_or(|(_, _, b)| found.2 > b)
                {
                    best = Some(found);
                }
            }
            if let Some(found) = best {
                return Some(as_result(found));
            }
        }
    }
    best_in_window(hay, tpl, &st, (0, hay.w - tpl.w), (0, hay.h - tpl.h)).map(as_result)
}

fn refine_window(hay: &Plane, tpl: &Plane, cx: usize, cy: usize) -> ((usize, usize), (usize, usize)) {
    let max_x = hay.w - tpl.w;
    let max_y = hay.h - tpl.h;
    let x = (cx * PYRAMID_FACTOR).min(max_x);
    let y = (cy * PYRAMID_FACTOR).min(max_y);
    (
        (x.saturating_sub(REFINE_RADIUS), (x + REFINE_RADIUS).min(max_x)),
        (y.saturating_sub(REFINE_RADIUS), (y + REFINE_RADIUS).min(max_y)),
    )
}

fn fits(hay: &Plane, tpl: &Plane) -> bool {
    tpl.w > 0 && tpl.h > 0 && hay.w >= tpl.w && hay.h >= tpl.h
}

fn as_result((x, y, score): (usize, usize, f64)) -> (Point, f32) {
    (Point::new(x as i32, y as i32), score.clamp(0.0, 1.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// Deterministic pseudo random image so tests do not depend on a random crate.
    struct Lcg(u64);

    impl Lcg {
        fn next_u8(&mut self) -> u8 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) as u8
        }
    }

    fn noisy(w: u32, h: u32, seed: u64) -> RgbaImage {
        let mut rng = Lcg(seed);
        RgbaImage::from_fn(w, h, |_, _| Rgba([rng.next_u8(), rng.next_u8(), rng.next_u8(), 255]))
    }

    fn flat(w: u32, h: u32, v: u8) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([v, v, v, 255]))
    }

    fn crop(img: &RgbaImage, x: u32, y: u32, w: u32, h: u32) -> RgbaImage {
        image::imageops::crop_imm(img, x, y, w, h).to_image()
    }

    fn add_noise(img: &RgbaImage, amount: i32, seed: u64) -> RgbaImage {
        let mut rng = Lcg(seed);
        RgbaImage::from_fn(img.width(), img.height(), |x, y| {
            let p = img.get_pixel(x, y).0;
            let jitter = |c: u8, r: u8| {
                let delta = (r as i32 % (2 * amount + 1)) - amount;
                (c as i32 + delta).clamp(0, 255) as u8
            };
            Rgba([jitter(p[0], rng.next_u8()), jitter(p[1], rng.next_u8()), jitter(p[2], rng.next_u8()), 255])
        })
    }

    #[test]
    fn identical_images_score_one() {
        let img = noisy(20, 20, 7);
        assert_eq!(similarity_exact(&img, &img), 1.0);
        let (matched, score) = evaluate(ImageMatchMode::Exact, &img, &img, 1.0);
        assert!(matched);
        assert_eq!(score, 1.0);
    }

    #[test]
    fn mismatched_sizes_score_zero() {
        assert_eq!(similarity_exact(&noisy(20, 20, 1), &noisy(10, 10, 1)), 0.0);
    }

    #[test]
    fn exact_similarity_is_mean_absolute_difference() {
        let a = flat(4, 4, 100);
        let b = flat(4, 4, 110);
        let s = similarity_exact(&a, &b);
        assert!((s - (1.0 - 10.0 / 255.0)).abs() < 1e-6, "{s}");
    }

    #[test]
    fn shifted_template_is_found_at_the_right_offset() {
        let hay = noisy(64, 48, 42);
        let tpl = crop(&hay, 13, 7, 20, 12);
        let (pos, score) = search(&hay, &tpl).unwrap();
        assert_eq!(pos, Point::new(13, 7));
        assert!(score > 0.99, "{score}");
    }

    #[test]
    fn noise_lowers_the_score_monotonically() {
        let hay = noisy(48, 48, 5);
        let clean = crop(&hay, 8, 8, 20, 20);
        let mut previous = 1.01f32;
        for amount in [0, 10, 30, 60, 100] {
            let tpl = add_noise(&clean, amount.max(1), 99 + amount as u64);
            let score =
                if amount == 0 { search(&hay, &clean).unwrap().1 } else { search(&hay, &tpl).unwrap().1 };
            assert!(score < previous, "amount {amount}: {score} >= {previous}");
            previous = score;
        }
    }

    #[test]
    fn flat_template_yields_none_instead_of_nan() {
        let hay = noisy(40, 40, 3);
        let tpl = flat(20, 20, 128);
        assert!(search(&hay, &tpl).is_none());
        let (matched, score) = evaluate(ImageMatchMode::Search, &hay, &tpl, 0.9);
        assert!(!matched);
        assert!(score.is_finite());
        assert_eq!(score, 0.0);
        assert!(search(&flat(40, 40, 7), &flat(20, 20, 7)).is_none());
    }

    #[test]
    fn template_larger_than_haystack_yields_none() {
        assert!(search(&noisy(10, 10, 2), &noisy(20, 20, 2)).is_none());
    }

    #[test]
    fn pyramid_matches_brute_force() {
        let hay = noisy(40, 36, 11);
        let tpl = crop(&hay, 17, 9, 16, 16);
        let pyramid = search(&hay, &tpl).unwrap();
        let brute = search_full(&Plane::from_rgba(&hay), &Plane::from_rgba(&tpl)).unwrap();
        assert_eq!(pyramid.0, brute.0);
        assert!((pyramid.1 - brute.1).abs() < 1e-6);
        assert_eq!(pyramid.0, Point::new(17, 9));
    }

    #[test]
    fn pyramid_matches_brute_force_on_a_noisy_template() {
        let hay = noisy(64, 64, 21);
        let tpl = add_noise(&crop(&hay, 30, 24, 18, 18), 12, 5);
        let pyramid = search(&hay, &tpl).unwrap();
        let brute = search_full(&Plane::from_rgba(&hay), &Plane::from_rgba(&tpl)).unwrap();
        assert_eq!(pyramid.0, brute.0);
        assert!((pyramid.1 - brute.1).abs() < 1e-6);
    }
}
