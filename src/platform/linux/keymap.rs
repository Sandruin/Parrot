use anyhow::Result;

/// A compiled xkb keymap used to resolve layout-dependent keys.
pub struct Xkb {}

impl Xkb {
    /// Compiles the keymap text the compositor shared.
    pub fn new(_text: &str) -> Result<Self> {
        Ok(Self {})
    }

    /// Windows virtual-key code for a letter or digit key on this layout, `None` for other keys.
    pub fn vk_for_keycode(&self, _evdev: u16) -> Option<u16> {
        None
    }
}
