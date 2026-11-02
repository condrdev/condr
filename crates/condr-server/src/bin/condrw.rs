//! `condrw.exe` runs the `condr.exe` beside it with the same arguments and no console
//! window, and exits with its exit code. Windows gives a console program its window
//! before any of its code runs, so Autostart's login item starts this instead (ADR 0057).

#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    #[cfg(windows)]
    std::process::exit(run());
    #[cfg(not(windows))]
    {
        eprintln!("condrw only exists on Windows; run condr");
        std::process::exit(2);
    }
}

#[cfg(windows)]
fn run() -> i32 {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(launcher) = std::env::current_exe() else {
        return 1;
    };
    std::process::Command::new(launcher.with_file_name("condr.exe"))
        .args(std::env::args_os().skip(1))
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_or(1, |status| status.code().unwrap_or(1))
}
