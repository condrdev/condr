use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use condr_server::{Endpoint, ServerConfig};

/// The Condr server: owns sessions, terminals and agents; the GUI is one of its clients.
#[derive(Parser)]
#[command(name = "condr-server", bin_name = "condr-server", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start a detached server unless one already answers at the endpoint
    Start {
        #[command(flatten)]
        endpoint: EndpointArgs,
        /// Where the session snapshot is persisted
        #[arg(long, value_name = "PATH")]
        snapshot: Option<PathBuf>,
    },
    /// Report whether a server answers at the endpoint
    Status {
        #[command(flatten)]
        endpoint: EndpointArgs,
    },
    /// Ask the server at the endpoint to shut down
    Stop {
        #[command(flatten)]
        endpoint: EndpointArgs,
    },
    /// Run the server in the foreground
    Run {
        #[command(flatten)]
        endpoint: EndpointArgs,
        /// Where the session snapshot is persisted
        #[arg(long, value_name = "PATH")]
        snapshot: Option<PathBuf>,
        /// Leave the parent session first; `start` uses this for the server it spawns
        #[arg(long)]
        detached: bool,
    },
}

/// `--endpoint` and `--listen` are alternatives; neither means the platform default.
#[derive(Args)]
#[group(multiple = false)]
struct EndpointArgs {
    /// Local socket or named pipe path
    #[arg(long, value_name = "PATH")]
    endpoint: Option<PathBuf>,
    /// TCP address to listen on, normally loopback behind an SSH tunnel
    #[arg(long, value_name = "ADDR")]
    listen: Option<SocketAddr>,
}

impl EndpointArgs {
    fn resolve(self) -> Endpoint {
        match (self.endpoint, self.listen) {
            (Some(path), _) => Endpoint::local(path),
            (None, Some(address)) => Endpoint::tcp(address),
            (None, None) => ServerConfig::default().endpoint,
        }
    }
}

fn main() {
    std::process::exit(dispatch(Cli::parse().command));
}

fn dispatch(command: Command) -> i32 {
    match command {
        Command::Start { endpoint, snapshot } => {
            match condr_server::ensure_server(server_config(endpoint.resolve(), snapshot)) {
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
        Command::Status { endpoint } => {
            let endpoint = endpoint.resolve();
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
        Command::Stop { endpoint } => match condr_server::stop_server(&endpoint.resolve()) {
            Ok(()) => {
                println!("condr-server: stopping");
                0
            }
            Err(error) => {
                eprintln!("condr-server: failed to stop: {error}");
                1
            }
        },
        Command::Run {
            endpoint,
            snapshot,
            detached,
        } => {
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
            match condr_server::run(server_config(endpoint.resolve(), snapshot)) {
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

fn endpoint_text(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Local(path) => path.display().to_string(),
        Endpoint::Tcp(address) => address.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, clap::Error> {
        Cli::try_parse_from(std::iter::once("condr-server").chain(args.iter().copied()))
            .map(|cli| cli.command)
    }

    #[test]
    fn lifecycle_commands_parse_their_supported_options() {
        let Command::Start { endpoint, snapshot } = parse(&[
            "start",
            "--listen",
            "127.0.0.1:4242",
            "--snapshot",
            "state.snapshot",
        ])
        .unwrap() else {
            panic!("expected start");
        };
        assert_eq!(
            endpoint.resolve(),
            Endpoint::tcp("127.0.0.1:4242".parse().unwrap())
        );
        assert_eq!(snapshot, Some("state.snapshot".into()));

        let Command::Run {
            endpoint, detached, ..
        } = parse(&["run", "--endpoint", "test.sock", "--detached"]).unwrap()
        else {
            panic!("expected run");
        };
        assert_eq!(endpoint.resolve(), Endpoint::local("test.sock"));
        assert!(detached);

        let Command::Status { endpoint } = parse(&["status"]).unwrap() else {
            panic!("expected status");
        };
        assert_eq!(endpoint.resolve(), ServerConfig::default().endpoint);
        assert!(matches!(parse(&["stop"]), Ok(Command::Stop { .. })));
    }

    #[test]
    fn invalid_command_option_combinations_are_rejected() {
        assert!(parse(&[]).is_err());
        assert!(parse(&["launch"]).is_err());
        assert!(
            parse(&[
                "run",
                "--endpoint",
                "test.sock",
                "--listen",
                "127.0.0.1:4242"
            ])
            .is_err()
        );
        assert!(parse(&["status", "--snapshot", "state.snapshot"]).is_err());
        assert!(parse(&["start", "--detached"]).is_err());
        assert!(parse(&["start", "--listen", "not-an-address"]).is_err());
    }
}
