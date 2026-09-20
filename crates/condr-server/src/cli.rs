//! `condr workspace …`, `condr tab …`, and `condr agent …`: how a program in a Pane
//! changes or coordinates the Session it lives in. Shaped after herdr's CLI: JSON on
//! stdout, a JSON error on stderr with exit 1, and usage errors from clap with exit 2.
//! Agent targets are numeric Pane ids or names assigned by `agent start`.

mod agent;
mod device;
mod pane;
mod workspace;

use clap::{Subcommand, ValueEnum};
use condr_core::protocol::{LayoutCommand, LayoutResult};
use condr_core::{
    AgentKind, AgentState, PaneDirection, PaneEnvironment, PaneId, PaneLayout, Session,
    SplitDirection, Tab, TabId, TerminalCommand, TerminalKey, TerminalModifiers, Workspace,
    WorkspaceId,
};
use condr_server::{ClientConnection, Endpoint, SavedServer, default_socket_path};
use pane::pane_info;
use serde::Serialize;
use serde_json::{Value, json};
use std::io;
use std::path::{Path, PathBuf};
use workspace::find_workspace;

pub(crate) use agent::{AgentCommand, run_agent};
pub(crate) use device::{DeviceCommand, run_device};
pub(crate) use pane::{PaneCommand, run_pane};
pub(crate) use workspace::{TabCommand, WorkspaceCommand, run_tab, run_workspace};

/// What stderr gets: `{"error":{"code":…,"message":…}}`.
#[derive(Debug, Serialize)]
struct CliError {
    code: String,
    message: String,
}

impl CliError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    fn workspace_not_found(id: u64) -> Self {
        Self::new("workspace_not_found", format!("workspace {id} not found"))
    }
}

impl From<io::Error> for CliError {
    fn from(error: io::Error) -> Self {
        Self::new("io", error.to_string())
    }
}

/// Which Server a command talks to. `--device` (or `CONDR_DEVICE`) names one of the
/// `[[client.servers]]` Devices saved in `config.toml`; otherwise `CONDR_SOCKET_PATH`
/// names the Server the Pane belongs to, and outside a Pane the CLI talks to the
/// machine's default local Server.
fn target_endpoint(device: Option<&str>) -> Result<Endpoint, CliError> {
    if let Some(name) = device {
        return device::device_endpoint(&device::saved_devices()?, name);
    }
    Ok(match std::env::var(PaneEnvironment::SOCKET_PATH) {
        Ok(value) => Endpoint::from_env_value(&value)?,
        Err(_) => Endpoint::local(default_socket_path()),
    })
}

fn connect(device: Option<&str>) -> Result<ClientConnection, CliError> {
    connect_to(&target_endpoint(device)?)
}

fn connect_to(endpoint: &Endpoint) -> Result<ClientConnection, CliError> {
    ClientConnection::connect_overview(endpoint, "condr-cli").map_err(|error| match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => CliError::new(
            "server_not_running",
            endpoint.describe_connect_error(&error),
        ),
        io::ErrorKind::PermissionDenied => {
            CliError::new("not_authorized", endpoint.describe_connect_error(&error))
        }
        _ => error.into(),
    })
}

fn apply(client: &mut ClientConnection, command: LayoutCommand) -> Result<LayoutResult, CliError> {
    client
        .layout(command)?
        .map_err(|reason| CliError::new("rejected", reason))
}

/// Runs a command against the target Server's Session and prints its JSON; the
/// process exit code.
fn run(
    device: Option<&str>,
    command: impl FnOnce(&mut ClientConnection) -> Result<Value, CliError>,
) -> i32 {
    match connect(device).and_then(|mut client| command(&mut client)) {
        Ok(value) => {
            println!("{value}");
            0
        }
        Err(error) => {
            eprintln!("{}", json!({ "error": error }));
            1
        }
    }
}
