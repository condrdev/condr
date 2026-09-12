use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use condr_core::protocol::relative_age;
use condr_server::ServerConfig;
use condr_server::noise::{self, ServerIdentity};

mod cli;

/// The Condr Server and its command line.
///
/// `condr server` starts, stops and installs this machine's Server, which owns the
/// Workspaces, Tabs, Panes and agents. The other commands act on that Server; run
/// inside a Condr Pane they default to the Pane's own Workspace, Tab and Pane.
#[derive(Parser)]
#[command(
    name = "condr",
    bin_name = "condr",
    version,
    arg_required_else_help = true,
    args_conflicts_with_subcommands = true
)]
struct Cli {
    /// Print the bundled agent skill without connecting to a Server
    #[arg(long)]
    skill: bool,
    #[command(subcommand)]
    command: Option<Command>,
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
    /// Agents detected in Panes and their prompt lifecycle
    #[command(subcommand)]
    Agent(cli::AgentCommand),
    /// Installed into an agent CLI's hooks: reports one lifecycle event to the Pane's
    /// Server (ADR 0014). Inert outside a Condr Pane; always exits 0.
    #[command(hide = true)]
    AgentHook {
        #[arg(value_name = "AGENT")]
        agent: String,
        #[arg(value_name = "EVENT")]
        event: String,
    },
}

/// One Server per machine. It always answers on its private local socket; `--listen`
/// adds a TCP address for other devices and remembers it in `config.toml`.
#[derive(Subcommand)]
enum ServerCommand {
    /// Forward stdin/stdout to the running Server's private local socket (for SSH)
    Bridge {
        /// Local socket or named pipe path; defaults to the platform path
        #[arg(long, value_name = "PATH")]
        endpoint: Option<PathBuf>,
    },
    /// Start the Server detached unless it is already running
    Start {
        /// Also listen on this TCP address from now on (saved to config.toml)
        #[arg(long, value_name = "ADDR")]
        listen: Option<SocketAddr>,
        /// Where the session snapshot is persisted
        #[arg(long, value_name = "PATH")]
        snapshot: Option<PathBuf>,
    },
    /// Stop the Server, wait for shutdown, then start it detached
    Restart {
        /// Also listen on this TCP address from now on (saved to config.toml)
        #[arg(long, value_name = "ADDR")]
        listen: Option<SocketAddr>,
        /// Where the session snapshot is persisted
        #[arg(long, value_name = "PATH")]
        snapshot: Option<PathBuf>,
    },
    /// Report whether the Server is running
    Status,
    /// Copy this executable into the user's Condr directory and put it on PATH
    Install {
        /// Start the Server from the installed copy if none is running
        #[arg(long, conflicts_with = "restart")]
        start: bool,
        /// Stop the running Server and start it from the installed copy
        #[arg(long)]
        restart: bool,
        /// Confirm the restart without a terminal
        #[arg(long)]
        yes: bool,
        /// Print one JSON object instead of step lines
        #[arg(long)]
        json: bool,
    },
    /// Stop the Server, remove the installed `condr` and its PATH entry; keep your data
    Uninstall {
        /// Confirm stopping the Server without a terminal
        #[arg(long)]
        yes: bool,
        /// Print one JSON object instead of step lines
        #[arg(long)]
        json: bool,
    },
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
    /// Revoke a paired device by its fingerprint, or a unique prefix of it, and drop its
    /// live connections
    Revoke {
        #[arg(value_name = "FINGERPRINT")]
        key: String,
    },
}

fn main() {
    let cli = Cli::parse();
    if cli.skill {
        print!("{}", include_str!("../../../skills/condr/SKILL.md"));
        return;
    }
    std::process::exit(
        match cli.command.expect("subcommand required without --skill") {
            Command::Server(command) => dispatch(command),
            Command::Workspace(command) => cli::run_workspace(command),
            Command::Tab(command) => cli::run_tab(command),
            Command::Pane(command) => cli::run_pane(command),
            Command::Agent(command) => cli::run_agent(command),
            Command::AgentHook { agent, event } => {
                condr_core::agent_hook::run(&agent, &event);
                0
            }
        },
    );
}

