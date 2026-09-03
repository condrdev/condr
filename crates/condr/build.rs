//! Embeds every vendored Alacritty color scheme as `(name, toml)` pairs.

use std::path::Path;
use std::{env, fs};

fn main() {
    let schemes = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/color_schemes");
    println!("cargo:rerun-if-changed={}", schemes.display());

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
