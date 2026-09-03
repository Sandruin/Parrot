use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use image::{Rgba, RgbaImage};
use wayland_client::protocol::wl_shm;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1};

use super::layout::{Layout, Monitor};
use super::wayland::{ShmBuffer, State, Wayland};
use crate::model::Rect;
use crate::platform::ScreenCapture;

/// How long a single screencopy request may take before it is treated as a failure.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(2);

/// One in-flight screencopy frame's fields as the compositor's events fill them in.
#[derive(Default)]
struct FrameState {
    format: Option<wl_shm::Format>,
    width: i32,
    height: i32,
    stride: i32,
    y_invert: bool,
    buffer_done: bool,
    ready: bool,
    failed: bool,
}

/// Screencopy bookkeeping the Wayland dispatcher fills in while a capture is pending.
#[derive(Default)]
pub struct WlState {
    frame: FrameState,
}

/// The connection and event queue this backend keeps open for the life of the process.
struct Session {
    wayland: Wayland,
    queue: EventQueue<State>,
}

/// Screen capture through wlr-screencopy on its own Wayland connection.
pub struct WaylandCapture {
    session: Mutex<Session>,
}

impl WaylandCapture {
    pub fn new() -> Result<Self> {
        let (wayland, queue) = Wayland::connect().context("connecting to Wayland for screen capture")?;
        Ok(Self { session: Mutex::new(Session { wayland, queue }) })
    }

    /// Current monitor layout after a best-effort roundtrip to pick up hot-plugged outputs.
    fn layout(&self) -> Layout {
        let mut guard = self.session.lock().unwrap();
        let Session { wayland, queue } = &mut *guard;
        if let Err(e) = wayland.roundtrip(queue) {
            log::warn!("wayland roundtrip while reading the output layout failed: {e:#}");
        }
        wayland.layout()
    }
}

impl ScreenCapture for WaylandCapture {
    fn virtual_screen(&self) -> Rect {
        self.layout().virtual_screen()
    }

    fn monitors(&self) -> Vec<Rect> {
        self.layout().physical_rects()
    }

    fn capture(&self, region: Rect) -> Result<RgbaImage> {
        if region.w <= 0 || region.h <= 0 {
            bail!("capture region {region:?} is empty");
        }
        let mut session = self.session.lock().unwrap();
        {
            let Session { wayland, queue } = &mut *session;
            wayland.roundtrip(queue).context("refreshing the output layout")?;
        }
        if session.wayland.state.screencopy.is_none() {
            bail!("compositor has no zwlr_screencopy_manager_v1");
        }
        let layout = session.wayland.layout();

        let mut image = RgbaImage::from_pixel(region.w as u32, region.h as u32, Rgba([0, 0, 0, 255]));
        for monitor in layout.monitors() {
            let mrect = layout.physical(monitor);
            if let Some(overlap) = intersect(mrect, region) {
                capture_monitor(&mut session, monitor, mrect, overlap, region, &mut image)?;
            }
        }
        Ok(image)
    }
}

/// Overlap of two rectangles, or `None` when they do not touch.
fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = a.right().min(b.right());
    let bottom = a.bottom().min(b.bottom());
    (right > x && bottom > y).then(|| Rect::new(x, y, right - x, bottom - y))
}

/// Output-local logical rectangle that fully covers `overlap`, floored and ceiled to whole units.
fn logical_request(monitor: &Monitor, mrect: Rect, overlap: Rect) -> (i32, i32, i32, i32) {
    let scale = monitor.scale();
    let local_x = overlap.x - mrect.x;
    let local_y = overlap.y - mrect.y;
    let lx0 = ((local_x as f64 / scale).floor() as i32).clamp(0, monitor.logical.w);
    let ly0 = ((local_y as f64 / scale).floor() as i32).clamp(0, monitor.logical.h);
    let lx1 = (((local_x + overlap.w) as f64 / scale).ceil() as i32).clamp(lx0, monitor.logical.w);
    let ly1 = (((local_y + overlap.h) as f64 / scale).ceil() as i32).clamp(ly0, monitor.logical.h);
    (lx0, ly0, (lx1 - lx0).max(1), (ly1 - ly0).max(1))
}

/// Whether the buffer's red and blue channels must be swapped to produce RGBA.
fn channel_order(format: wl_shm::Format) -> Result<bool> {
    match format {
        wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888 => Ok(true),
        wl_shm::Format::Abgr8888 | wl_shm::Format::Xbgr8888 => Ok(false),
        other => bail!("unsupported screencopy buffer format {other:?}"),
    }
}

