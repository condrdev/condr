//! The `condr` command-line surface: everything except the bare GUI launch.

use condr_server::{Endpoint, ServerConfig, default_socket_path};

const USAGE: &str = "\
usage:
  condr                  launch the GUI
  condr server start     start the local server if it is not already running
  condr server status    report whether the local server is running
  condr server stop      stop the local server
  condr server run       run a local server in the foreground";

pub(crate) fn run(args: Vec<String>) -> ! {
    attach_parent_console();
    std::process::exit(dispatch(&args));
}

fn dispatch(args: &[String]) -> i32 {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["help" | "--help" | "-h"] => {
            println!("{USAGE}");
            0
        }
        ["server", "start"] => match condr_server::ensure_local_server() {
            Ok(endpoint) => {
                println!("condr-server: running at {}", endpoint_text(&endpoint));
                0
            }
            Err(error) => {
                eprintln!("condr: failed to start condr-server: {error}");
                1
            }
        },
        ["server", "status"] => {
            let endpoint = Endpoint::local(default_socket_path());
            match condr_server::probe_server(&endpoint) {
                Ok(()) => {
                    println!("condr-server: running at {}", endpoint_text(&endpoint));
                    0
                }
                Err(error) => {
                    println!("condr-server: not running ({error})");
                    1
                }
            }
        }
        ["server", "stop"] => {
            match condr_server::stop_server(&Endpoint::local(default_socket_path())) {
                Ok(()) => {
                    println!("condr-server: stopping");
                    0
                }
                Err(error) => {
                    eprintln!("condr: failed to stop condr-server: {error}");
                    1
                }
            }
        }
        ["server", "run"] => match condr_server::run(ServerConfig::default()) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("condr: server exited with an error: {error}");
                1
            }
        },
        _ => {
            eprintln!("condr: unknown command: {}", words.join(" "));
            eprintln!("{USAGE}");
            2
        }
    }
}

fn endpoint_text(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Local(path) => path.display().to_string(),
        Endpoint::Tcp(address) => address.to_string(),
    }
}

/// `condr` 以 windows subsystem 链接以避免双击弹出控制台;CLI 模式下把
/// stdout/stderr 接回父进程的 console,从 Explorer 启动时静默失败。
// ponytail: shell 不会等待 GUI subsystem 进程,退出码和管道语义不完整;
// M2 CLI 需要稳定退出码时换 console shim(condr.com)或 24H2 consoleAllocationPolicy。
#[cfg(windows)]
#[allow(unsafe_code)]
fn attach_parent_console() {
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

#[cfg(not(windows))]
fn attach_parent_console() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_succeeds_and_unknown_commands_fail_with_usage_exit_code() {
        assert_eq!(dispatch(&["help".into()]), 0);
        assert_eq!(dispatch(&["--help".into()]), 0);
        assert_eq!(dispatch(&["frobnicate".into()]), 2);
        assert_eq!(dispatch(&["server".into(), "frobnicate".into()]), 2);
    }
}
