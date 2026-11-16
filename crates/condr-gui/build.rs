//! Embeds Windows branding and vendored Alacritty color schemes, and rebuilds the GUI
//! when a translation changes.

#[path = "../../packaging/windows.rs"]
mod windows;

use std::path::Path;
use std::{env, fs};

fn main() {
    windows::embed_icon("condr-gui");
    // Read at run time, not `env!` at compile time: a compiled-in path goes stale when the
    // checkout moves, and cargo has no reason to rebuild the script for that.
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    let schemes = Path::new(&manifest_dir).join("assets/color_schemes");
    println!("cargo:rerun-if-changed={}", schemes.display());
    // `rust_i18n::i18n!` reads the locale files at compile time but tells cargo nothing.
    let locales = Path::new(&manifest_dir).join("locales");
    println!("cargo:rerun-if-changed={}", locales.display());

    let mut files = fs::read_dir(&schemes)
        .expect("assets/color_schemes must exist")
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "toml")
        })
        .collect::<Vec<_>>();
    // By scheme name, not file name: "Adventure" must precede "Adventure Time".
    files.sort_by(|a, b| a.file_stem().cmp(&b.file_stem()));

    let mut source = String::from("pub(crate) static BUILT_IN: &[(&str, &str)] = &[\n");
    for path in files {
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("scheme file names are UTF-8");
        source.push_str(&format!(
            "    ({name:?}, include_str!({:?})),\n",
            path.display().to_string()
        ));
    }
    source.push_str("];\n");

    let out = Path::new(&env::var("OUT_DIR").expect("OUT_DIR is set")).join("color_schemes.rs");
    fs::write(out, source).expect("write color_schemes.rs");
}
