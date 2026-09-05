use embed_manifest::{embed_manifest, manifest::DpiAwareness, new_manifest};

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let manifest = new_manifest("Parrot").dpi_awareness(DpiAwareness::PerMonitorV2);
        embed_manifest(manifest).expect("failed to embed windows manifest");
        // Icon resource for explorer, the taskbar and shortcuts; the manifest above is
        // embedded separately, so this script only carries the icon.
        embed_resource::compile("assets/parrot.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("failed to embed the icon resource");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/parrot.rc");
    println!("cargo:rerun-if-changed=assets/parrot.ico");
}
