use anyhow::Result;

use super::wayland::Wayland;
use crate::model::OverlayScene;

/// Layer-shell namespace of the overlay surfaces, matched by the compositor's layer rules.
pub const NAMESPACE: &str = "macro-recorder-overlay";

/// Set this environment variable to keep the overlay visible to screen capture, for manual checks.
pub const CAPTURABLE_ENV: &str = "MACRO_OVERLAY_CAPTURABLE";

/// Layer surface events the Wayland dispatcher collects for [`Overlay::poll`].
#[derive(Default)]
pub struct WlState {}

/// Click-through layer-shell surfaces that draw an [`OverlayScene`] on the outputs it touches.
#[derive(Default)]
pub struct Overlay {}

impl Overlay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Renders the scene and shows it without taking focus.
    pub fn show(&mut self, _wl: &mut Wayland, _scene: &OverlayScene) -> Result<()> {
        Ok(())
    }

    pub fn hide(&mut self, _wl: &mut Wayland) {}

    /// Handles configure events that arrived since the last dispatch.
    pub fn poll(&mut self, _wl: &mut Wayland) {}
}
