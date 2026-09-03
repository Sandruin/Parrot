use anyhow::{Context, Result};
use tiny_skia::Pixmap;
use wayland_client::protocol::wl_shm;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1,
};

use super::layout::Monitor;
use super::wayland::{ShmBuffer, State, Wayland};
use crate::model::{OverlayScene, Rect};
use crate::platform::overlay_render::{render, window_rect};

/// Layer-shell namespace of the overlay surfaces, matched by the compositor's layer rules.
pub const NAMESPACE: &str = "macro-recorder-overlay";

/// Set this environment variable to keep the overlay visible to screen capture, for manual checks.
pub const CAPTURABLE_ENV: &str = "MACRO_OVERLAY_CAPTURABLE";

/// One configure or closed event a layer surface saw, keyed by the id given to it at creation.
enum SurfaceEvent {
    Configure { serial: u32, width: u32, height: u32 },
    Closed,
}

/// Layer surface events the Wayland dispatcher collects for [`Overlay::poll`].
#[derive(Default)]
pub struct WlState {
    events: Vec<(u32, SurfaceEvent)>,
}

/// One layer-shell surface covering the part of the scene that falls on a single monitor.
struct Surface {
    id: u32,
    monitor: String,
    wl_surface: WlSurface,
    layer_surface: ZwlrLayerSurfaceV1,
    viewport: Option<WpViewport>,
    /// Covered rect in logical coordinates, relative to the monitor's own logical origin.
    logical: Rect,
    /// Set once the compositor has sent the first `configure`, after which buffers may be attached.
    configured: bool,
    /// Rendered content waiting for the first `configure` before it can be attached.
    pending: Option<Pixmap>,
    buffer: Option<ShmBuffer>,
    /// Kept alive until superseded by another buffer, since the compositor's release event is unused.
    previous: Option<ShmBuffer>,
}

impl Drop for Surface {
    fn drop(&mut self) {
        if let Some(viewport) = self.viewport.take() {
            viewport.destroy();
        }
        self.layer_surface.destroy();
        self.wl_surface.destroy();
    }
}

/// Click-through layer-shell surfaces that draw an [`OverlayScene`] on the outputs it touches.
#[derive(Default)]
pub struct Overlay {
    surfaces: Vec<Surface>,
    next_id: u32,
}