/// Every failure prints as a headline followed by indented detail lines:
///
/// ```text
/// condr-server: failed to stop
///   nothing is listening at /run/user/1000/condr/condr.sock
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
    let restart = matches!(command, ServerCommand::Restart { .. });
    match command {
        ServerCommand::Bridge { endpoint } => {
            condr_server::ssh::bridge(&endpoint.unwrap_or_else(condr_server::default_socket_path))?;
            Ok(0)
        }
        ServerCommand::Start { listen, snapshot } | ServerCommand::Restart { listen, snapshot } => {
            // Shutdown terminates every Pane's process tree, including a restart CLI
            // running inside it, before that CLI could launch the replacement Server.
            if restart && std::env::var_os("CONDR_PANE_ID").is_some() {
                return Err(io::Error::other(
                    "run `condr server restart` from a terminal outside Condr; shutdown stops every Pane",
                ));
            }
            let mut config = ServerConfig::default();
            let already_running =
                !restart && condr_server::probe_server(&config.local_endpoint()).is_ok();
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
            let endpoint = if restart {
                condr_server::restart_server(config.clone())
                    .map_err(|error| failure("failed to restart the Server", [error.to_string()]))?
            } else {
                condr_server::ensure_server(config.clone())
                    .map_err(|error| failure("failed to start the Server", [error.to_string()]))?
            };
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
        ServerCommand::Install {
            start,
            restart,
            yes,
            json,
        } => condr_server::install::install(condr_server::install::InstallOptions {
            start,
            restart,
            yes,
            json,
        }),
        ServerCommand::Uninstall { yes, json } => {
            condr_server::install::uninstall(condr_server::install::UninstallOptions { yes, json })
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
                "condr-server: invite valid for {} minutes; in Condr, Connect Remote Device with",
                noise::INVITE_TTL.as_secs() / 60
            );
            println!(
                "  tcp://{}.{}@<host>:{port}",
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
            println!("{:<name_width$}  {:<14}  FINGERPRINT", "NAME", "LAST SEEN");
            for client in clients {
                let seen = if connected.contains(&client.key.to_hex()) {
                    "connected".to_owned()
                } else {
                    relative_age(client.last_seen)
                };
                println!("{:<name_width$}  {seen:<14}  {}", client.name, client.key);
            }
            println!("revoke a device with `condr server revoke <fingerprint or prefix>`");
            Ok(0)
        }
        ServerCommand::Revoke { key } => {
            let directory = identity_directory()?;
            let Some(key) = noise::revoke(&directory, &key)? else {
                return Err(failure(
                    format!("no paired device matches {key}"),
                    ["list paired devices with `condr server clients`".to_owned()],
                ));
            };
            println!("condr-server: revoked device {key}");
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
#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<ServerCommand, clap::Error> {
        Cli::try_parse_from(["condr", "server"].into_iter().chain(args.iter().copied())).map(
            |cli| match cli.command {
                Some(Command::Server(command)) => command,
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

        let ServerCommand::Restart { listen, snapshot } = parse(&[
            "restart",
            "--listen",
            "127.0.0.1:4243",
            "--snapshot",
            "restart.snapshot",
        ])
        .unwrap() else {
            panic!("expected restart");
        };
        assert_eq!(listen, Some("127.0.0.1:4243".parse().unwrap()));
        assert_eq!(snapshot, Some("restart.snapshot".into()));
        assert!(matches!(
            parse(&["restart"]),
            Ok(ServerCommand::Restart {
                listen: None,
                snapshot: None
            })
        ));

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
        assert!(matches!(
            parse(&["install", "--restart", "--yes", "--json"]),
            Ok(ServerCommand::Install {
                start: false,
                restart: true,
                yes: true,
                json: true
            })
        ));
        assert!(matches!(
            parse(&["install", "--start"]),
            Ok(ServerCommand::Install {
                start: true,
                restart: false,
                yes: false,
                json: false
            })
        ));
        assert!(matches!(
            parse(&["uninstall", "--yes", "--json"]),
            Ok(ServerCommand::Uninstall {
                yes: true,
                json: true
            })
        ));
        assert!(matches!(parse(&["stop"]), Ok(ServerCommand::Stop)));
        assert!(matches!(parse(&["invite"]), Ok(ServerCommand::Invite)));
        assert!(matches!(parse(&["clients"]), Ok(ServerCommand::Clients)));
        assert!(matches!(
            parse(&["revoke", "abcd"]),
            Ok(ServerCommand::Revoke { key }) if key == "abcd"
        ));
    }

    #[test]
    fn invalid_command_option_combinations_are_rejected() {
        assert!(parse(&[]).is_err());
        assert!(parse(&["launch"]).is_err());
        // The lifecycle verbs only exist under `server`.
        assert!(Cli::try_parse_from(["condr", "start"]).is_err());
        // Only `run` takes an explicit socket path; the others use the host's Server.
        assert!(parse(&["start", "--endpoint", "test.sock"]).is_err());
        assert!(parse(&["restart", "--endpoint", "test.sock"]).is_err());
        assert!(parse(&["restart", "--detached"]).is_err());
        assert!(parse(&["status", "--listen", "127.0.0.1:4242"]).is_err());
        assert!(parse(&["stop", "--snapshot", "state.snapshot"]).is_err());
        assert!(parse(&["start", "--detached"]).is_err());
        assert!(parse(&["start", "--listen", "not-an-address"]).is_err());
        assert!(parse(&["revoke"]).is_err());
        assert!(parse(&["install", "--start", "--restart"]).is_err());
        assert!(parse(&["uninstall", "--start"]).is_err());
        assert!(Cli::try_parse_from(["condr", "--skill", "server", "stop"]).is_err());
        assert!(Cli::try_parse_from(["condr", "server", "stop", "--skill"]).is_err());
    }
}
