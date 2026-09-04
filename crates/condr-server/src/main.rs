use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use condr_server::noise::{self, ServerIdentity};
use condr_server::{Endpoint, ServerConfig, TcpEndpoint};

mod cli;

/// Condr: the Server and its command line. The Server owns sessions, terminals and
/// agents; the GUI and the CLI subcommands are its clients.
#[derive(Parser)]
#[command(name = "condr", bin_name = "condr", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Manage the Server on this machine
    #[command(subcommand)]
    Server(ServerCommand),
    /// Workspaces of the Server this Pane belongs to
    #[command(subcommand)]
    Workspace(cli::WorkspaceCommand),
    /// Tabs of the Server this Pane belongs to
    #[command(subcommand)]
    Tab(cli::TabCommand),
    /// Panes of the Server this Pane belongs to: split, read and type into terminals
    #[command(subcommand)]
    Pane(cli::PaneCommand),
}

#[derive(Subcommand)]
enum ServerCommand {
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
    /// Print a one-time invite that pairs a new device with this host's TCP Server
    Invite,
    /// List the devices paired with this host's TCP Server
    Clients,
    /// Revoke a paired device by its key, or a unique prefix of it, and drop its live
    /// connections to the Server at the endpoint
    Revoke {
        #[command(flatten)]
        endpoint: EndpointArgs,
        #[arg(value_name = "KEY")]
        key: String,
    },
}

/// `--endpoint` and `--listen` are alternatives; neither means the platform default.
#[derive(Args)]
#[group(multiple = false)]
struct EndpointArgs {
    /// Local socket or named pipe path
    #[arg(long, value_name = "PATH")]
    endpoint: Option<PathBuf>,
    /// TCP address to listen on; every TCP connection authenticates with this host's keys
    #[arg(long, value_name = "ADDR")]
    listen: Option<SocketAddr>,
}

impl EndpointArgs {
    fn resolve(self) -> io::Result<Endpoint> {
        match (self.endpoint, self.listen) {
            (Some(path), _) => Ok(Endpoint::local(path)),
            (None, Some(address)) => tcp_endpoint(address),
            (None, None) => Ok(ServerConfig::default().endpoint),
        }
    }
}

/// The TCP endpoint of this host's Server as its own CLI connects to it.
fn tcp_endpoint(address: SocketAddr) -> io::Result<Endpoint> {
    let directory = noise::identity_directory()?;
    let identity = ServerIdentity::load_or_create(&directory)?;
    Ok(Endpoint::tcp(TcpEndpoint {
        address,
        server_key: identity.public_key(),
        client_key: noise::host_client_key(&directory)?,
        invite: None,
    }))
}

fn main() {
    std::process::exit(match Cli::parse().command {
        Command::Server(command) => dispatch(command),
        Command::Workspace(command) => cli::run_workspace(command),
        Command::Tab(command) => cli::run_tab(command),
        Command::Pane(command) => cli::run_pane(command),
    });
}

fn dispatch(command: ServerCommand) -> i32 {
    match run_server_command(command) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("condr-server: {error}");
            1
        }
    }
}

