//! `condr workspace …`, `condr tab …`, and `condr agent …`: how a program in a Pane
//! changes or coordinates the Session it lives in. Shaped after herdr's CLI: JSON on
//! stdout, a JSON error on stderr with exit 1, and usage errors from clap with exit 2.
//! Agent targets are numeric Pane ids or names assigned by `agent start`.

use std::io;
use std::path::{Path, PathBuf};

use clap::{Subcommand, ValueEnum};
use condr_core::protocol::{LayoutCommand, LayoutResult};
use condr_core::{
    AgentKind, AgentState, PaneDirection, PaneEnvironment, PaneId, PaneLayout, Session,
    SplitDirection, Tab, TabId, TerminalCommand, TerminalKey, TerminalModifiers, Workspace,
    WorkspaceId,
};
use condr_server::{ClientConnection, Endpoint, default_socket_path};
use serde::Serialize;
use serde_json::{Value, json};

/// How many rows `pane read` returns by default, as herdr does.
const DEFAULT_READ_LINES: u32 = 80;
const DEFAULT_AGENT_WAIT_MS: u64 = 120_000;

#[derive(Subcommand)]
pub(crate) enum PaneCommand {
    /// List Panes, in one Workspace or across all of them
    List {
        #[arg(long, value_name = "ID")]
        workspace: Option<u64>,
    },
    /// Show one Pane
    Get { pane_id: u64 },
    /// Show the Pane this command runs in (`CONDR_PANE_ID`)
    Current,
    /// Describe a Pane's Tab: the split tree, every Pane's edges, and the Pane's neighbors
    Layout {
        /// Defaults to the calling Pane
        pane_id: Option<u64>,
    },
    /// Split a Pane; the new Pane opens a shell in the same directory
    Split {
        /// Defaults to the calling Pane
        pane_id: Option<u64>,
        #[arg(long, value_enum)]
        direction: SplitSide,
        /// Focus the new Pane instead of leaving the GUI where it was
        #[arg(long)]
        focus: bool,
    },
    /// Focus a Pane by id, or the neighbor of a Pane with --direction
    Focus {
        /// Defaults to the calling Pane when --direction is given
        pane_id: Option<u64>,
        #[arg(long, value_enum)]
        direction: Option<Side>,
    },
    /// Move a Pane's edge; the amount is a fraction of the split
    Resize {
        /// Defaults to the calling Pane
        pane_id: Option<u64>,
        #[arg(long, value_enum)]
        direction: Side,
        #[arg(long, default_value_t = 0.05)]
        amount: f32,
    },
    /// Swap a Pane with its neighbor
    Swap {
        /// Defaults to the calling Pane
        pane_id: Option<u64>,
        #[arg(long, value_enum)]
        direction: Side,
    },
    /// Detach a Pane and reattach it beside another Pane in the same Tab
    Move {
        /// Defaults to the calling Pane
        pane_id: Option<u64>,
        /// The Pane to attach beside
        #[arg(long)]
        to: u64,
        /// Which side of --to the moved Pane lands on
        #[arg(long, value_enum)]
        side: Side,
    },
    /// Zoom a Pane to fill its Tab, or back; toggles unless --on or --off is given
    Zoom {
        /// Defaults to the calling Pane
        pane_id: Option<u64>,
        #[arg(long, conflicts_with = "off")]
        on: bool,
        #[arg(long)]
        off: bool,
    },
    /// Close a Pane and stop its terminal; the last Pane closes its Tab
    Close { pane_id: u64 },
    /// Print the last rows of a Pane's terminal as plain text, scrollback included
    Read {
        pane_id: u64,
        #[arg(long, value_name = "N", default_value_t = DEFAULT_READ_LINES)]
        lines: u32,
    },
    /// Type text into a Pane exactly as given, without Enter
    SendText { pane_id: u64, text: String },
    /// Press keys in a Pane: `enter`, `esc`, `tab`, `up`, `f5`, `ctrl+c`, `alt+shift+x`, `a`…
    SendKeys {
        pane_id: u64,
        #[arg(required = true)]
        keys: Vec<String>,
    },
    /// Paste a command into a Pane and press Enter
    Run { pane_id: u64, command: String },
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum SplitSide {
    /// The new Pane appears to the right
    Right,
    /// The new Pane appears below
    Down,
}

impl From<SplitSide> for SplitDirection {
    fn from(side: SplitSide) -> Self {
        match side {
            SplitSide::Right => Self::Horizontal,
            SplitSide::Down => Self::Vertical,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum Side {
    Left,
    Right,
    Up,
    Down,
}

impl From<Side> for PaneDirection {
    fn from(side: Side) -> Self {
        match side {
            Side::Left => Self::Left,
            Side::Right => Self::Right,
            Side::Up => Self::Up,
            Side::Down => Self::Down,
        }
    }
}

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

#[derive(Subcommand)]
pub(crate) enum AgentCommand {
    /// List native agent CLIs available on the Server's PATH
    Available,
    /// List agents currently detected in Panes
    List,
    /// Start a named agent in an existing idle shell and wait until it is ready
    Start {
        name: String,
        #[arg(long)]
        kind: String,
        #[arg(long)]
        pane: u64,
        #[arg(long, default_value_t = 30_000)]
        timeout: u64,
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Send a prompt to an agent; optionally wait for a settled state
    Prompt {
        target: String,
        text: String,
        #[arg(long)]
        wait: bool,
        #[arg(long = "until", requires = "wait")]
        until: Vec<String>,
        #[arg(long, requires = "wait")]
        timeout: Option<u64>,
    },
    /// Wait for an agent to reach a state
    Wait {
        target: String,
        #[arg(long = "until")]
        until: Vec<String>,
        #[arg(long, default_value_t = DEFAULT_AGENT_WAIT_MS)]
        timeout: u64,
    },
    /// Install, inspect or remove the hooks that report an agent's status to Condr; edits
    /// that agent's own configuration on this machine
    Hooks {
        #[arg(value_enum)]
        action: HooksAction,
        /// claude, codex or opencode
        agent: String,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub(crate) enum HooksAction {
    Install,
    Uninstall,
    Status,
}

/// What stderr gets: `{"error":{"code":…,"message":…}}`.
#[derive(Serialize)]
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
    ClientConnection::connect_overview(&endpoint, "condr-cli").map_err(|error| match error.kind() {
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

pub(crate) fn run_pane(command: PaneCommand) -> i32 {
    // `read` prints the text itself so agents can consume it without a JSON step.
    if let PaneCommand::Read { pane_id, lines } = command {
        return match connect().and_then(|mut client| {
            find_pane(&client.session()?, pane_id)?;
            Ok(client.read_pane(PaneId::from_u64(pane_id), lines)?)
        }) {
            Ok(text) => {
                if !text.is_empty() {
                    println!("{text}");
                }
                0
            }
            Err(error) => {
                eprintln!("{}", json!({ "error": error }));
                1
            }
        };
    }
    run(|client| pane(client, command))
}

pub(crate) fn run_agent(command: AgentCommand) -> i32 {
    if let AgentCommand::Hooks { action, agent } = command {
        return match agent_hooks(action, &agent) {
            Ok(value) => {
                println!("{value}");
                0
            }
            Err(error) => {
                eprintln!("{}", json!({ "error": error }));
                1
            }
        };
    }
    run(|client| agent(client, command))
}

/// Local only: the hooks live in the agent's configuration on this machine, and the
/// Server is not involved until an agent runs them.
fn agent_hooks(action: HooksAction, agent: &str) -> Result<Value, CliError> {
    use condr_core::agent_hooks as hooks;
    let kind = AgentKind::parse_label(agent).ok_or_else(|| {
        CliError::new("unknown_agent_kind", format!("unknown agent kind {agent}"))
    })?;
    let target = hooks::HookTarget::local()
        .ok_or_else(|| CliError::new("no_home_directory", "cannot locate the home directory"))?;
    let io = |error: io::Error| {
        CliError::new(
            match error.kind() {
                io::ErrorKind::InvalidData => "hooks_config_invalid",
                _ => "io",
            },
            error.to_string(),
        )
    };
    let mut warning = None;
    let path = match action {
        HooksAction::Install => {
            let path = hooks::install(&target, kind).map_err(io)?;
            if kind == AgentKind::Codex {
                // Codex ignores hooks.json until its hooks feature is on; the user can
                // also set `[features] hooks = true` in config.toml by hand.
                let enabled = std::process::Command::new(kind.executable())
                    .args(["features", "enable", "hooks"])
                    .stdin(std::process::Stdio::null())
                    .output();
                match enabled {
                    Ok(output) if output.status.success() => {}
                    Ok(output) => {
                        warning = Some(format!(
                            "codex features enable hooks failed ({}): {}; set [features] hooks = true in Codex's config.toml",
                            output.status,
                            String::from_utf8_lossy(&output.stderr).trim()
                        ));
                    }
                    Err(error) => {
                        warning = Some(format!(
                            "could not run codex to enable hooks: {error}; set [features] hooks = true in Codex's config.toml"
                        ));
                    }
                }
            }
            path
        }
        HooksAction::Uninstall => hooks::uninstall(&target, kind).map_err(io)?,
        HooksAction::Status => target.path(kind),
    };
    let state = hooks::state(&target, kind).map_err(io)?;
    let mut value = json!({
        "agent": kind.id(),
        "path": path,
        "state": state.name(),
    });
    if kind == AgentKind::Codex {
        value["note"] = Value::String(
            "Codex runs hooks only with the hooks feature enabled and, unless managed, after they are trusted from /hooks inside Codex".into(),
        );
    }
    if let Some(warning) = warning {
        value["warning"] = Value::String(warning);
    }
    Ok(value)
}

#[derive(Serialize)]
struct PaneInfo {
    pane_id: u64,
    tab_id: u64,
    workspace_id: u64,
    /// The focused Pane of its Tab.
    focused: bool,
    /// Fills its Tab, hiding the other Panes.
    zoomed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    exited: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent: Option<&'static str>,
    /// `idle`, `working`, `blocked`, or `unknown` for a plain shell.
    agent_status: &'static str,
}

fn pane_info(
    client: &ClientConnection,
    workspace: &Workspace,
    tab: &Tab,
    pane_id: PaneId,
) -> PaneInfo {
    let overview = client.overview();
    let terminal = overview
        .terminals
        .iter()
        .find(|terminal| terminal.pane_id == pane_id);
    let agent = overview
        .agents
        .iter()
        .find(|agent| agent.pane_id == pane_id)
        .map(|agent| agent.agent);
    PaneInfo {
        pane_id: pane_id.as_u64(),
        tab_id: tab.id().as_u64(),
        workspace_id: workspace.id().as_u64(),
        focused: tab.focused_pane().id() == pane_id,
        // Zoom is live state, not part of the structural Snapshot (ADR 0005).
        zoomed: overview.zoomed_panes.contains(&pane_id),
        cwd: tab
            .panes()
            .iter()
            .find(|pane| pane.id() == pane_id)
            .and_then(|pane| pane.cwd())
            .map(Path::to_path_buf),
        title: terminal.and_then(|terminal| terminal.title.clone()),
        exited: terminal.is_some_and(|terminal| terminal.exited),
        agent: agent.map(|agent| agent.kind.id()),
        agent_status: match agent.map(|agent| agent.state) {
            Some(AgentState::Idle) => "idle",
            Some(AgentState::Working) => "working",
            Some(AgentState::Blocked) => "blocked",
            Some(AgentState::Unknown) | None => "unknown",
        },
    }
}

fn find_pane(session: &Session, id: u64) -> Result<(&Workspace, &Tab), CliError> {
    let pane_id = PaneId::from_u64(id);
    session
        .workspaces()
        .iter()
        .find_map(|workspace| {
            workspace
                .tabs()
                .iter()
                .find(|tab| tab.panes().iter().any(|pane| pane.id() == pane_id))
                .map(|tab| (workspace, tab))
        })
        .ok_or_else(|| CliError::new("pane_not_found", format!("pane {id} not found")))
}

/// The Pane this process runs in, or a usage-style error when there is none.
fn caller_pane() -> Result<u64, CliError> {
    std::env::var(PaneEnvironment::PANE_ID)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .ok_or_else(|| {
            CliError::new(
                "no_current_pane",
                "CONDR_PANE_ID is not set: this is not a Condr Pane, pass a pane id",
            )
        })
}

/// `ctrl+shift+x`, `alt+enter`, `f5`, `esc`, `a`. Modifier names: ctrl/control, alt/opt,
/// shift, super/cmd/win.
fn parse_key(spec: &str) -> Result<TerminalCommand, CliError> {
    let invalid = || CliError::new("invalid_key", format!("unsupported key {spec}"));
    let mut modifiers = TerminalModifiers::default();
    let mut parts = spec.split('+').collect::<Vec<_>>();
    // A trailing "+" is the plus key itself.
    if spec.ends_with('+') {
        parts.pop();
        parts.pop();
        parts.push("+");
    }
    let key = parts.pop().ok_or_else(invalid)?;
    for modifier in parts {
        match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers.control = true,
            "alt" | "opt" | "option" | "meta" => modifiers.alt = true,
            "shift" => modifiers.shift = true,
            "super" | "cmd" | "win" => modifiers.platform = true,
            _ => return Err(invalid()),
        }
    }
    let key = match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => TerminalKey::Enter,
        "tab" => TerminalKey::Tab,
        "backtab" => TerminalKey::BackTab,
        "backspace" => TerminalKey::Backspace,
        "delete" | "del" => TerminalKey::Delete,
        "esc" | "escape" => TerminalKey::Escape,
        "up" => TerminalKey::Up,
        "down" => TerminalKey::Down,
        "left" => TerminalKey::Left,
        "right" => TerminalKey::Right,
        "home" => TerminalKey::Home,
        "end" => TerminalKey::End,
        "pageup" => TerminalKey::PageUp,
        "pagedown" => TerminalKey::PageDown,
        "insert" => TerminalKey::Insert,
        "space" => TerminalKey::Character(" ".into()),
        lower => {
            if let Some(number) = lower
                .strip_prefix('f')
                .and_then(|digits| digits.parse::<u8>().ok())
                .filter(|number| (1..=20).contains(number))
            {
                TerminalKey::Function(number)
            } else if key.chars().count() == 1 {
                TerminalKey::Character(key.into())
            } else {
                return Err(invalid());
            }
        }
    };
    Ok(TerminalCommand::Key { key, modifiers })
}

#[test]
fn function_keys_are_validated_within_the_terminal_encoders_range() {
    for number in 1..=20 {
        assert!(matches!(
            parse_key(&format!("ctrl+f{number}")),
            Ok(TerminalCommand::Key { key: TerminalKey::Function(n), .. }) if n == number
        ));
    }
    for number in [0, 21, 22, 23, 24, 255] {
        let keys = ["a".to_owned(), format!("f{number}")];
        assert!(
            keys.iter()
                .map(|key| parse_key(key))
                .collect::<Result<Vec<_>, _>>()
                .is_err()
        );
    }
}

fn pane(client: &mut ClientConnection, command: PaneCommand) -> Result<Value, CliError> {
    match command {
        PaneCommand::List { workspace } => {
            let client = &*client;
            let session = client.session()?;
            let workspaces: Vec<&Workspace> = match workspace {
                Some(id) => vec![find_workspace(&session, id)?],
                None => session.workspaces().iter().collect(),
            };
            let panes: Vec<_> = workspaces
                .iter()
                .flat_map(|workspace| {
                    workspace.tabs().iter().flat_map(move |tab| {
                        tab.panes()
                            .iter()
                            .map(move |pane| pane_info(client, workspace, tab, pane.id()))
                    })
                })
                .collect();
            Ok(json!({ "panes": panes }))
        }
        PaneCommand::Current => {
            let pane_id = caller_pane()?;
            let session = client.session()?;
            let (workspace, tab) = find_pane(&session, pane_id)?;
            Ok(json!({ "pane": pane_info(client, workspace, tab, PaneId::from_u64(pane_id)) }))
        }
        PaneCommand::Get { pane_id } => {
            let session = client.session()?;
            let (workspace, tab) = find_pane(&session, pane_id)?;
            Ok(json!({ "pane": pane_info(client, workspace, tab, PaneId::from_u64(pane_id)) }))
        }
        PaneCommand::Split {
            pane_id,
            direction,
            focus,
        } => {
            let pane_id = match pane_id {
                Some(id) => id,
                None => caller_pane()?,
            };
            find_pane(&client.session()?, pane_id)?;
            let LayoutResult::PaneCreated { pane_id: new_pane } = apply(
                client,
                LayoutCommand::SplitPane {
                    pane_id: PaneId::from_u64(pane_id),
                    direction: direction.into(),
                    focus,
                },
            )?
            else {
                return Err(CliError::new(
                    "pane_split_failed",
                    "the Server returned no created Pane ID",
                ));
            };
            let session = client.session()?;
            let (workspace, tab) = find_pane(&session, new_pane.as_u64())?;
            Ok(json!({ "pane": pane_info(client, workspace, tab, new_pane) }))
        }
        PaneCommand::Focus {
            pane_id,
            direction: None,
        } => {
            let pane_id = target_pane(pane_id)?;
            find_pane(&client.session()?, pane_id.as_u64())?;
            apply(client, LayoutCommand::FocusPane { pane_id })?;
            let session = client.session()?;
            let (workspace, tab) = find_pane(&session, pane_id.as_u64())?;
            Ok(json!({ "pane": pane_info(client, workspace, tab, pane_id) }))
        }
        PaneCommand::Focus {
            pane_id,
            direction: Some(direction),
        } => {
            let pane_id = target_pane(pane_id)?;
            let (changed, tab_id) = arrange(
                client,
                pane_id,
                LayoutCommand::FocusPaneDirection {
                    pane_id,
                    direction: direction.into(),
                },
            )?;
            // The result is whichever Pane holds the focus now.
            let session = client.session()?;
            let tab = session
                .tab(tab_id)
                .ok_or_else(|| CliError::new("pane_not_found", "the Tab vanished"))?;
            let focused = tab.focused_pane().id();
            let (workspace, tab) = find_pane(&session, focused.as_u64())?;
            Ok(json!({
                "pane": pane_info(client, workspace, tab, focused),
                "changed": changed,
            }))
        }
        PaneCommand::Resize {
            pane_id,
            direction,
            amount,
        } => {
            let pane_id = target_pane(pane_id)?;
            let (changed, _) = arrange(
                client,
                pane_id,
                LayoutCommand::ResizePane {
                    pane_id,
                    direction: direction.into(),
                    amount,
                },
            )?;
            let session = client.session()?;
            let (workspace, tab) = find_pane(&session, pane_id.as_u64())?;
            Ok(json!({
                "pane": pane_info(client, workspace, tab, pane_id),
                "changed": changed,
            }))
        }
        PaneCommand::Swap { pane_id, direction } => {
            let pane_id = target_pane(pane_id)?;
            let (changed, _) = arrange(
                client,
                pane_id,
                LayoutCommand::SwapPane {
                    pane_id,
                    direction: direction.into(),
                },
            )?;
            let session = client.session()?;
            let (workspace, tab) = find_pane(&session, pane_id.as_u64())?;
            Ok(json!({
                "pane": pane_info(client, workspace, tab, pane_id),
                "changed": changed,
            }))
        }
        PaneCommand::Move { pane_id, to, side } => {
            let pane_id = target_pane(pane_id)?;
            let (changed, _) = arrange(
                client,
                pane_id,
                LayoutCommand::MovePane {
                    pane_id,
                    target_pane_id: PaneId::from_u64(to),
                    side: side.into(),
                },
            )?;
            let session = client.session()?;
            let (workspace, tab) = find_pane(&session, pane_id.as_u64())?;
            Ok(json!({
                "pane": pane_info(client, workspace, tab, pane_id),
                "changed": changed,
            }))
        }
        PaneCommand::Zoom { pane_id, on, off } => {
            let pane_id = target_pane(pane_id)?;
            find_pane(&client.session()?, pane_id.as_u64())?;
            let zoomed = client.overview().zoomed_panes.contains(&pane_id);
            // The protocol only toggles; --on and --off skip the toggle when already there.
            let changed = !(on && zoomed || off && !zoomed);
            if changed {
                apply(client, LayoutCommand::TogglePaneZoom { pane_id })?;
            }
            let session = client.session()?;
            let (workspace, tab) = find_pane(&session, pane_id.as_u64())?;
            Ok(json!({
                "pane": pane_info(client, workspace, tab, pane_id),
                "changed": changed,
            }))
        }
        PaneCommand::Layout { pane_id } => {
            let pane_id = target_pane(pane_id)?;
            let session = client.session()?;
            let (workspace, tab) = find_pane(&session, pane_id.as_u64())?;
            let rects = tab.pane_rects();
            let edges = rects.iter().find(|rect| rect.id == pane_id).map(|rect| {
                json!({
                    "left": rect.left, "top": rect.top, "right": rect.right, "bottom": rect.bottom,
                })
            });
            let neighbor = |direction| tab.neighbor(pane_id, direction).map(PaneId::as_u64);
            Ok(json!({
                "workspace_id": workspace.id().as_u64(),
                "tab_id": tab.id().as_u64(),
                "focused_pane_id": tab.focused_pane().id().as_u64(),
                "zoomed_pane_id": client.overview().zoomed_panes.iter()
                    .find(|zoomed| tab.panes().iter().any(|pane| pane.id() == **zoomed))
                    .map(|zoomed| zoomed.as_u64()),
                "pane": {
                    "pane_id": pane_id.as_u64(),
                    "edges": edges,
                    "neighbors": {
                        "left": neighbor(PaneDirection::Left),
                        "right": neighbor(PaneDirection::Right),
                        "up": neighbor(PaneDirection::Up),
                        "down": neighbor(PaneDirection::Down),
                    },
                },
                "panes": rects.iter().map(|rect| json!({
                    "pane_id": rect.id.as_u64(),
                    "left": rect.left, "top": rect.top, "right": rect.right, "bottom": rect.bottom,
                })).collect::<Vec<_>>(),
                "tree": layout_tree(tab.layout()),
            }))
        }
        PaneCommand::Close { pane_id } => {
            find_pane(&client.session()?, pane_id)?;
            apply(
                client,
                LayoutCommand::ClosePane {
                    pane_id: PaneId::from_u64(pane_id),
                },
            )?;
            Ok(json!({ "ok": true }))
        }
        PaneCommand::Read { .. } => unreachable!("read prints text, see run_pane"),
        PaneCommand::SendText { pane_id, text } => {
            find_pane(&client.session()?, pane_id)?;
            send(client, pane_id, [TerminalCommand::Text(text)])?;
            Ok(json!({ "ok": true }))
        }
        PaneCommand::SendKeys { pane_id, keys } => {
            find_pane(&client.session()?, pane_id)?;
            // Every key is validated before anything is typed.
            let commands = keys
                .iter()
                .map(|key| parse_key(key))
                .collect::<Result<Vec<_>, _>>()?;
            send(client, pane_id, commands)?;
            Ok(json!({ "ok": true }))
        }
        PaneCommand::Run { pane_id, command } => {
            find_pane(&client.session()?, pane_id)?;
            // Paste, so a program with bracketed paste on sees one unit of text, then Enter.
            send(
                client,
                pane_id,
                [
                    TerminalCommand::Paste(command),
                    TerminalCommand::Key {
                        key: TerminalKey::Enter,
                        modifiers: TerminalModifiers::default(),
                    },
                ],
            )?;
            Ok(json!({ "ok": true }))
        }
    }
}

fn agent(client: &mut ClientConnection, command: AgentCommand) -> Result<Value, CliError> {
    use condr_core::protocol::{AgentCommand as Request, AgentResponse};
    let request = match command {
        AgentCommand::Available => Request::Available,
        AgentCommand::List => Request::List,
        AgentCommand::Start {
            name,
            kind,
            pane,
            timeout,
            args,
        } => Request::Start {
            name,
            kind: AgentKind::parse_label(&kind).ok_or_else(|| {
                CliError::new("unknown_agent_kind", format!("unknown agent kind {kind}"))
            })?,
            pane_id: PaneId::from_u64(pane),
            args,
            timeout_ms: timeout,
        },
        AgentCommand::Prompt {
            target,
            text,
            wait,
            until,
            timeout,
        } => Request::Prompt {
            target,
            text,
            until: wait.then(|| parse_until(&until)).transpose()?,
            timeout_ms: timeout.unwrap_or(DEFAULT_AGENT_WAIT_MS),
        },
        AgentCommand::Wait {
            target,
            until,
            timeout,
        } => Request::Wait {
            target,
            until: parse_until(&until)?,
            timeout_ms: timeout,
        },
        AgentCommand::Hooks { .. } => unreachable!("handled locally by run_agent"),
    };
    let retry_until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let response = loop {
        let response = client.agent(request.clone())?;
        // A new shell can still be initializing. Retrying is safe only when the
        // Server explicitly rejected the launch before enqueueing any input.
        if matches!(request, Request::Start { .. })
            && response
                .as_ref()
                .is_err_and(|error| error.code == "pane_busy")
            && std::time::Instant::now() < retry_until
        {
            std::thread::sleep(std::time::Duration::from_millis(100));
            continue;
        }
        break response;
    }
    .map_err(|error| CliError {
        code: error.code,
        message: error.message,
    })?;
    match response {
        AgentResponse::Available(agents) => Ok(json!({
            "agents": agents.into_iter().map(|entry| json!({
                "agent": entry.kind.id(),
                "label": entry.kind.label(),
                "command": entry.kind.executable(),
                "executable": entry.executable,
            })).collect::<Vec<_>>()
        })),
        AgentResponse::List(agents) => Ok(json!({
            "agents": agents.into_iter().map(agent_info).collect::<Vec<_>>()
        })),
        AgentResponse::Ready(agent) => Ok(json!({ "agent": agent_info(agent) })),
    }
}

fn agent_info(info: condr_core::protocol::AgentInfo) -> Value {
    json!({
        "pane_id": info.pane_id.as_u64(),
        "name": info.name,
        "agent": info.agent.kind.id(),
        "agent_status": agent_state_name(info.agent.state),
        "launch_pending": info.launch_pending,
    })
}

fn agent_state_name(state: AgentState) -> &'static str {
    match state {
        AgentState::Unknown => "unknown",
        AgentState::Idle => "idle",
        AgentState::Working => "working",
        AgentState::Blocked => "blocked",
    }
}

fn parse_until(values: &[String]) -> Result<Vec<AgentState>, CliError> {
    if values.is_empty() {
        return Ok(vec![AgentState::Idle, AgentState::Blocked]);
    }
    values
        .iter()
        .map(|value| match value.as_str() {
            "unknown" => Ok(AgentState::Unknown),
            "idle" => Ok(AgentState::Idle),
            "working" => Ok(AgentState::Working),
            "blocked" => Ok(AgentState::Blocked),
            _ => Err(CliError::new(
                "invalid_agent_state",
                format!("unknown agent state {value}"),
            )),
        })
        .collect()
}

/// The split tree as JSON: `{"pane": id}` leaves under `{"split": "horizontal"|"vertical",
/// "ratio": r, "first": …, "second": …}` nodes; `first` is the left or top side.
fn layout_tree(layout: &PaneLayout) -> Value {
    match layout {
        PaneLayout::Pane(id) => json!({ "pane": id.as_u64() }),
        PaneLayout::Split {
            direction,
            ratio,
            first,
            second,
        } => json!({
            "split": match direction {
                SplitDirection::Horizontal => "horizontal",
                SplitDirection::Vertical => "vertical",
            },
            "ratio": ratio,
            "first": layout_tree(first),
            "second": layout_tree(second),
        }),
    }
}

/// An explicit Pane id, or the calling Pane.
fn target_pane(pane_id: Option<u64>) -> Result<PaneId, CliError> {
    Ok(PaneId::from_u64(match pane_id {
        Some(id) => id,
        None => caller_pane()?,
    }))
}

/// Applies a rearrangement inside a Pane's Tab and reports whether the Tab changed at
/// all: the Server accepts a resize or swap with no neighbor as a no-op.
fn arrange(
    client: &mut ClientConnection,
    pane_id: PaneId,
    command: LayoutCommand,
) -> Result<(bool, TabId), CliError> {
    let before = client.session()?;
    let (_, tab) = find_pane(&before, pane_id.as_u64())?;
    let tab_id = tab.id();
    let previous = (tab.layout().clone(), tab.focused_pane().id());
    apply(client, command)?;
    let after = client.session()?;
    let tab = after
        .tab(tab_id)
        .ok_or_else(|| CliError::new("pane_not_found", "the Tab vanished"))?;
    let changed = (tab.layout().clone(), tab.focused_pane().id()) != previous;
    Ok((changed, tab_id))
}

fn send(
    client: &mut ClientConnection,
    pane_id: u64,
    commands: impl IntoIterator<Item = TerminalCommand>,
) -> Result<(), CliError> {
    for command in commands {
        client
            .terminal(PaneId::from_u64(pane_id), command)
            .map_err(|error| CliError::new("pane_send_failed", error.to_string()))?;
    }
    Ok(())
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
            let LayoutResult::WorkspaceCreated { workspace_id, .. } = apply(
                client,
                LayoutCommand::CreateWorkspace {
                    root_directory,
                    name: label,
                    focus,
                },
            )?
            else {
                return Err(CliError::new(
                    "workspace_create_failed",
                    "the Server returned no created Workspace ID",
                ));
            };
            let session = client.session()?;
            let workspace = find_workspace(&session, workspace_id.as_u64())?;
            let tab = workspace.active_tab();
            Ok(json!({
                "workspace": workspace_info(&session, workspace),
                "tab": tab_info(workspace, tab),
                "root_pane": pane_info(client, workspace, tab, tab.focused_pane().id()),
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
            let LayoutResult::TabCreated { tab_id, .. } = apply(
                client,
                LayoutCommand::CreateTab {
                    workspace_id,
                    name: label,
                    focus,
                },
            )?
            else {
                return Err(CliError::new(
                    "tab_create_failed",
                    "the Server returned no created Tab ID",
                ));
            };
            let session = client.session()?;
            let (workspace, tab) = find_tab(&session, tab_id.as_u64())?;
            Ok(json!({
                "tab": tab_info(workspace, tab),
                "root_pane": pane_info(client, workspace, tab, tab.focused_pane().id()),
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
