//! `condr workspace …` and `condr tab …`: how a program in a Pane changes the Session it
//! lives in. Shaped after herdr's CLI: the same command groups and verbs, JSON on stdout,
//! a JSON error on stderr with exit 1, usage errors from clap with exit 2. Targets are
//! the numeric ids the protocol and `CONDR_PANE_ID` already use.

use std::collections::HashSet;
use std::io;
use std::path::PathBuf;

use clap::Subcommand;
use condr_core::protocol::LayoutCommand;
use condr_core::{PaneEnvironment, PaneId, Session, Tab, TabId, Workspace, WorkspaceId};
use condr_server::{ClientConnection, Endpoint, default_socket_path};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Subcommand)]
pub(crate) enum WorkspaceCommand {
    /// List every Workspace
    List,
    /// Create a Workspace; its first Tab opens a shell in the root directory
    Create {
        /// Root directory; defaults to the current directory
        #[arg(long, value_name = "PATH")]
        cwd: Option<PathBuf>,
        /// Name shown in the sidebar; defaults to the directory name
        #[arg(long, value_name = "TEXT")]
        label: Option<String>,
        /// Switch the GUI to the new Workspace instead of leaving it in the background
        #[arg(long)]
        focus: bool,
    },
    /// Show one Workspace
    Get { workspace_id: u64 },
    /// Make a Workspace the active one
    Focus { workspace_id: u64 },
    /// Rename a Workspace
    Rename { workspace_id: u64, label: String },
    /// Close a Workspace and stop its terminals
    Close { workspace_id: u64 },
}

#[derive(Subcommand)]
pub(crate) enum TabCommand {
    /// List Tabs, in one Workspace or across all of them
    List {
        #[arg(long, value_name = "ID")]
        workspace: Option<u64>,
    },
    /// Create a Tab; defaults to the calling Pane's Workspace, else the active one
    Create {
        #[arg(long, value_name = "ID")]
        workspace: Option<u64>,
        /// Name shown on the Tab; defaults to "Tab N"
        #[arg(long, value_name = "TEXT")]
        label: Option<String>,
        /// Switch the GUI to the new Tab instead of leaving it in the background
        #[arg(long)]
        focus: bool,
    },
    /// Show one Tab
    Get { tab_id: u64 },
    /// Make a Tab the active one in its Workspace
    Focus { tab_id: u64 },
    /// Rename a Tab
    Rename { tab_id: u64, label: String },
    /// Close a Tab and stop its terminals; the last Tab closes its Workspace
    Close { tab_id: u64 },
}

/// What stderr gets: `{"error":{"code":…,"message":…}}`.
#[derive(Serialize)]
struct CliError {
    code: &'static str,
    message: String,
}

impl CliError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
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

#[derive(Serialize)]
struct WorkspaceInfo {
    workspace_id: u64,
    name: String,
    root_directory: PathBuf,
    focused: bool,
    tab_count: usize,
    pane_count: usize,
    active_tab_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    worktree: Option<WorktreeInfo>,
}

#[derive(Serialize)]
struct WorktreeInfo {
    parent_workspace_id: u64,
    parent_root_directory: PathBuf,
    managed: bool,
}

#[derive(Serialize)]
struct TabInfo {
    tab_id: u64,
    workspace_id: u64,
    name: String,
    /// The active Tab of its Workspace.
    focused: bool,
    pane_count: usize,
    focused_pane_id: u64,
}

#[derive(Serialize)]
struct PaneInfo {
    pane_id: u64,
}

fn workspace_info(session: &Session, workspace: &Workspace) -> WorkspaceInfo {
    WorkspaceInfo {
        workspace_id: workspace.id().as_u64(),
        name: workspace.name().to_owned(),
        root_directory: workspace.root_directory().to_path_buf(),
        focused: session.active_workspace_id() == Some(workspace.id()),
        tab_count: workspace.tabs().len(),
        pane_count: workspace.tabs().iter().map(|tab| tab.panes().len()).sum(),
        active_tab_id: workspace.active_tab().id().as_u64(),
        worktree: workspace.worktree().map(|worktree| WorktreeInfo {
            parent_workspace_id: worktree.parent_workspace_id().as_u64(),
            parent_root_directory: worktree.parent_root_directory().to_path_buf(),
            managed: worktree.is_managed(),
        }),
    }
}

fn tab_info(workspace: &Workspace, tab: &Tab) -> TabInfo {
    TabInfo {
        tab_id: tab.id().as_u64(),
        workspace_id: workspace.id().as_u64(),
        name: tab.name().to_owned(),
        focused: workspace.active_tab().id() == tab.id(),
        pane_count: tab.panes().len(),
        focused_pane_id: tab.focused_pane().id().as_u64(),
    }
}

fn root_pane(tab: &Tab) -> PaneInfo {
    PaneInfo {
        pane_id: tab.focused_pane().id().as_u64(),
    }
}