/// Sends `destroy` for the wrapped frame once it goes out of scope, on every return path.
struct DestroyOnDrop<'a>(&'a ZwlrScreencopyFrameV1);

impl Drop for DestroyOnDrop<'_> {
    fn drop(&mut self) {
        self.0.destroy();
    }
}

/// Blocks until `ready` returns true for the pending frame, or bails out past `deadline`.
fn wait_until(
    wayland: &mut Wayland,
    queue: &mut EventQueue<State>,
    deadline: Instant,
    ready: impl Fn(&FrameState) -> bool,
) -> Result<()> {
    while !ready(&wayland.state.capture.frame) {
        if Instant::now() >= deadline {
            bail!("timed out waiting for the compositor's screencopy events");
        }
        wayland.roundtrip(queue)?;
    }
    Ok(())
}

/// Requests, waits for and blits the part of `monitor` that `overlap` covers into `image`.
fn capture_monitor(
    session: &mut Session,
    monitor: &Monitor,
    mrect: Rect,
    overlap: Rect,
    region: Rect,
    image: &mut RgbaImage,
) -> Result<()> {
    let Session { wayland, queue } = session;
    let (lx0, ly0, lw, lh) = logical_request(monitor, mrect, overlap);

    let manager = wayland.state.screencopy.clone().context("compositor has no zwlr_screencopy_manager_v1")?;
    let output = wayland
        .state
        .output_named(&monitor.name)
        .with_context(|| format!("no wl_output for monitor {}", monitor.name))?
        .output
        .clone();

    wayland.state.capture.frame = FrameState::default();
    let frame = manager.capture_output_region(0, &output, lx0, ly0, lw, lh, &wayland.qh, ());
    let _destroy = DestroyOnDrop(&frame);
    wayland.flush()?;

    let deadline = Instant::now() + CAPTURE_TIMEOUT;
    wait_until(wayland, queue, deadline, |f| f.buffer_done || f.failed)?;
    if wayland.state.capture.frame.failed {
        bail!("screencopy request failed for {}", monitor.name);
    }
    let format = wayland
        .state
        .capture
        .frame
        .format
        .with_context(|| format!("compositor offered no wl_shm buffer for {}", monitor.name))?;
    let bw = wayland.state.capture.frame.width;
    let bh = wayland.state.capture.frame.height;
    let swap_rb = channel_order(format)?;

    let shm = wayland.state.shm.clone().context("compositor has no wl_shm")?;
    let shm_buffer = ShmBuffer::new(&shm, &wayland.qh, bw, bh, format)?;
    frame.copy(&shm_buffer.buffer);
    wayland.flush()?;

    let deadline = Instant::now() + CAPTURE_TIMEOUT;
    wait_until(wayland, queue, deadline, |f| f.ready || f.failed)?;
    if wayland.state.capture.frame.failed {
        bail!("screencopy copy failed for {}", monitor.name);
    }
    let y_invert = wayland.state.capture.frame.y_invert;

    let buf_x0 = (lx0 as f64 * monitor.scale()).round() as i32;
    let buf_y0 = (ly0 as f64 * monitor.scale()).round() as i32;
    let crop_x = (overlap.x - mrect.x) - buf_x0;
    let crop_y = (overlap.y - mrect.y) - buf_y0;
    blit(image, region, overlap, &shm_buffer, y_invert, swap_rb, (crop_x, crop_y));
    Ok(())
}

