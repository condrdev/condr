//! Embed the same icon in both Windows executables, without propagating the
//! resource through condr-server's library into its GUI consumer.

use std::{env, fs, path::Path};

pub fn embed_icon(binary: &str) {
    println!("cargo:rerun-if-changed=../../packaging/windows.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    let icon = Path::new(&manifest).join("../../packaging/icons/condr.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let resource = Path::new(&env::var("OUT_DIR").expect("OUT_DIR is set")).join("condr.rc");
    // GPUI's Windows platform loads icon resource 1 from the executable. This
    // does not replace GPUI's manifest: Windows resources also have a type.
    let icon_path = icon.display().to_string().replace('\\', "/");
    fs::write(&resource, format!("1 ICON \"{icon_path}\"\n")).expect("write icon resource");
    embed_resource::compile_for(resource, [binary], embed_resource::NONE)
        .manifest_required()
        .expect("compile Windows icon resource");
}