fn find_workspace(session: &Session, id: u64) -> Result<&Workspace, CliError> {
    session
        .workspace(WorkspaceId::from_u64(id))
        .ok_or_else(|| CliError::workspace_not_found(id))
}

fn find_tab(session: &Session, id: u64) -> Result<(&Workspace, &Tab), CliError> {
    let tab_id = TabId::from_u64(id);
    session
        .workspaces()
        .iter()
        .find_map(|workspace| {
            workspace
                .tabs()
                .iter()
                .find(|tab| tab.id() == tab_id)
                .map(|tab| (workspace, tab))
        })
        .ok_or_else(|| CliError::new("tab_not_found", format!("tab {id} not found")))
}

/// The Workspace of the Pane this process runs in, when `CONDR_PANE_ID` says so.
fn caller_workspace(session: &Session) -> Option<WorkspaceId> {
    let pane_id = std::env::var(PaneEnvironment::PANE_ID).ok()?.parse().ok()?;
    session
        .workspace_for_pane(PaneId::from_u64(pane_id))
        .map(Workspace::id)
}

/// `CONDR_SOCKET_PATH` names the Server the Pane belongs to; outside a Pane the CLI
/// talks to the machine's default local Server.
fn connect() -> Result<ClientConnection, CliError> {
    let endpoint = match std::env::var(PaneEnvironment::SOCKET_PATH) {
        Ok(value) => Endpoint::from_env_value(&value)?,
        Err(_) => Endpoint::local(default_socket_path()),
    };
    ClientConnection::connect(&endpoint, "condr-cli").map_err(|error| match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => CliError::new(
            "server_not_running",
            format!("no Server at {}: {error}", endpoint.env_value()),
        ),
        _ => error.into(),
    })
}

fn apply(client: &mut ClientConnection, command: LayoutCommand) -> Result<(), CliError> {
    client
        .layout(command)?
        .map(drop)
        .map_err(|reason| CliError::new("rejected", reason))
}

