//! `build.rs` refuses a source tarball, a Zig and headers other than the ones
//! `ghostty.toml` names (ADR 0051).

#[path = "../build/checks.rs"]
mod checks;

use std::{fs, process};

/// SHA-256 of `abc`.
const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

#[test]
fn a_tarball_with_another_hash_is_refused() {
    assert!(checks::check_source(b"abc", ABC).is_ok());
    assert!(checks::check_source(b"abd", ABC).is_err());
}

#[test]
fn another_zig_is_refused() {
    assert!(checks::check_zig("0.17.0\n", "0.17.0").is_ok());
    let refused = checks::check_zig("0.16.0\n", "0.17.0").unwrap_err();
    assert!(refused.contains("mise use -g zig@0.17.0"), "{refused}");
}

#[test]
fn headers_other_than_the_bindings_are_refused() {
    let include = std::env::temp_dir().join(format!("condr-ghostty-vt-headers-{}", process::id()));
    fs::create_dir_all(include.join("ghostty/vt")).unwrap();
    // `vt.h` sorts before `vt/` as the script's `sort` puts it, so the bytes are "ab".
    fs::write(include.join("ghostty/vt/terminal.h"), "b").unwrap();
    fs::write(include.join("ghostty/vt.h"), "a").unwrap();
    fs::write(include.join("ghostty/README"), "not a header").unwrap();
    fs::write(include.join("ghostty.h"), "the full app's").unwrap();
    let ab = "fb8e20fc2e4c3f248c60c39bd652f3c1347298bb977b8b4d5903b85055620603";

    assert!(checks::check_headers(&include, ab).is_ok());
    fs::write(include.join("ghostty/vt/terminal.h"), "c").unwrap();
    assert!(checks::check_headers(&include, ab).is_err());
    fs::remove_dir_all(&include).unwrap();
}
