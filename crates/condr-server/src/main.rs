use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use condr_server::ServerConfig;
use condr_server::noise::{self, ServerIdentity};

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
    /// Manage this machine's Server
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

/// One Server per machine. It always answers on its private local socket; `--listen`
/// adds a TCP address for other devices and remembers it in `config.toml`.
#[derive(Subcommand)]
enum ServerCommand {
    /// Start the Server detached unless it is already running
    Start {
        /// Also listen on this TCP address from now on (saved to config.toml)
        #[arg(long, value_name = "ADDR")]
        listen: Option<SocketAddr>,
        /// Where the session snapshot is persisted
        #[arg(long, value_name = "PATH")]
        snapshot: Option<PathBuf>,
    },
    /// Report whether the Server is running
    Status,
    /// Ask the Server to shut down
    Stop,
    /// Run the Server in the foreground
    Run {
        /// Local socket or named pipe path; defaults to the platform path
        #[arg(long, value_name = "PATH")]
        endpoint: Option<PathBuf>,
        /// Also listen on this TCP address, overriding config.toml for this run
        #[arg(long, value_name = "ADDR")]
        listen: Option<SocketAddr>,
        /// Where the session snapshot is persisted
        #[arg(long, value_name = "PATH")]
        snapshot: Option<PathBuf>,
        /// Leave the parent session first; `start` uses this for the server it spawns
        #[arg(long)]
        detached: bool,
    },
    /// Print a one-time invite that pairs a new device over TCP
    Invite,
    /// List the devices paired over TCP
    Clients,
    /// Revoke a paired device by its key, or a unique prefix of it, and drop its live
    /// connections
    Revoke {
        #[arg(value_name = "KEY")]
        key: String,
    },
}

fn main() {
    std::process::exit(match Cli::parse().command {
        Command::Server(command) => dispatch(command),
        Command::Workspace(command) => cli::run_workspace(command),
        Command::Tab(command) => cli::run_tab(command),
        Command::Pane(command) => cli::run_pane(command),
    });
}

/// Every failure prints as a headline followed by indented detail lines:
///
/// ```text
/// condr-server: failed to stop
///   no Server is listening at /run/user/1000/condr/condr.sock
/// ```
fn dispatch(command: ServerCommand) -> i32 {
    match run_server_command(command) {
        Ok(code) => code,
        Err(error) => {
            report("condr-server", &error.to_string());
            1
        }
    }
}

/// Prints `headline` prefixed with `prefix`, then each following line indented.
fn report(prefix: &str, text: &str) {
    for (index, line) in text.lines().enumerate() {
        if index == 0 {
            eprintln!("{prefix}: {line}");
        } else {
            eprintln!("  {line}");
        }
    }
}

/// An error whose message is a headline plus detail lines, as [`report`] prints them.
fn failure(headline: impl Into<String>, details: impl IntoIterator<Item = String>) -> io::Error {
    let mut text = headline.into();
    for detail in details {
        text.push('\n');
        text.push_str(&detail);
    }
    io::Error::other(text)
}

