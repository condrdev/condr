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
        Ok(code) => code,
        Err(error) => {
            eprintln!("condr-server: {error}");
            1
        }
    }
}

/// A connection failure in words: a missing socket file or a refused TCP connect both
/// mean nobody is listening, and the raw OS error hides which endpoint was tried.
fn connect_problem(endpoint: &Endpoint, error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => {
            let hint = match endpoint {
                Endpoint::Local(_) => "; a TCP Server needs --listen <addr>",
                Endpoint::Tcp(_) => "",
            };
            format!("no Server is listening at {endpoint}{hint}")
        }
        io::ErrorKind::PermissionDenied => format!("{endpoint} refused this device: {error}"),
        _ => format!("{endpoint}: {error}"),
    }
}

/// Runs one `condr server …` command; `Ok` carries the process exit code.
fn run_server_command(command: ServerCommand) -> io::Result<i32> {
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
            Ok(0)
        }
        ServerCommand::Status { endpoint } => {
            let endpoint = endpoint.resolve()?;
            match condr_server::probe_server(&endpoint) {
                Ok(()) => {
                    println!("condr-server: running at {endpoint}");
                    Ok(0)
                }
                Err(error) => {
                    println!(
                        "condr-server: not running: {}",
                        connect_problem(&endpoint, &error)
                    );
                    Ok(1)
                }
            }
        }
        ServerCommand::Stop { endpoint } => {
            let endpoint = endpoint.resolve()?;
            condr_server::stop_server(&endpoint).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("failed to stop: {}", connect_problem(&endpoint, &error)),
                )
            })?;
            println!("condr-server: stopping");
            Ok(0)
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
            condr_server::run(server_config(endpoint, snapshot)).map(|()| 0)
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
            Ok(0)
        }
        ServerCommand::Clients => {
            let directory = noise::identity_directory()?;
            let clients = noise::read_authorized(&directory)?;
            if clients.is_empty() {
                println!("condr-server: no paired devices");
                return Ok(0);
            }
            let name_width = clients
                .iter()
                .map(|client| client.name.chars().count())
                .max()
                .unwrap_or(0)
                .max("NAME".len());
            println!("{:<name_width$}  {:<14}  KEY", "NAME", "PAIRED");
            for client in clients {
                println!(
                    "{:<name_width$}  {:<14}  {}",
                    client.name,
                    ago(client.paired_at),
                    client.key
                );
            }
            println!("revoke a device with `condr server revoke <key or prefix>`");
            Ok(0)
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
            let endpoint = endpoint.resolve()?;
            match condr_server::revoke_devices(&endpoint, &key) {
                Ok(disconnected) => {
                    println!("condr-server: closed {disconnected} live connection(s)");
                    Ok(0)
                }
                // The file edit stands, but nobody closed the device's live connections;
                // a script must not read that as a complete revocation.
                Err(error) => Err(io::Error::new(
                    error.kind(),
                    format!(
                        "live connections were not dropped: {}",
                        connect_problem(&endpoint, &error)
                    ),
                )),
            }
        }
    }
}

/// "3 hours ago" for a Unix timestamp; coarse on purpose, a device list is not a log.
fn ago(unix_seconds: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let elapsed = now.saturating_sub(unix_seconds);
    let (amount, unit) = match elapsed {
        0..60 => return "just now".into(),
        60..3_600 => (elapsed / 60, "minute"),
        3_600..86_400 => (elapsed / 3_600, "hour"),
        _ => (elapsed / 86_400, "day"),
    };
    format!("{amount} {unit}{} ago", if amount == 1 { "" } else { "s" })
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
    fn pairing_age_reads_as_a_coarse_relative_time() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(ago(now), "just now");
        assert_eq!(ago(now - 60), "1 minute ago");
        assert_eq!(ago(now - 5 * 60), "5 minutes ago");
        assert_eq!(ago(now - 3 * 3_600), "3 hours ago");
        assert_eq!(ago(now - 86_400), "1 day ago");
        assert_eq!(ago(now - 40 * 86_400), "40 days ago");
        assert_eq!(ago(now + 100), "just now", "a clock skew is not the future");
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
