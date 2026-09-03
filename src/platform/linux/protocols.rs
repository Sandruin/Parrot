#![allow(non_upper_case_globals, non_camel_case_types, unused_imports, missing_docs, clippy::all)]

pub(crate) use wayland_backend;
pub(crate) use wayland_client;

pub mod __interfaces {
    use wayland_client::protocol::__interfaces::*;
    wayland_scanner::generate_interfaces!("protocols/hyprland-global-shortcuts-v1.xml");
}

use self::__interfaces::*;
use wayland_client::protocol::*;
wayland_scanner::generate_client_code!("protocols/hyprland-global-shortcuts-v1.xml");
