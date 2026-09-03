//! The `condr` command-line surface: anything but the bare GUI launch. Subcommands land
//! here as GitHub #27 progresses; today only `--help` and `--version` exist.
//!
//! One binary serves the GUI and the CLI on every platform. On Linux and macOS that is
//! free. On Windows the binary is a Windows-subsystem executable so double-clicking never
//! opens a console, and CLI mode attaches to the parent shell's console before printing.

const USAGE: &str = "\
usage:
  condr              launch the GUI
  condr --version    print the version
  condr --help       show this help";

pub(crate) fn run(args: Vec<String>) -> ! {
    attach_parent_console();
    std::process::exit(dispatch(&args));
}

fn dispatch(args: &[String]) -> i32 {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["help" | "--help" | "-h"] => {
            print_line(&mut std::io::stdout(), USAGE);
            0
        }
        ["--version" | "-V" | "version"] => {
            print_line(
                &mut std::io::stdout(),
                &format!("condr {}", env!("CARGO_PKG_VERSION")),
            );
            0
        }
        _ => {
            let mut stderr = std::io::stderr();
            print_line(
                &mut stderr,
                &format!("condr: unknown command: {}", words.join(" ")),
            );
            print_line(&mut stderr, USAGE);
            2
        }
    }
}

/// Writes one line and swallows failures: started with arguments from Explorer there
/// is no console to attach to, and `println!` would panic instead of exiting quietly.
fn print_line(out: &mut impl std::io::Write, line: &str) {
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// Joins the parent process's console so stdout/stderr reach the shell that started
/// us (Alacritty and Neovide do the same). Fails silently without a parent console.
// ponytail: cmd and an interactive PowerShell do not wait for a Windows-subsystem
// process, so their prompt can return before the output and `$LASTEXITCODE` is
// unreliable there. Git Bash, scripts and agents capturing output wait normally. The
// alternatives (a console-subsystem binary, or `consoleAllocationPolicy=detached`)
// flash a console on double-click or need GPUI's embedded manifest changed.
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

    #[test]
    fn help_and_version_succeed_and_unknown_commands_exit_with_usage_status() {
        assert_eq!(dispatch(&["--help".into()]), 0);
        assert_eq!(dispatch(&["help".into()]), 0);
        assert_eq!(dispatch(&["--version".into()]), 0);
        assert_eq!(dispatch(&["frobnicate".into()]), 2);
        assert_eq!(dispatch(&["pane".into(), "list".into()]), 2);
    }
}
