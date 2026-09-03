//! `condr.com`: the console-subsystem shim next to `condr.exe` on Windows.
//!
//! An interactive PowerShell or cmd does not wait for a Windows-subsystem process, so
//! `condr --version` typed there would print after the prompt returned and lose its
//! exit code. `PATHEXT` ranks `.com` above `.exe`, so a typed `condr` resolves to this
//! shim, which runs `condr.exe` with the same arguments and inherited stdio, waits and
//! forwards the exit code. Double-clicks and shortcuts still open `condr.exe` directly.
//! Cargo names the output `condr-shim.exe`; packaging renames it to `condr.com`.

#[cfg(windows)]
fn main() {
    let exe = match std::env::current_exe() {
        Ok(path) => path.with_file_name("condr.exe"),
        Err(error) => exit_with(&format!("cannot locate condr.exe: {error}")),
    };
    let mut command = std::process::Command::new(&exe);
    command.args(std::env::args_os().skip(1));
    // A bare `condr` opens the GUI; return the prompt instead of waiting for it to close.
    if std::env::args_os().len() == 1 {
        match command.spawn() {
            Ok(_) => return,
            Err(error) => exit_with(&format!("cannot start {}: {error}", exe.display())),
        }
    }
    match command.status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(error) => exit_with(&format!("cannot start {}: {error}", exe.display())),
    }
}

#[cfg(windows)]
fn exit_with(message: &str) -> ! {
    eprintln!("condr: {message}");
    std::process::exit(1)
}

// Cargo cannot restrict a bin target to one OS; elsewhere this is an empty program.
#[cfg(not(windows))]
fn main() {}