/// Runs a command against the Session and prints its JSON; the process exit code.
fn run(command: impl FnOnce(&mut ClientConnection) -> Result<Value, CliError>) -> i32 {
    match connect().and_then(|mut client| command(&mut client)) {
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

pub(crate) fn run_workspace(command: WorkspaceCommand) -> i32 {
    run(|client| workspace(client, command))
}

pub(crate) fn run_tab(command: TabCommand) -> i32 {
    run(|client| tab(client, command))
}

fn workspace(client: &mut ClientConnection, command: WorkspaceCommand) -> Result<Value, CliError> {
    match command {
        WorkspaceCommand::List => {
            let session = client.session()?;
            let workspaces: Vec<_> = session
                .workspaces()
                .iter()
                .map(|workspace| workspace_info(&session, workspace))
                .collect();
            Ok(json!({ "workspaces": workspaces }))
        }
        WorkspaceCommand::Create { cwd, label, focus } => {
            let root_directory = std::path::absolute(match cwd {
                Some(cwd) => cwd,
                None => std::env::current_dir()?,
            })?;
            let before = client.session()?;
            let previously_active = before.active_workspace_id();
            let known: HashSet<WorkspaceId> =
                before.workspaces().iter().map(Workspace::id).collect();
            apply(client, LayoutCommand::CreateWorkspace { root_directory })?;
            client.refresh()?;
            let workspace_id = client
                .session()?
                .workspaces()
                .iter()
                .map(Workspace::id)
                .find(|id| !known.contains(id))
                .ok_or_else(|| {
                    CliError::new(
                        "workspace_create_failed",
                        "the Server applied the command but reports no new Workspace",
                    )
                })?;
            if let Some(name) = label {
                apply(
                    client,
                    LayoutCommand::RenameWorkspace { workspace_id, name },
                )?;
            }
            // Creation activates the new Workspace; without --focus the GUI stays put.
            if !focus && let Some(previous) = previously_active {
                apply(
                    client,
                    LayoutCommand::ActivateWorkspace {
                        workspace_id: previous,
                    },
                )?;
            }
            client.refresh()?;
            let session = client.session()?;
            let workspace = find_workspace(&session, workspace_id.as_u64())?;
            let tab = workspace.active_tab();
            Ok(json!({
                "workspace": workspace_info(&session, workspace),
                "tab": tab_info(workspace, tab),
                "root_pane": root_pane(tab),
            }))
        }
        WorkspaceCommand::Get { workspace_id } => {
            let session = client.session()?;
            let workspace = find_workspace(&session, workspace_id)?;
            Ok(json!({ "workspace": workspace_info(&session, workspace) }))
        }
        WorkspaceCommand::Focus { workspace_id } => {
            find_workspace(&client.session()?, workspace_id)?;
            apply(
                client,
                LayoutCommand::ActivateWorkspace {
                    workspace_id: WorkspaceId::from_u64(workspace_id),
                },
            )?;
            client.refresh()?;
            let session = client.session()?;
            let workspace = find_workspace(&session, workspace_id)?;
            Ok(json!({ "workspace": workspace_info(&session, workspace) }))
        }
        WorkspaceCommand::Rename {
            workspace_id,
            label,
        } => {
            find_workspace(&client.session()?, workspace_id)?;
            apply(
                client,
                LayoutCommand::RenameWorkspace {
                    workspace_id: WorkspaceId::from_u64(workspace_id),
                    name: label,
                },
            )?;
            client.refresh()?;
            let session = client.session()?;
            let workspace = find_workspace(&session, workspace_id)?;
            Ok(json!({ "workspace": workspace_info(&session, workspace) }))
        }
        WorkspaceCommand::Close { workspace_id } => {
            find_workspace(&client.session()?, workspace_id)?;
            apply(
                client,
                LayoutCommand::CloseWorkspace {
                    workspace_id: WorkspaceId::from_u64(workspace_id),
                },
            )?;
            Ok(json!({ "ok": true }))
        }
    }
}

fn tab(client: &mut ClientConnection, command: TabCommand) -> Result<Value, CliError> {
    match command {
        TabCommand::List { workspace } => {
            let session = client.session()?;
            let workspaces: Vec<&Workspace> = match workspace {
                Some(id) => vec![find_workspace(&session, id)?],
                None => session.workspaces().iter().collect(),
            };
            let tabs: Vec<_> = workspaces
                .iter()
                .flat_map(|workspace| {
                    workspace
                        .tabs()
                        .iter()
                        .map(move |tab| tab_info(workspace, tab))
                })
                .collect();
            Ok(json!({ "tabs": tabs }))
        }
        TabCommand::Create {
            workspace,
            label,
            focus,
        } => {
            let before = client.session()?;
            let workspace_id = match workspace {
                Some(id) => find_workspace(&before, id)?.id(),
                None => caller_workspace(&before)
                    .or(before.active_workspace_id())
                    .ok_or_else(|| CliError::new("workspace_not_found", "no active workspace"))?,
            };
            let target = find_workspace(&before, workspace_id.as_u64())?;
            let previously_active_workspace = before.active_workspace_id();
            let previously_active_tab = target.active_tab().id();
            let known: HashSet<TabId> = target.tabs().iter().map(Tab::id).collect();
            apply(client, LayoutCommand::CreateTab { workspace_id })?;
            client.refresh()?;
            let tab_id = find_workspace(&client.session()?, workspace_id.as_u64())?
                .tabs()
                .iter()
                .map(Tab::id)
                .find(|id| !known.contains(id))
                .ok_or_else(|| {
                    CliError::new(
                        "tab_create_failed",
                        "the Server applied the command but reports no new Tab",
                    )
                })?;
            if let Some(name) = label {
                apply(client, LayoutCommand::RenameTab { tab_id, name })?;
            }
            // Creation activates the new Tab and its Workspace; without --focus both go
            // back to where the GUI was.
            if !focus {
                apply(
                    client,
                    LayoutCommand::ActivateTab {
                        tab_id: previously_active_tab,
                    },
                )?;
                if let Some(previous) = previously_active_workspace
                    && previous != workspace_id
                {
                    apply(
                        client,
                        LayoutCommand::ActivateWorkspace {
                            workspace_id: previous,
                        },
                    )?;
                }
            }
            client.refresh()?;
            let session = client.session()?;
            let (workspace, tab) = find_tab(&session, tab_id.as_u64())?;
            Ok(json!({
                "tab": tab_info(workspace, tab),
                "root_pane": root_pane(tab),
            }))
        }
        TabCommand::Get { tab_id } => {
            let session = client.session()?;
            let (workspace, tab) = find_tab(&session, tab_id)?;
            Ok(json!({ "tab": tab_info(workspace, tab) }))
        }
        TabCommand::Focus { tab_id } => {
            find_tab(&client.session()?, tab_id)?;
            apply(
                client,
                LayoutCommand::ActivateTab {
                    tab_id: TabId::from_u64(tab_id),
                },
            )?;
            client.refresh()?;
            let session = client.session()?;
            let (workspace, tab) = find_tab(&session, tab_id)?;
            Ok(json!({ "tab": tab_info(workspace, tab) }))
        }
        TabCommand::Rename { tab_id, label } => {
            find_tab(&client.session()?, tab_id)?;
            apply(
                client,
                LayoutCommand::RenameTab {
                    tab_id: TabId::from_u64(tab_id),
                    name: label,
                },
            )?;
            client.refresh()?;
            let session = client.session()?;
            let (workspace, tab) = find_tab(&session, tab_id)?;
            Ok(json!({ "tab": tab_info(workspace, tab) }))
        }
        TabCommand::Close { tab_id } => {
            find_tab(&client.session()?, tab_id)?;
            apply(
                client,
                LayoutCommand::CloseTab {
                    tab_id: TabId::from_u64(tab_id),
                },
            )?;
            Ok(json!({ "ok": true }))
        }
    }
}
