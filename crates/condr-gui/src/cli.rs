//! The `condr` command-line surface: anything but the bare GUI launch. Plain
//! `println!` on a console-subsystem binary, as herdr does; subcommands land here as
//! GitHub #27 progresses. Today only `--help` and `--version` exist.

const USAGE: &str = "\
usage:
  condr               launch the GUI (returns at once; the GUI runs detached)
  condr --foreground  run the GUI in this console, for development
  condr --version     print the version
  condr --help        show this help";

pub(crate) fn run(args: Vec<String>) -> ! {
    std::process::exit(dispatch(&args));
}

fn dispatch(args: &[String]) -> i32 {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["help" | "--help" | "-h"] => {
            println!("{USAGE}");
            0
        }
        ["--version" | "-V" | "version"] => {
            println!("condr {}", env!("CARGO_PKG_VERSION"));
            0
        }
        _ => {
            eprintln!("condr: unknown command: {}", words.join(" "));
            eprintln!("{USAGE}");
            2
        }
    }
}

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