fn run_server_command(command: ServerCommand) -> io::Result<()> {
    match command {
        ServerCommand::Start { endpoint, snapshot } => {
            let endpoint = endpoint.resolve()?;
            let endpoint = condr_server::ensure_server(server_config(endpoint, snapshot)).map_err(
                |error| io::Error::new(error.kind(), format!("failed to start: {error}")),
            )?;
            println!("condr-server: running at {endpoint}");
            if let Endpoint::Tcp(tcp) = &endpoint {
                println!("condr-server: Server key {}", tcp.server_key);
                println!("condr-server: pair another device with `condr server invite`");
            }
            Ok(())
        }
        ServerCommand::Status { endpoint } => {
            let endpoint = endpoint.resolve()?;
            match condr_server::probe_server(&endpoint) {
                Ok(()) => {
                    println!("condr-server: running at {endpoint}");
                    Ok(())
                }
                Err(error) => {
                    println!("condr-server: not running ({error})");
                    Err(io::Error::new(io::ErrorKind::NotConnected, "not running"))
                }
            }
        }
        ServerCommand::Stop { endpoint } => {
            condr_server::stop_server(&endpoint.resolve()?).map_err(|error| {
                io::Error::new(error.kind(), format!("failed to stop: {error}"))
            })?;
            println!("condr-server: stopping");
            Ok(())
        }
        ServerCommand::Run {
            endpoint,
            snapshot,
            detached,
        } => {
            let endpoint = endpoint.resolve()?;
            if detached {
                #[cfg(unix)]
                nix::unistd::setsid().map_err(|error| {
                    io::Error::other(format!("failed to detach from the parent session: {error}"))
                })?;
                eprintln!(
                    "condr-server: detached process {} starting",
                    std::process::id()
                );
            }
            condr_server::run(server_config(endpoint, snapshot))
        }
        ServerCommand::Invite => {
            let directory = noise::identity_directory()?;
            let identity = ServerIdentity::load_or_create(&directory)?;
            let invite = noise::create_invite(&directory)?;
            println!(
                "condr-server: invite valid for {} minutes; in Condr, Add Server with",
                noise::INVITE_TTL.as_secs() / 60
            );
            println!(
                "  {}.{}@<host>:<port>",
                identity.public_key(),
                invite.secret.to_hex()
            );
            Ok(())
        }
        ServerCommand::Clients => {
            let directory = noise::identity_directory()?;
            let clients = noise::read_authorized(&directory)?;
            if clients.is_empty() {
                println!("condr-server: no paired devices");
            }
            for client in clients {
                println!("{} {} {}", client.key, client.paired_at, client.name);
            }
            Ok(())
        }
        ServerCommand::Revoke { endpoint, key } => {
            let directory = noise::identity_directory()?;
            let removed = noise::revoke(&directory, &key)?;
            if removed == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no paired device matches {key}"),
                ));
            }
            println!("condr-server: revoked {removed} device(s)");
            // The file already refuses their next handshake; a running Server also drops
            // the connections they hold now.
            match condr_server::revoke_devices(&endpoint.resolve()?, &key) {
                Ok(disconnected) => {
                    println!("condr-server: closed {disconnected} live connection(s)");
                    Ok(())
                }
                // The file edit stands, but nobody closed the device's live connections;
                // a script must not read that as a complete revocation.
                Err(error) => Err(io::Error::new(
                    error.kind(),
                    format!(
                        "revoked in authorized-clients, but no running Server was reached to \
                         drop live connections: {error}"
                    ),
                )),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<ServerCommand, clap::Error> {
        Cli::try_parse_from(["condr", "server"].into_iter().chain(args.iter().copied())).map(
            |cli| match cli.command {
                Command::Server(command) => command,
                _ => panic!("expected a server command"),
            },
        )
    }

    #[test]
    fn lifecycle_commands_parse_their_supported_options() {
        let ServerCommand::Start { endpoint, snapshot } = parse(&[
            "start",
            "--listen",
            "127.0.0.1:4242",
            "--snapshot",
            "state.snapshot",
        ])
        .unwrap() else {
            panic!("expected start");
        };
        assert_eq!(endpoint.listen, Some("127.0.0.1:4242".parse().unwrap()));
        assert_eq!(endpoint.endpoint, None);
        assert_eq!(snapshot, Some("state.snapshot".into()));

        let ServerCommand::Run {
            endpoint, detached, ..
        } = parse(&["run", "--endpoint", "test.sock", "--detached"]).unwrap()
        else {
            panic!("expected run");
        };
        assert_eq!(endpoint.resolve().unwrap(), Endpoint::local("test.sock"));
        assert!(detached);

        let ServerCommand::Status { endpoint } = parse(&["status"]).unwrap() else {
            panic!("expected status");
        };
        assert_eq!(
            endpoint.resolve().unwrap(),
            ServerConfig::default().endpoint
        );
        assert!(matches!(parse(&["stop"]), Ok(ServerCommand::Stop { .. })));
        assert!(matches!(parse(&["invite"]), Ok(ServerCommand::Invite)));
        assert!(matches!(parse(&["clients"]), Ok(ServerCommand::Clients)));
        assert!(matches!(
            parse(&["revoke", "abcd"]),
            Ok(ServerCommand::Revoke { key, .. }) if key == "abcd"
        ));
    }

    #[test]
    fn invalid_command_option_combinations_are_rejected() {
        assert!(parse(&[]).is_err());
        assert!(parse(&["launch"]).is_err());
        // The lifecycle verbs only exist under `server`.
        assert!(Cli::try_parse_from(["condr", "start"]).is_err());
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
        assert!(parse(&["revoke"]).is_err());
    }
}
