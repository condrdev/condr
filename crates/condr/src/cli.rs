//! The `condr` command-line surface: anything but the bare GUI launch. Subcommands land
//! here as GitHub #27 progresses; today only `--help` and `--version` exist.
//!
//! One binary serves the GUI and the CLI on every platform. On Linux and macOS that is
//! free. On Windows the binary is a Windows-subsystem executable so double-clicking never
//! opens a console; CLI mode attaches to the parent console before clap prints anything,
//! and `condr.com` (see `shim.rs`) makes interactive shells wait for it.

use clap::Parser;

/// Condr: a multi-agent terminal. Without arguments this opens the GUI.
#[derive(Parser)]
#[command(name = "condr", bin_name = "condr", version)]
struct Cli {}

pub(crate) fn run(args: Vec<String>) -> ! {
    attach_parent_console();
    // Only called with arguments, and no argument is accepted yet, so clap prints help,
    // the version or a usage error and exits here.
    let Cli {} = Cli::parse_from(std::iter::once("condr".to_owned()).chain(args));
    std::process::exit(0)
}

/// Joins the parent process's console so stdout/stderr reach the shell that started
/// us (Alacritty and Neovide do the same). Fails silently without a parent console.
#[cfg(windows)]
#[allow(unsafe_code)]
fn attach_parent_console() {
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    // SAFETY: a documented Win32 call with no pointer arguments; failure only means
    // there is no parent console, which every caller tolerates.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

#[cfg(not(windows))]
fn attach_parent_console() {}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    #[test]
    fn help_and_version_are_handled_by_clap_and_anything_else_is_a_usage_error() {
        let kind = |args: &[&str]| {
            Cli::try_parse_from(std::iter::once("condr").chain(args.iter().copied()))
                .err()
                .map(|error| error.kind())
        };
        assert_eq!(kind(&["--help"]), Some(ErrorKind::DisplayHelp));
        assert_eq!(kind(&["--version"]), Some(ErrorKind::DisplayVersion));
        assert_eq!(kind(&["frobnicate"]), Some(ErrorKind::UnknownArgument));
        assert_eq!(kind(&["pane", "list"]), Some(ErrorKind::UnknownArgument));
    }
}