/// Copies the shm buffer's pixels covering `overlap` into `image`, converting to RGBA.
fn blit(
    image: &mut RgbaImage,
    region: Rect,
    overlap: Rect,
    buf: &ShmBuffer,
    y_invert: bool,
    swap_rb: bool,
    crop: (i32, i32),
) {
    let (crop_x, crop_y) = crop;
    let stride = buf.stride as usize;
    for row in 0..overlap.h {
        let src_y = crop_y + row;
        if src_y < 0 || src_y >= buf.height {
            continue;
        }
        let buf_row = if y_invert { buf.height - 1 - src_y } else { src_y };
        let row_start = buf_row as usize * stride;
        let row_bytes = &buf.map[row_start..row_start + stride];
        let dst_y = (overlap.y - region.y + row) as u32;
        for col in 0..overlap.w {
            let src_x = crop_x + col;
            if src_x < 0 || src_x >= buf.width {
                continue;
            }
            let px = &row_bytes[src_x as usize * 4..src_x as usize * 4 + 4];
            let rgba = if swap_rb { [px[2], px[1], px[0], 255] } else { [px[0], px[1], px[2], 255] };
            let dst_x = (overlap.x - region.x + col) as u32;
            image.put_pixel(dst_x, dst_y, Rgba(rgba));
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let frame = &mut state.capture.frame;
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer { format, width, height, stride } => {
                if let WEnum::Value(format) = format {
                    frame.format = Some(format);
                }
                frame.width = width as i32;
                frame.height = height as i32;
                frame.stride = stride as i32;
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => frame.buffer_done = true,
            zwlr_screencopy_frame_v1::Event::Flags { flags: WEnum::Value(flags) } => {
                frame.y_invert = flags.contains(zwlr_screencopy_frame_v1::Flags::YInvert);
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => frame.ready = true,
            zwlr_screencopy_frame_v1::Event::Failed => frame.failed = true,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    /// Runs grim on the given logical geometry and loads the resulting PNG as RGBA.
    fn grim_shot(geometry: &str) -> RgbaImage {
        let path = std::env::temp_dir().join(format!("parrot-capture-test-{}.png", std::process::id()));
        let status = Command::new("grim")
            .arg("-g")
            .arg(geometry)
            .arg(&path)
            .status()
            .expect("failed to run grim, is it installed?");
        assert!(status.success(), "grim exited with {status}");
        let image = image::open(&path).expect("failed to load grim output").to_rgba8();
        let _ = std::fs::remove_file(&path);
        image
    }

    /// Compares two same-sized images pixel by pixel, returning the count that differ by more
    /// than one level per channel.
    fn mismatch_count(a: &RgbaImage, b: &RgbaImage) -> usize {
        assert_eq!(a.dimensions(), b.dimensions(), "image sizes differ");
        a.pixels()
            .zip(b.pixels())
            .filter(|(pa, pb)| {
                pa.0.iter().zip(pb.0.iter()).any(|(ca, cb)| (*ca as i16 - *cb as i16).abs() > 1)
            })
            .count()
    }

    #[test]
    #[ignore = "requires a live Hyprland session with grim installed"]
    fn matches_grim_top_left() {
        let reference = grim_shot("5589,97 400x300");
        let capture = WaylandCapture::new().expect("connect");
        let shot = capture.capture(Rect::new(8942, 155, 640, 480)).expect("capture");
        let mismatches = mismatch_count(&reference, &shot);
        eprintln!("top-left region: {mismatches} mismatched pixels out of {}", reference.pixels().len());
        assert!(mismatches * 1000 < reference.pixels().len(), "too many mismatched pixels: {mismatches}");
    }

    #[test]
    #[ignore = "requires a live Hyprland session with grim installed"]
    fn region_crossing_the_monitor_edge_blacks_the_outside_and_matches_grim_inside() {
        // Physical 8842,155 300x200 straddles the left edge of eDP-1 at 8942,155; its right
        // 200 columns are logical 5589,97 125x125 on the monitor at scale 1.6.
        let reference = grim_shot("5589,97 125x125");
        let capture = WaylandCapture::new().expect("connect");
        let shot = capture.capture(Rect::new(8842, 155, 300, 200)).expect("capture");
        assert_eq!((shot.width(), shot.height()), (300, 200));

        for y in 0..200u32 {
            for x in 0..100u32 {
                assert_eq!(
                    shot.get_pixel(x, y).0,
                    [0, 0, 0, 255],
                    "expected black outside the monitor at {x},{y}"
                );
            }
        }

        let overlap = image::imageops::crop_imm(&shot, 100, 0, 200, 200).to_image();
        let mismatches = mismatch_count(&reference, &overlap);
        eprintln!("edge-crossing region: {mismatches} mismatched pixels out of {}", reference.pixels().len());
        assert!(mismatches * 1000 < reference.pixels().len(), "too many mismatched pixels: {mismatches}");
    }

    #[test]
    #[ignore = "requires a live Hyprland session"]
    fn region_outside_every_monitor_is_black() {
        let capture = WaylandCapture::new().expect("connect");
        let screen = capture.virtual_screen();
        let region = Rect::new(screen.right() + 1000, screen.y, 100, 100);
        let shot = capture.capture(region).expect("capture");
        assert!(shot.pixels().all(|p| p.0 == [0, 0, 0, 255]));
    }
}
