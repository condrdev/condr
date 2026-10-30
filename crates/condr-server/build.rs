//! Embeds the icon and, for Windows, puts Microsoft's ConPTY beside the built `condr.exe`,
//! where portable-pty loads it instead of the one Windows ships (ADR 0056).
//!
//! The package is downloaded once, checked against the SHA-256 pinned here and kept
//! unpacked in the user's cache directory, outside `target`, as libghostty-vt's source
//! is (ADR 0051). Only a Windows host unpacks it; another host checking the Windows
//! target builds a `condr.exe` that falls back to Windows' own ConPTY.

#[path = "../../packaging/windows.rs"]
mod windows;

use std::{
    env, fs,
    path::PathBuf,
    process::{self, Command},
};

use sha2::{Digest, Sha256};

/// Microsoft.Windows.Console.ConPTY on nuget.org: `conpty.dll` and the `OpenConsole.exe`
/// it starts, from github.com/microsoft/terminal under the MIT license.
const CONPTY_VERSION: &str = "1.25.260930003";
const CONPTY_SHA256: &str = "02b07b349af66d801159bdf9e440d4a1ce78bb951f37fc8609731665afdae7ee";

fn main() {
    windows::embed_icon("condr");
    if !cfg!(windows) || env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if let Err(error) = stage_conpty() {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

fn stage_conpty() -> Result<(), String> {
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("aarch64") => "arm64",
        other => return Err(format!("the ConPTY package has no build for {other:?}")),
    };
    let package = conpty_package()?;
    // OUT_DIR is <profile>/build/condr-server-<hash>/out; Cargo puts condr.exe in <profile>.
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let bin = out
        .ancestors()
        .nth(3)
        .expect("OUT_DIR is below the profile directory");
    for (name, source) in [
        (
            "conpty.dll",
            format!("runtimes/win-{arch}/native/conpty.dll"),
        ),
        (
            "OpenConsole.exe",
            format!("build/native/runtimes/{arch}/OpenConsole.exe"),
        ),
    ] {
        let source = package.join(source);
        let target = bin.join(name);
        println!("cargo:rerun-if-changed={}", target.display());
        let bytes =
            fs::read(&source).map_err(|error| format!("reading {}: {error}", source.display()))?;
        if fs::read(&target).is_ok_and(|current| current == bytes) {
            continue;
        }
        // A copy keeps the cached file's time, so Cargo does not rerun this for it.
        fs::copy(&source, &target).map_err(|error| {
            format!(
                "copying {} to {}: {error}; a Server started from {} still runs it, so stop \
                 that Server first",
                source.display(),
                target.display(),
                bin.display()
            )
        })?;
    }
    Ok(())
}

/// The unpacked package, downloaded and checked on first use.
fn conpty_package() -> Result<PathBuf, String> {
    let cache = dirs::cache_dir()
        .ok_or("this machine has no cache directory to keep the ConPTY package in")?
        .join("condr")
        .join("conpty")
        .join(CONPTY_VERSION);
    let package = cache.join("package");
    if package.join("runtimes").is_dir() {
        return Ok(package);
    }
    fs::create_dir_all(&cache).map_err(|error| format!("creating {}: {error}", cache.display()))?;

    // Each build downloads and unpacks under its own names and renames the tree into
    // place, so a build that races this one never sees half of it.
    let download = cache.join(format!("download-{}.nupkg", process::id()));
    let staging = cache.join(format!("staging-{}", process::id()));
    let url = format!(
        "https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/\
         {CONPTY_VERSION}/microsoft.windows.console.conpty.{CONPTY_VERSION}.nupkg"
    );
    let unpacked = (|| {
        run(Command::new("curl")
            .args(["-fsSL", "--retry", "3", "-o"])
            .arg(&download)
            .arg(&url))?;
        let bytes = fs::read(&download).map_err(|error| format!("reading {url}: {error}"))?;
        let found: String = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if found != CONPTY_SHA256 {
            return Err(format!(
                "the ConPTY package's SHA-256 is {found}, not the {CONPTY_SHA256} build.rs pins"
            ));
        }
        fs::create_dir_all(&staging)
            .map_err(|error| format!("creating {}: {error}", staging.display()))?;
        // Windows' own bsdtar reads the package's zip; a Git Bash `tar` first on PATH would not.
        let system =
            env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        run(Command::new(system.join(r"System32\tar.exe"))
            .arg("-xf")
            .arg(&download)
            .arg("-C")
            .arg(&staging))?;
        if fs::rename(&staging, &package).is_err() && !package.join("runtimes").is_dir() {
            return Err(format!(
                "moving {} to {}",
                staging.display(),
                package.display()
            ));
        }
        Ok(())
    })();
    let _ = fs::remove_file(&download);
    let _ = fs::remove_dir_all(&staging);
    unpacked.map(|()| package)
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
