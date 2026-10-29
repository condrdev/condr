//! Builds libghostty-vt from the source `ghostty.toml` pins, with the Zig it names,
//! ReleaseFast whatever the Cargo profile, and links it statically (ADR 0051).
//!
//! The tarball and its extracted source are kept in the user's cache directory, outside
//! `target`, so neither `cargo clean` nor another profile downloads it again, and Zig's
//! own cache inside it outlives them too. `CONDR_GHOSTTY_VT_SOURCE_DIR` builds a local
//! Ghostty checkout instead; `CONDR_GHOSTTY_VT_SKIP_NATIVE` builds and links nothing,
//! for checking another target without its toolchain.

#[path = "build/checks.rs"]
mod checks;

use std::{
    env, fs,
    path::PathBuf,
    process::{self, Command},
};

const MANIFEST: &str = include_str!("ghostty.toml");

fn main() {
    println!("cargo:rerun-if-changed=ghostty.toml");
    println!("cargo:rerun-if-changed=build/checks.rs");
    println!("cargo:rerun-if-env-changed=CONDR_GHOSTTY_VT_SOURCE_DIR");
    println!("cargo:rerun-if-env-changed=CONDR_GHOSTTY_VT_SKIP_NATIVE");
    if env::var_os("CONDR_GHOSTTY_VT_SKIP_NATIVE").is_some() {
        return;
    }
    if let Err(error) = build() {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

fn build() -> Result<(), String> {
    let target = env::var("TARGET").unwrap_or_default();
    if target != env::var("HOST").unwrap_or_default() {
        return Err(format!(
            "libghostty-vt is only built for the host, not {target}; set \
             CONDR_GHOSTTY_VT_SKIP_NATIVE to check that target without linking"
        ));
    }
    let zig = field("zig")?;
    let zig_version = Command::new("zig")
        .arg("version")
        .output()
        .map_err(|error| {
            format!("running Zig: {error}; install Zig {zig} with `mise use -g zig@{zig}`")
        })?;
    checks::check_zig(&String::from_utf8_lossy(&zig_version.stdout), zig)?;

    let source = source_dir()?;
    checks::check_headers(&source.join("include"), field("headers_sha256")?)?;

    let prefix = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    // Baseline CPU, so the binary runs on any machine of the target's architecture.
    let status = Command::new("zig")
        .current_dir(&source)
        .args([
            "build",
            "-Demit-lib-vt=true",
            "-Demit-xcframework=false",
            "-Doptimize=ReleaseFast",
            "-Dcpu=baseline",
            "--prefix",
        ])
        .arg(&prefix)
        .status()
        .map_err(|error| format!("running zig build: {error}"))?;
    if !status.success() {
        return Err(format!(
            "zig build in {} failed: {status}",
            source.display()
        ));
    }

    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib").display()
    );
    // Windows names the static library apart from the shared one's import library.
    let library = if target.contains("windows") {
        "ghostty-vt-static"
    } else {
        "ghostty-vt"
    };
    println!("cargo:rustc-link-lib=static={library}");
    Ok(())
}

/// The extracted source of the pinned tarball, downloaded and checked on first use.
fn source_dir() -> Result<PathBuf, String> {
    if let Some(dir) = env::var_os("CONDR_GHOSTTY_VT_SOURCE_DIR") {
        let dir = PathBuf::from(dir);
        println!("cargo:rerun-if-changed={}", dir.display());
        return Ok(dir);
    }
    let cache = dirs::cache_dir()
        .ok_or("this machine has no cache directory to keep libghostty-vt's source in")?
        .join("condr")
        .join("libghostty-vt")
        .join(field("commit")?);
    let source = cache.join("source");
    if source.join("build.zig").is_file() {
        return Ok(source);
    }
    fs::create_dir_all(&cache).map_err(|error| format!("creating {}: {error}", cache.display()))?;

    // Each build downloads and extracts under its own names and renames the tree into
    // place, so a build that races this one never sees half of it.
    let tarball = cache.join(format!("download-{}.tar.gz", process::id()));
    let staging = cache.join(format!("staging-{}", process::id()));
    let url = format!(
        "https://tip.files.ghostty.org/{}/libghostty-vt-source.tar.gz",
        field("commit")?
    );
    let extracted = (|| {
        run(Command::new("curl")
            .args(["-fsSL", "--retry", "3", "-o"])
            .arg(&tarball)
            .arg(&url))?;
        let bytes = fs::read(&tarball).map_err(|error| format!("reading {url}: {error}"))?;
        checks::check_source(&bytes, field("source_sha256")?)?;
        fs::create_dir_all(&staging)
            .map_err(|error| format!("creating {}: {error}", staging.display()))?;
        run(Command::new("tar")
            .arg("-xzf")
            .arg(&tarball)
            .arg("-C")
            .arg(&staging))?;
        let extracted = fs::read_dir(&staging)
            .and_then(|mut entries| entries.next().transpose())
            .map_err(|error| format!("reading {}: {error}", staging.display()))?
            .ok_or("the libghostty-vt source tarball is empty")?
            .path();
        if fs::rename(&extracted, &source).is_err() && !source.join("build.zig").is_file() {
            return Err(format!(
                "moving {} to {}",
                extracted.display(),
                source.display()
            ));
        }
        Ok(())
    })();
    let _ = fs::remove_file(&tarball);
    let _ = fs::remove_dir_all(&staging);
    extracted.map(|()| source)
}

/// A `key = "value"` line of `ghostty.toml`, which the update script writes.
fn field(key: &str) -> Result<&'static str, String> {
    MANIFEST
        .lines()
        .find_map(|line| {
            let value = line
                .strip_prefix(key)?
                .trim_start()
                .strip_prefix('=')?
                .trim();
            value.strip_prefix('"')?.strip_suffix('"')
        })
        .ok_or_else(|| format!("ghostty.toml has no {key}"))
}

fn run(command: &mut Command) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|error| format!("running {command:?}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{command:?} failed: {status}"))
    }
}
