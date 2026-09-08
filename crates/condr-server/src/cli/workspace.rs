use super::*;

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
        /// Optional Tab name; unnamed Tabs show only their Workspace position in the GUI
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

#[derive(Serialize)]
pub(super) struct WorkspaceInfo {
    pub(super) workspace_id: u64,
    pub(super) name: String,
    pub(super) root_directory: PathBuf,
    pub(super) focused: bool,
    pub(super) tab_count: usize,
    pub(super) pane_count: usize,
    pub(super) active_tab_id: u64,
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
pub(super) struct TabInfo {
    pub(super) tab_id: u64,
    pub(super) workspace_id: u64,
    pub(super) name: String,
    /// The active Tab of its Workspace.
    pub(super) focused: bool,
    pub(super) pane_count: usize,
    pub(super) focused_pane_id: u64,
}

pub(super) fn workspace_info(session: &Session, workspace: &Workspace) -> WorkspaceInfo {
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

pub(super) fn tab_info(workspace: &Workspace, tab: &Tab) -> TabInfo {
    TabInfo {
        tab_id: tab.id().as_u64(),
        workspace_id: workspace.id().as_u64(),
        name: tab.name().to_owned(),
        focused: workspace.active_tab().id() == tab.id(),
        pane_count: tab.panes().len(),
        focused_pane_id: tab.focused_pane().id().as_u64(),
    }
}

pub(super) fn find_workspace(session: &Session, id: u64) -> Result<&Workspace, CliError> {
    session
        .workspace(WorkspaceId::from_u64(id))
        .ok_or_else(|| CliError::workspace_not_found(id))
}

pub(super) fn find_tab(session: &Session, id: u64) -> Result<(&Workspace, &Tab), CliError> {
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
pub(super) fn caller_workspace(session: &Session) -> Option<WorkspaceId> {
    let pane_id = std::env::var(PaneEnvironment::PANE_ID).ok()?.parse().ok()?;
    session
        .workspace_for_pane(PaneId::from_u64(pane_id))
        .map(Workspace::id)
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
