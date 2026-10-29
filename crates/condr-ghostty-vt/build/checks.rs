//! What `build.rs` checks before it builds libghostty-vt, apart so the crate's tests run
//! it too (ADR 0051).

use sha2::{Digest, Sha256};
use std::{fs, io, path::Path};

pub fn check_source(tarball: &[u8], expected: &str) -> Result<(), String> {
    let found = hex(&Sha256::digest(tarball));
    if found == expected {
        Ok(())
    } else {
        Err(format!(
            "the libghostty-vt source tarball's SHA-256 is {found}, not the {expected} \
             ghostty.toml pins"
        ))
    }
}

/// `found` is what `zig version` printed.
pub fn check_zig(found: &str, expected: &str) -> Result<(), String> {
    let found = found.trim();
    if found == expected {
        Ok(())
    } else {
        Err(format!(
            "libghostty-vt builds with Zig {expected}, but `zig version` says {found}; \
             install it with `mise use -g zig@{expected}`"
        ))
    }
}

pub fn check_headers(include: &Path, expected: &str) -> Result<(), String> {
    let found = headers_sha256(include)
        .map_err(|error| format!("reading {}: {error}", include.display()))?;
    if found == expected {
        Ok(())
    } else {
        Err(format!(
            "the headers in {} are not the ones src/bindings.rs was generated from; move \
             the pin with script/update-libghostty-vt.sh",
            include.display()
        ))
    }
}

/// Every header under `include/ghostty`, concatenated in the byte order of their
/// `/`-separated paths, as script/update-libghostty-vt.sh hashes them. The full app's
/// `include/ghostty.h` beside them is not libghostty-vt's.
pub fn headers_sha256(include: &Path) -> io::Result<String> {
    let mut paths = Vec::new();
    collect_headers(include, &include.join("ghostty"), &mut paths)?;
    paths.sort();
    let mut hasher = Sha256::new();
    for path in paths {
        hasher.update(fs::read(include.join(path))?);
    }
    Ok(hex(&hasher.finalize()))
}

fn collect_headers(include: &Path, dir: &Path, paths: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_headers(include, &path, paths)?;
        } else if path.extension().is_some_and(|extension| extension == "h") {
            let relative = path.strip_prefix(include).expect("found under include");
            let parts: Vec<_> = relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect();
            paths.push(parts.join("/"));
        }
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
