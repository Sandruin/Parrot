use crate::model::{Point, Rect};

/// One output as the compositor lays it out: logical position and size plus its size in pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Monitor {
    pub name: String,
    /// Position and size in the compositor's logical coordinate space.
    pub logical: Rect,
    /// Size in physical pixels after the output transform.
    pub width: i32,
    pub height: i32,
}

impl Monitor {
    /// Physical pixels per logical unit.
    pub fn scale(&self) -> f64 {
        if self.logical.w > 0 { self.width as f64 / self.logical.w as f64 } else { 1.0 }
    }
}

/// Maps between the compositor's logical space and the physical virtual screen the macros use.
/// Every monitor sits at its logical position times the largest scale, so pixel rectangles never overlap.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    monitors: Vec<Monitor>,
    factor: f64,
}

impl Layout {
    pub fn new(monitors: Vec<Monitor>) -> Self {
        let factor = monitors.iter().map(Monitor::scale).fold(1.0, f64::max);
        Self { monitors, factor }
    }

    pub fn monitors(&self) -> &[Monitor] {
        &self.monitors
    }

    pub fn is_empty(&self) -> bool {
        self.monitors.is_empty()
    }

    pub fn monitor_named(&self, name: &str) -> Option<&Monitor> {
        self.monitors.iter().find(|m| m.name == name)
    }

    /// Rectangle of one monitor in physical virtual-screen pixels.
    pub fn physical(&self, monitor: &Monitor) -> Rect {
        Rect::new(
            (monitor.logical.x as f64 * self.factor).round() as i32,
            (monitor.logical.y as f64 * self.factor).round() as i32,
            monitor.width,
            monitor.height,
        )
    }

    pub fn physical_rects(&self) -> Vec<Rect> {
        self.monitors.iter().map(|m| self.physical(m)).collect()
    }

    /// Bounding rectangle of all monitors in physical pixels, empty when no output is known.
    pub fn virtual_screen(&self) -> Rect {
        let mut rects = self.monitors.iter().map(|m| self.physical(m));
        let Some(first) = rects.next() else { return Rect::default() };
        rects.fold(first, |acc, r| {
            let left = acc.x.min(r.x);
            let top = acc.y.min(r.y);
            let right = acc.right().max(r.right());
            let bottom = acc.bottom().max(r.bottom());
            Rect::new(left, top, right - left, bottom - top)
        })
    }

    /// The monitor whose pixels contain `p`.
    pub fn monitor_at(&self, p: Point) -> Option<&Monitor> {
        self.monitors.iter().find(|m| self.physical(m).contains(p))
    }

    /// The monitor containing `p`, or the closest one when `p` is off screen.
    pub fn monitor_near(&self, p: Point) -> Option<&Monitor> {
        self.monitor_at(p).or_else(|| {
            self.monitors.iter().min_by_key(|m| {
                let r = self.physical(m);
                let dx = (r.x - p.x).max(p.x - (r.right() - 1)).max(0) as i64;
                let dy = (r.y - p.y).max(p.y - (r.bottom() - 1)).max(0) as i64;
                dx * dx + dy * dy
            })
        })
    }

    /// Logical coordinates of a physical point and the monitor it falls on.
    pub fn to_logical(&self, p: Point) -> Option<(&Monitor, f64, f64)> {
        let monitor = self.monitor_near(p)?;
        let rect = self.physical(monitor);
        let scale = monitor.scale();
        let x = monitor.logical.x as f64 + (p.x - rect.x) as f64 / scale;
        let y = monitor.logical.y as f64 + (p.y - rect.y) as f64 / scale;
        Some((monitor, x, y))
    }

    /// Physical pixel under logical coordinates such as the compositor's cursor position.
    pub fn to_physical(&self, x: f64, y: f64) -> Option<Point> {
        let monitor = self
            .monitors
            .iter()
            .find(|m| {
                let r = m.logical;
                x >= r.x as f64 && y >= r.y as f64 && x < r.right() as f64 && y < r.bottom() as f64
            })
            .or_else(|| self.monitor_near(Point::new(x.round() as i32, y.round() as i32)))?;
        let rect = self.physical(monitor);
        let scale = monitor.scale();
        let px = rect.x + ((x - monitor.logical.x as f64) * scale).floor() as i32;
        let py = rect.y + ((y - monitor.logical.y as f64) * scale).floor() as i32;
        Some(Point::new(px.clamp(rect.x, rect.right() - 1), py.clamp(rect.y, rect.bottom() - 1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn laptop() -> Monitor {
        Monitor { name: "eDP-1".into(), logical: Rect::new(5589, 97, 1600, 1000), width: 2560, height: 1600 }
    }

    fn dell() -> Monitor {
        Monitor {
            name: "DP-1".into(),
            logical: Rect::new(-3440, -360, 3440, 1440),
            width: 3440,
            height: 1440,
        }
    }

    #[test]
    fn single_scaled_monitor_maps_exactly() {
        let layout = Layout::new(vec![laptop()]);
        assert!((laptop().scale() - 1.6).abs() < 1e-9);
        let screen = layout.virtual_screen();
        assert_eq!(screen, Rect::new(8942, 155, 2560, 1600));
        assert_eq!(layout.to_physical(5589.0, 97.0), Some(Point::new(8942, 155)));
        assert_eq!(layout.to_physical(6327.0, 798.0), Some(Point::new(8942 + 1180, 155 + 1121)));
        let (_, x, y) = layout.to_logical(Point::new(8942 + 1180, 155 + 1121)).unwrap();
        assert!((x - 6327.0).abs() < 1.0 && (y - 798.0).abs() < 1.0, "{x} {y}");
    }

    #[test]
    fn mixed_scales_never_overlap() {
        let layout = Layout::new(vec![dell(), laptop()]);
        let rects = layout.physical_rects();
        assert_eq!(rects[0], Rect::new(-5504, -576, 3440, 1440));
        assert_eq!(rects[1], Rect::new(8942, 155, 2560, 1600));
        assert!(rects[0].right() <= rects[1].x);
        let screen = layout.virtual_screen();
        assert_eq!(screen.x, -5504);
        assert_eq!(screen.right(), 8942 + 2560);
        assert_eq!(layout.to_physical(-3440.0, -360.0), Some(Point::new(-5504, -576)));
        assert_eq!(layout.to_physical(-1.0, 1079.0), Some(Point::new(-5504 + 3439, -576 + 1439)));
    }

    #[test]
    fn off_screen_points_snap_to_the_nearest_monitor() {
        let layout = Layout::new(vec![dell(), laptop()]);
        assert_eq!(layout.monitor_near(Point::new(-9000, 0)).map(|m| m.name.as_str()), Some("DP-1"));
        assert_eq!(layout.monitor_near(Point::new(20000, 0)).map(|m| m.name.as_str()), Some("eDP-1"));
        assert_eq!(layout.monitor_at(Point::new(0, 0)), None);
        assert!(Layout::default().to_physical(1.0, 1.0).is_none());
        assert_eq!(Layout::default().virtual_screen(), Rect::default());
    }
}
