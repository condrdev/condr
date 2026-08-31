use std::net::SocketAddr;
use std::path::PathBuf;

use condr_server::{Endpoint, ServerConfig};

const USAGE: &str = "\
usage:
  condr-server start [--endpoint PATH | --listen ADDR] [--snapshot PATH]
  condr-server status [--endpoint PATH | --listen ADDR]
  condr-server stop [--endpoint PATH | --listen ADDR]
  condr-server run [--endpoint PATH | --listen ADDR] [--snapshot PATH]";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ServerCommand {
    Start,
    Status,
    Stop,
    Run,
}

#[derive(Debug, Eq, PartialEq)]
struct Arguments {
    command: ServerCommand,
    endpoint: Endpoint,
    snapshot_path: Option<PathBuf>,
    detached: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(
        args.as_slice(),
        [arg] if matches!(arg.as_str(), "help" | "--help" | "-h")
    ) || matches!(
        args.as_slice(),
        [_, arg] if matches!(arg.as_str(), "--help" | "-h")
    ) {
        println!("{USAGE}");
        return;
    }
    let arguments = match parse_args(&args) {
        Ok(arguments) => arguments,
        Err(error) => {
            eprintln!("condr-server: {error}");
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    std::process::exit(dispatch(arguments));
}

fn dispatch(arguments: Arguments) -> i32 {
    let Arguments {
        command,
        endpoint,
        snapshot_path,
        detached,
    } = arguments;
    match command {
        ServerCommand::Start => {
            match condr_server::ensure_server(server_config(endpoint, snapshot_path)) {
                Ok(endpoint) => {
                    println!("condr-server: running at {}", endpoint_text(&endpoint));
                    0
                }
                Err(error) => {
                    eprintln!("condr-server: failed to start: {error}");
                    1
                }
            }
        }
        ServerCommand::Status => match condr_server::probe_server(&endpoint) {
            Ok(()) => {
                println!("condr-server: running at {}", endpoint_text(&endpoint));
                0
            }
            Err(error) => {
                println!("condr-server: not running ({error})");
                1
            }
        },
        ServerCommand::Stop => match condr_server::stop_server(&endpoint) {
            Ok(()) => {
                println!("condr-server: stopping");
                0
            }
            Err(error) => {
                eprintln!("condr-server: failed to stop: {error}");
                1
            }
        },
        ServerCommand::Run => {
            if detached {
                #[cfg(unix)]
                if let Err(error) = nix::unistd::setsid() {
                    eprintln!("condr-server: failed to detach from the parent session: {error}");
                    return 1;
                }
                eprintln!(
                    "condr-server: detached process {} starting",
                    std::process::id()
                );
            }
            match condr_server::run(server_config(endpoint, snapshot_path)) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("condr-server: {error}");
                    1
                }
            }
        }
    }
}

fn server_config(endpoint: Endpoint, snapshot_path: Option<PathBuf>) -> ServerConfig {
    let mut config = ServerConfig::new(endpoint);
    if let Some(path) = snapshot_path {
        config = config.with_snapshot_path(path);
    }
    config
}

fn parse_args(args: &[String]) -> Result<Arguments, String> {
    let command = match args.first().map(String::as_str) {
        Some("start") => ServerCommand::Start,
        Some("status") => ServerCommand::Status,
        Some("stop") => ServerCommand::Stop,
        Some("run") => ServerCommand::Run,
        Some(unknown) => return Err(format!("unknown command {unknown}")),
        None => return Err("missing command".into()),
    };
    let mut endpoint = None;
    let mut snapshot_path = None;
    let mut detached = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--endpoint" => {
                if endpoint.is_some() {
                    return Err("--endpoint and --listen may only be specified once".into());
                }
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--endpoint requires a path".to_string())?;
                endpoint = Some(Endpoint::local(PathBuf::from(value)));
                index += 2;
            }
            "--listen" => {
                if endpoint.is_some() {
                    return Err("--endpoint and --listen may only be specified once".into());
                }
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--listen requires an address".to_string())?;
                let address: SocketAddr = value
                    .parse()
                    .map_err(|error| format!("invalid listen address {value}: {error}"))?;
                endpoint = Some(Endpoint::tcp(address));
                index += 2;
            }
            "--snapshot" => {
                if snapshot_path.is_some() {
                    return Err("--snapshot may only be specified once".into());
                }
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--snapshot requires a path".to_string())?;
                snapshot_path = Some(PathBuf::from(value));
                index += 2;
            }
            "--detached" => {
                detached = true;
                index += 1;
            }
            unknown => return Err(format!("unknown argument {unknown}")),
        }
    }
    let endpoint = endpoint.unwrap_or_else(|| ServerConfig::default().endpoint);
    if matches!(command, ServerCommand::Status | ServerCommand::Stop) && snapshot_path.is_some() {
        return Err("--snapshot is only valid with start or run".into());
    }
    if command != ServerCommand::Run && detached {
        return Err("--detached is only valid with run".into());
    }
    if command == ServerCommand::Start
        && matches!(&endpoint, Endpoint::Tcp(address) if address.port() == 0)
    {
        return Err("start requires a non-zero TCP port".into());
    }
    Ok(Arguments {
        command,
        endpoint,
        snapshot_path,
        detached,
    })
}

fn endpoint_text(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Local(path) => path.display().to_string(),
        Endpoint::Tcp(address) => address.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).into()).collect()
    }

    #[test]
    fn lifecycle_commands_parse_their_supported_options() {
        let start = parse_args(&strings(&[
            "start",
            "--listen",
            "127.0.0.1:4242",
            "--snapshot",
            "state.snapshot",
        ]))
        .unwrap();
        assert_eq!(start.command, ServerCommand::Start);
        assert_eq!(
            start.endpoint,
            Endpoint::tcp("127.0.0.1:4242".parse().unwrap())
        );
        assert_eq!(start.snapshot_path, Some("state.snapshot".into()));

        let run = parse_args(&strings(&["run", "--endpoint", "test.sock", "--detached"])).unwrap();
        assert_eq!(run.command, ServerCommand::Run);
        assert_eq!(run.endpoint, Endpoint::local("test.sock"));
        assert!(run.detached);

        assert_eq!(
            parse_args(&strings(&["status"])).unwrap().command,
            ServerCommand::Status
        );
        assert_eq!(
            parse_args(&strings(&["stop"])).unwrap().command,
            ServerCommand::Stop
        );
    }

    #[test]
    fn invalid_command_option_combinations_are_rejected() {
        assert_eq!(parse_args(&[]).unwrap_err(), "missing command");
        assert_eq!(
            parse_args(&strings(&["launch"])).unwrap_err(),
            "unknown command launch"
        );
        assert_eq!(
            parse_args(&strings(&[
                "run",
                "--endpoint",
                "test.sock",
                "--listen",
                "127.0.0.1:4242",
            ]))
            .unwrap_err(),
            "--endpoint and --listen may only be specified once"
        );
        assert_eq!(
            parse_args(&strings(&["status", "--snapshot", "state.snapshot"])).unwrap_err(),
            "--snapshot is only valid with start or run"
        );
        assert_eq!(
            parse_args(&strings(&["start", "--detached"])).unwrap_err(),
            "--detached is only valid with run"
        );
        assert_eq!(
            parse_args(&strings(&["start", "--listen", "127.0.0.1:0"])).unwrap_err(),
            "start requires a non-zero TCP port"
        );
    }
}