impl Overlay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Renders the scene and shows it without taking focus.
    pub fn show(&mut self, wl: &mut Wayland, scene: &OverlayScene) -> Result<()> {
        let layout = wl.layout();
        let Some(scene_rect) = window_rect(scene, layout.virtual_screen()) else {
            self.hide(wl);
            return Ok(());
        };
        let has_viewport = wl.state.viewporter.is_some();

        let mut wanted = Vec::new();
        for monitor in layout.monitors() {
            let mon_physical = layout.physical(monitor);
            let Some((physical, logical)) = covered_rect(monitor, mon_physical, scene_rect, has_viewport)
            else {
                continue;
            };
            let pixmap = render(scene, physical).context("rendering the overlay scene")?;
            wanted.push(monitor.name.clone());
            self.place(wl, monitor, physical, logical, pixmap)?;
        }

        if wanted.is_empty() {
            log::debug!("overlay scene lies outside every monitor");
            self.hide(wl);
            return Ok(());
        }
        self.surfaces.retain(|s| wanted.contains(&s.monitor));
        Ok(())
    }

    pub fn hide(&mut self, _wl: &mut Wayland) {
        self.surfaces.clear();
    }

    /// Handles configure and closed events that arrived since the last dispatch.
    pub fn poll(&mut self, wl: &mut Wayland) {
        let events = std::mem::take(&mut wl.state.overlay.events);
        for (id, event) in events {
            match event {
                SurfaceEvent::Configure { serial, width, height } => {
                    let Some(surface) = self.surfaces.iter_mut().find(|s| s.id == id) else { continue };
                    surface.layer_surface.ack_configure(serial);
                    log::trace!("overlay surface on {} configured to {width}x{height}", surface.monitor);
                    if !surface.configured {
                        surface.configured = true;
                        if let Some(pixmap) = surface.pending.take()
                            && let Err(e) = attach(wl, surface, pixmap)
                        {
                            log::error!("attaching the overlay buffer failed: {e:#}");
                        }
                    }
                }
                SurfaceEvent::Closed => self.surfaces.retain(|s| s.id != id),
            }
        }
    }

    /// Updates the surface already covering `monitor`, or creates one, and attaches or queues `pixmap`.
    fn place(
        &mut self,
        wl: &Wayland,
        monitor: &Monitor,
        physical: Rect,
        logical: Rect,
        pixmap: Pixmap,
    ) -> Result<()> {
        if let Some(surface) = self.surfaces.iter_mut().find(|s| s.monitor == monitor.name) {
            let resized = surface.logical != logical;
            surface.logical = logical;
            if resized {
                resize(surface, logical);
            }
            if surface.configured {
                attach(wl, surface, pixmap)?;
            } else {
                surface.pending = Some(pixmap);
                if resized {
                    surface.wl_surface.commit();
                }
            }
            return Ok(());
        }
        self.create(wl, monitor, physical, logical, pixmap)
    }

    /// Creates the layer surface for a monitor and commits it without a buffer, per the protocol.
    fn create(
        &mut self,
        wl: &Wayland,
        monitor: &Monitor,
        physical: Rect,
        logical: Rect,
        pixmap: Pixmap,
    ) -> Result<()> {
        let compositor = wl.state.compositor.as_ref().context("no wl_compositor")?;
        let layer_shell = wl.state.layer_shell.as_ref().context("no zwlr_layer_shell_v1")?;
        let output = wl.state.output_named(&monitor.name).map(|o| &o.output);

        let wl_surface = compositor.create_surface(&wl.qh, ());
        let region = compositor.create_region(&wl.qh, ());
        wl_surface.set_input_region(Some(&region));
        region.destroy();

        let id = self.next_id;
        self.next_id += 1;
        let layer_surface = layer_shell.get_layer_surface(
            &wl_surface,
            output,
            Layer::Overlay,
            NAMESPACE.to_string(),
            &wl.qh,
            id,
        );
        layer_surface.set_anchor(Anchor::Top | Anchor::Left);
        layer_surface.set_size(logical.w as u32, logical.h as u32);
        layer_surface.set_margin(logical.y, 0, 0, logical.x);
        layer_surface.set_exclusive_zone(-1);
        layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);

        let viewport = wl.state.viewporter.as_ref().map(|viewporter| {
            let viewport = viewporter.get_viewport(&wl_surface, &wl.qh, ());
            viewport.set_destination(logical.w, logical.h);
            viewport
        });
        if viewport.is_none() {
            wl_surface.set_buffer_scale(monitor.scale().ceil().max(1.0) as i32);
        }
        wl_surface.commit();

        log::debug!("overlay surface created on {} covering {physical:?}", monitor.name);
        self.surfaces.push(Surface {
            id,
            monitor: monitor.name.clone(),
            wl_surface,
            layer_surface,
            viewport,
            logical,
            configured: false,
            pending: Some(pixmap),
            buffer: None,
            previous: None,
        });
        Ok(())
    }
}

/// Re-applies size, margin and viewport destination after the covered rect changed.
fn resize(surface: &mut Surface, logical: Rect) {
    surface.layer_surface.set_size(logical.w as u32, logical.h as u32);
    surface.layer_surface.set_margin(logical.y, 0, 0, logical.x);
    if let Some(viewport) = &surface.viewport {
        viewport.set_destination(logical.w, logical.h);
    }
}

/// Uploads the pixmap as BGRA into a fresh shm buffer and attaches, damages and commits it.
fn attach(wl: &Wayland, surface: &mut Surface, pixmap: Pixmap) -> Result<()> {
    let shm = wl.state.shm.as_ref().context("no wl_shm")?;
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let mut buffer = ShmBuffer::new(shm, &wl.qh, width, height, wl_shm::Format::Argb8888)
        .context("allocating the overlay shm buffer")?;
    let mut pixels = pixmap.take();
    for chunk in pixels.as_chunks_mut::<4>().0 {
        chunk.swap(0, 2);
    }
    buffer.map.copy_from_slice(&pixels);

    surface.wl_surface.attach(Some(&buffer.buffer), 0, 0);
    surface.wl_surface.damage_buffer(0, 0, width, height);
    surface.wl_surface.commit();
    surface.previous = surface.buffer.replace(buffer);
    Ok(())
}