/// Runs one `condr server …` command; `Ok` carries the process exit code.
fn run_server_command(command: ServerCommand) -> io::Result<i32> {
    match command {
        ServerCommand::Start { listen, snapshot } => {
            let mut config = ServerConfig::default();
            let already_running = condr_server::probe_server(&config.local_endpoint()).is_ok();
            if let Some(address) = listen {
                let path = config_path()?;
                condr_server::save_listen(&path, Some(address)).map_err(|error| {
                    failure(
                        format!("could not save the listen address to {}", path.display()),
                        [error.to_string()],
                    )
                })?;
                config.listen = Some(address);
            }
            if let Some(path) = snapshot {
                config = config.with_snapshot_path(path);
            }
            let endpoint = condr_server::ensure_server(config.clone())
                .map_err(|error| failure("failed to start the Server", [error.to_string()]))?;
            println!("condr-server: running at {endpoint}");
            match (config.listen, already_running) {
                (Some(address), false) => {
                    println!("condr-server: also listening at tcp://{address}");
                    println!("condr-server: pair another device with `condr server invite`");
                }
                (Some(address), true) => {
                    println!(
                        "condr-server: the running Server keeps its previous TCP listener; \
                         restart it to listen at tcp://{address}"
                    );
                }
                (None, _) => {}
            }
            Ok(0)
        }
        ServerCommand::Status => {
            let config = ServerConfig::default();
            let endpoint = config.local_endpoint();
            match condr_server::probe_server(&endpoint) {
                Ok(()) => {
                    // First, so it lines up with the Fingerprint field in the GUI's Edit
                    // Server dialog for a side-by-side check.
                    let identity = load_identity(&identity_directory()?)?;
                    println!("condr-server: fingerprint {}", identity.public_key());
                    println!("condr-server: running at {endpoint}");
                    if let Some(address) = config.listen {
                        println!("condr-server: configured to listen at tcp://{address}");
                    }
                    Ok(0)
                }
                Err(error) => {
                    report(
                        "condr-server",
                        &format!("not running\n{}", endpoint.describe_connect_error(&error)),
                    );
                    Ok(1)
                }
            }
        }
        ServerCommand::Stop => {
            let endpoint = ServerConfig::default().local_endpoint();
            condr_server::stop_server(&endpoint).map_err(|error| {
                failure("failed to stop", [endpoint.describe_connect_error(&error)])
            })?;
            println!("condr-server: stopping");
            Ok(0)
        }
        ServerCommand::Run {
            endpoint,
            listen,
            snapshot,
            detached,
        } => {
            let mut config = match endpoint {
                Some(path) => ServerConfig::at_socket(path),
                None => ServerConfig::default(),
            };
            if let Some(address) = listen {
                config.listen = Some(address);
            }
            if let Some(path) = snapshot {
                config = config.with_snapshot_path(path);
            }
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
            condr_server::run(config)
                .map(|()| 0)
                .map_err(|error| failure("the Server stopped with an error", [error.to_string()]))
        }
        ServerCommand::Invite => {
            let directory = identity_directory()?;
            let identity = load_identity(&directory)?;
            let invite = noise::create_invite(&directory)?;
            let port = ServerConfig::default()
                .listen
                .map_or_else(|| "<port>".to_owned(), |address| address.port().to_string());
            println!(
                "condr-server: invite valid for {} minutes; in Condr, Add Server with",
                noise::INVITE_TTL.as_secs() / 60
            );
            println!(
                "  {}.{}@<host>:{port}",
                identity.public_key(),
                invite.secret.to_hex()
            );
            if port == "<port>" {
                println!(
                    "condr-server: no TCP listener is configured yet; start one with \
                     `condr server start --listen <addr>`"
                );
            }
            Ok(0)
        }
        ServerCommand::Clients => {
            let directory = identity_directory()?;
            let clients = noise::read_authorized(&directory)?;
            if clients.is_empty() {
                println!("condr-server: no paired devices");
                return Ok(0);
            }
            // A Server that is not running has no live connections; every device then
            // shows when it was last seen.
            let connected =
                condr_server::connected_devices(&ServerConfig::default().local_endpoint())
                    .unwrap_or_default();
            let name_width = clients
                .iter()
                .map(|client| client.name.chars().count())
                .max()
                .unwrap_or(0)
                .max("NAME".len());
            println!("{:<name_width$}  {:<14}  KEY", "NAME", "LAST SEEN");
            for client in clients {
                let seen = if connected.contains(&client.key.to_hex()) {
                    "connected".to_owned()
                } else {
                    ago(client.last_seen)
                };
                println!("{:<name_width$}  {seen:<14}  {}", client.name, client.key);
            }
            println!("revoke a device with `condr server revoke <key or prefix>`");
            Ok(0)
        }
        ServerCommand::Revoke { key } => {
            let directory = identity_directory()?;
            let removed = noise::revoke(&directory, &key)?;
            if removed == 0 {
                return Err(failure(
                    format!("no paired device matches {key}"),
                    ["list paired devices with `condr server clients`".to_owned()],
                ));
            }
            println!("condr-server: revoked {removed} device(s)");
            // The file already refuses their next handshake; a running Server also drops
            // the connections they hold now.
            let endpoint = ServerConfig::default().local_endpoint();
            match condr_server::revoke_devices(&endpoint, &key) {
                Ok(disconnected) => {
                    println!("condr-server: closed {disconnected} live connection(s)");
                    Ok(0)
                }
                // The file edit stands, but nobody closed the device's live connections;
                // a script must not read that as a complete revocation.
                Err(error) => Err(failure(
                    "revoked in authorized-clients, but its live connections were not dropped",
                    [endpoint.describe_connect_error(&error)],
                )),
            }
        }
    }
}

fn config_path() -> io::Result<PathBuf> {
    identity_directory().map(|directory| directory.join("config.toml"))
}

fn identity_directory() -> io::Result<PathBuf> {
    noise::identity_directory().map_err(|error| {
        failure(
            "no directory to keep this host's keys in",
            [
                error.to_string(),
                "set CONDR_CONFIG_DIR to a private directory".to_owned(),
            ],
        )
    })
}

fn load_identity(directory: &std::path::Path) -> io::Result<ServerIdentity> {
    ServerIdentity::load_or_create(directory).map_err(|error| {
        failure(
            format!(
                "could not load the Server identity in {}",
                directory.display()
            ),
            [error.to_string()],
        )
    })
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
        let ServerCommand::Start { listen, snapshot } = parse(&[
            "start",
            "--listen",
            "127.0.0.1:4242",
            "--snapshot",
            "state.snapshot",
        ])
        .unwrap() else {
            panic!("expected start");
        };
        assert_eq!(listen, Some("127.0.0.1:4242".parse().unwrap()));
        assert_eq!(snapshot, Some("state.snapshot".into()));

        let ServerCommand::Run {
            endpoint,
            listen,
            detached,
            ..
        } = parse(&["run", "--endpoint", "test.sock", "--detached"]).unwrap()
        else {
            panic!("expected run");
        };
        assert_eq!(endpoint, Some("test.sock".into()));
        assert_eq!(listen, None);
        assert!(detached);

        assert!(matches!(parse(&["status"]), Ok(ServerCommand::Status)));
        assert!(matches!(parse(&["stop"]), Ok(ServerCommand::Stop)));
        assert!(matches!(parse(&["invite"]), Ok(ServerCommand::Invite)));
        assert!(matches!(parse(&["clients"]), Ok(ServerCommand::Clients)));
        assert!(matches!(
            parse(&["revoke", "abcd"]),
            Ok(ServerCommand::Revoke { key }) if key == "abcd"
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
        // Only `run` takes an explicit socket path; the others use the host's Server.
        assert!(parse(&["start", "--endpoint", "test.sock"]).is_err());
        assert!(parse(&["status", "--listen", "127.0.0.1:4242"]).is_err());
        assert!(parse(&["stop", "--snapshot", "state.snapshot"]).is_err());
        assert!(parse(&["start", "--detached"]).is_err());
        assert!(parse(&["start", "--listen", "not-an-address"]).is_err());
        assert!(parse(&["revoke"]).is_err());
    }
}