/// The rect a scene covers on one monitor, in physical virtual-screen and monitor-logical space.
///
/// The rect is aligned outward to the coarsest step that keeps `logical size * scale` exactly
/// integral: the exact fractional steps derived from the output's true scale when a `wp_viewport`
/// will remap the buffer, or whole physical pixels at the ceiling integer scale otherwise.
fn covered_rect(
    monitor: &Monitor,
    mon_physical: Rect,
    scene_rect: Rect,
    has_viewport: bool,
) -> Option<(Rect, Rect)> {
    let inter = intersect(scene_rect, mon_physical)?;
    let (step_phys_x, step_log_x, step_phys_y, step_log_y) = if has_viewport {
        let (px, lx) = axis_steps(monitor.width, monitor.logical.w);
        let (py, ly) = axis_steps(monitor.height, monitor.logical.h);
        (px, lx, py, ly)
    } else {
        let scale = monitor.scale().ceil().max(1.0) as i32;
        (scale, 1, scale, 1)
    };

    let local_left = inter.x - mon_physical.x;
    let local_top = inter.y - mon_physical.y;
    let local_right = inter.right() - mon_physical.x;
    let local_bottom = inter.bottom() - mon_physical.y;

    let left = align_down(local_left, step_phys_x).max(0);
    let top = align_down(local_top, step_phys_y).max(0);
    let right = align_up(local_right, step_phys_x).min(align_down(monitor.width, step_phys_x));
    let bottom = align_up(local_bottom, step_phys_y).min(align_down(monitor.height, step_phys_y));
    if right <= left || bottom <= top {
        return None;
    }

    let logical = Rect::new(
        left / step_phys_x * step_log_x,
        top / step_phys_y * step_log_y,
        (right - left) / step_phys_x * step_log_x,
        (bottom - top) / step_phys_y * step_log_y,
    );
    let physical = Rect::new(mon_physical.x + left, mon_physical.y + top, right - left, bottom - top);
    Some((physical, logical))
}

/// Physical and logical pixel counts of the coarsest step that divides both exactly.
fn axis_steps(physical_len: i32, logical_len: i32) -> (i32, i32) {
    let g = gcd(physical_len, logical_len);
    (physical_len / g, logical_len / g)
}

fn gcd(a: i32, b: i32) -> i32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn align_down(value: i32, step: i32) -> i32 {
    value / step * step
}

fn align_up(value: i32, step: i32) -> i32 {
    align_down(value + step - 1, step)
}

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = a.right().min(b.right());
    let bottom = a.bottom().min(b.bottom());
    (right > x && bottom > y).then(|| Rect::new(x, y, right - x, bottom - y))
}

impl Dispatch<ZwlrLayerSurfaceV1, u32> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let event = match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                SurfaceEvent::Configure { serial, width, height }
            }
            zwlr_layer_surface_v1::Event::Closed => SurfaceEvent::Closed,
            _ => return,
        };
        state.overlay.events.push((*id, event));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn laptop() -> Monitor {
        Monitor { name: "eDP-1".into(), logical: Rect::new(5589, 97, 1600, 1000), width: 2560, height: 1600 }
    }

    #[test]
    fn covered_rect_stays_pixel_exact_with_a_viewport() {
        let monitor = laptop();
        let mon_physical = Rect::new(8942, 155, 2560, 1600);
        // A crosshair-sized region roughly centred on the monitor.
        let scene_rect = Rect::new(9200, 400, 200, 200);
        let (physical, logical) = covered_rect(&monitor, mon_physical, scene_rect, true).unwrap();

        assert!(physical.x >= mon_physical.x && physical.right() <= mon_physical.right());
        assert!(physical.y >= mon_physical.y && physical.bottom() <= mon_physical.bottom());
        assert!(physical.contains(crate::model::Point::new(9200, 400)));

        // logical size and origin must be multiples of 5 at this monitor's 1.6 scale.
        assert_eq!(logical.x % 5, 0);
        assert_eq!(logical.y % 5, 0);
        assert_eq!(logical.w % 5, 0);
        assert_eq!(logical.h % 5, 0);

        // and the buffer we would render must map 1:1 onto physical pixels through the viewport.
        assert_eq!((logical.w as f64 * monitor.scale()).round() as i32, physical.w);
        assert_eq!((logical.h as f64 * monitor.scale()).round() as i32, physical.h);
    }

    #[test]
    fn covered_rect_without_viewport_uses_the_ceiling_integer_scale() {
        let monitor = laptop();
        let mon_physical = Rect::new(8942, 155, 2560, 1600);
        let scene_rect = Rect::new(9200, 400, 200, 200);
        let (physical, logical) = covered_rect(&monitor, mon_physical, scene_rect, false).unwrap();
        assert_eq!(physical.w, logical.w * 2);
        assert_eq!(physical.h, logical.h * 2);
    }

    #[test]
    fn covered_rect_is_none_off_monitor() {
        let monitor = laptop();
        let mon_physical = Rect::new(8942, 155, 2560, 1600);
        let far_away = Rect::new(-500, -500, 10, 10);
        assert!(covered_rect(&monitor, mon_physical, far_away, true).is_none());
    }

    #[test]
    fn covered_rect_clamps_to_the_monitor_bounds() {
        let monitor = laptop();
        let mon_physical = Rect::new(8942, 155, 2560, 1600);
        let huge = Rect::new(0, 0, 20000, 20000);
        let (physical, logical) = covered_rect(&monitor, mon_physical, huge, true).unwrap();
        assert_eq!(physical, mon_physical);
        assert_eq!(logical, Rect::new(0, 0, monitor.logical.w, monitor.logical.h));
    }
}
