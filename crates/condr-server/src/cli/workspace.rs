use super::*;

#[derive(Subcommand)]
pub(crate) enum WorkspaceCommand {
    /// List every Workspace
    List {
        /// Also list the Workspaces of every saved Device; each entry names its `device`
        #[arg(long)]
        all_devices: bool,
    },
    /// Create a Workspace; its first Tab opens a shell in the root directory
    Create {
        /// Root directory; defaults to the current directory
        #[arg(long, value_name = "PATH")]
        cwd: Option<PathBuf>,
        /// Name shown in the sidebar; defaults to the directory name
        #[arg(long, value_name = "TEXT")]
        label: Option<String>,
        /// Ask every connected GUI to show the new Workspace
        #[arg(long)]
        focus: bool,
    },
    /// Show one Workspace
    Get { workspace_id: u64 },
    /// Ask every connected GUI to show a Workspace
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
    /// Create a Tab in a Workspace; defaults to the calling Pane's Workspace
    Create {
        #[arg(long, value_name = "ID")]
        workspace: Option<u64>,
        /// Optional Tab name; unnamed Tabs show only their Workspace position in the GUI
        #[arg(long, value_name = "TEXT")]
        label: Option<String>,
        /// Ask every connected GUI to show the new Tab
        #[arg(long)]
        focus: bool,
    },
    /// Show one Tab
    Get { tab_id: u64 },
    /// Ask every connected GUI to show a Tab
    Focus { tab_id: u64 },
    /// Rename a Tab
    Rename { tab_id: u64, label: String },
    /// Close a Tab and stop its terminals; the last Tab closes its Workspace
    Close { tab_id: u64 },
}

#[derive(Serialize)]
pub(super) struct WorkspaceInfo {
    /// The saved Device this Workspace lives on; absent for the command's own target.
    #[serde(skip_serializing_if = "Option::is_none")]
    device: Option<String>,
    pub(super) workspace_id: u64,
    pub(super) name: String,
    pub(super) root_directory: PathBuf,
    pub(super) tab_count: usize,
    pub(super) pane_count: usize,
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
    pub(super) pane_count: usize,
    /// `None` for a viewer Tab, which has no Panes (ADR 0017).
    pub(super) focused_pane_id: Option<u64>,
}

pub(super) fn workspace_info(workspace: &Workspace) -> WorkspaceInfo {
    WorkspaceInfo {
        device: None,
        workspace_id: workspace.id().as_u64(),
        name: workspace.name().to_owned(),
        root_directory: workspace.root_directory().to_path_buf(),
        tab_count: workspace.tabs().len(),
        pane_count: workspace.tabs().iter().map(|tab| tab.panes().len()).sum(),
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
        pane_count: tab.panes().len(),
        focused_pane_id: tab.focused_pane().map(|pane| pane.id().as_u64()),
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

/// The Pane this process runs in, when `CONDR_PANE_ID` says so and the Session has it.
pub(super) fn caller_pane_in(session: &Session) -> Option<PaneId> {
    let pane_id = std::env::var(PaneEnvironment::PANE_ID).ok()?.parse().ok()?;
    let pane_id = PaneId::from_u64(pane_id);
    session.pane(pane_id).map(|_| pane_id)
}

/// The Workspace of the Pane this process runs in, when `CONDR_PANE_ID` says so.
pub(super) fn caller_workspace(session: &Session) -> Option<WorkspaceId> {
    session
        .workspace_for_pane(caller_pane_in(session)?)
        .map(Workspace::id)
}

pub(crate) fn run_workspace(device: Option<&str>, command: WorkspaceCommand) -> i32 {
    run(device, |client| workspace(client, device, command))
}

pub(crate) fn run_tab(device: Option<&str>, command: TabCommand) -> i32 {
    run(device, |client| tab(client, command))
}

fn workspace(
    client: &mut ClientConnection,
    device: Option<&str>,
    command: WorkspaceCommand,
) -> Result<Value, CliError> {
    match command {
        WorkspaceCommand::List { all_devices } => {
            let session = client.session()?;
            let mut workspaces: Vec<_> = session.workspaces().iter().map(workspace_info).collect();
            if !all_devices {
                return Ok(json!({ "workspaces": workspaces }));
            }
            // Unreachable Devices are reported next to the list, never silently left out:
            // an orchestrator must know which machines it did not see.
            let mut unreachable = Vec::new();
            for (name, result) in super::device::list_workspaces_everywhere(device)? {
                match result {
                    Ok(remote) => workspaces.extend(remote.into_iter().map(|mut info| {
                        info.device = Some(name.clone());
                        info
                    })),
                    Err(error) => {
                        unreachable.push(json!({ "device": name, "error": error.message }))
                    }
                }
            }
            Ok(json!({ "workspaces": workspaces, "unreachable": unreachable }))
        }
        WorkspaceCommand::Create { cwd, label, focus } => {
            let root_directory = std::path::absolute(match cwd {
                Some(cwd) => cwd,
                None => std::env::current_dir()?,
            })?;
            let LayoutResult::WorkspaceCreated {
                workspace_id,
                tab_id,
                ..
            } = apply(
                client,
                LayoutCommand::CreateWorkspace {
                    root_directory,
                    name: label,
                },
            )?
            else {
                return Err(CliError::new(
                    "workspace_create_failed",
                    "the Server returned no created Workspace ID",
                ));
            };
            if focus {
                apply(client, LayoutCommand::ActivateWorkspace { workspace_id })?;
            }
            let session = client.session()?;
            let workspace = find_workspace(&session, workspace_id.as_u64())?;
            let tab = workspace
                .tab(tab_id)
                .ok_or_else(|| CliError::new("tab_not_found", "the created Tab vanished"))?;
            Ok(json!({
                "workspace": workspace_info(workspace),
                "tab": tab_info(workspace, tab),
                "root_pane": tab
                    .focused_pane()
                    .map(|pane| pane_info(client, workspace, tab, pane.id())),
            }))
        }
        WorkspaceCommand::Get { workspace_id } => {
            let session = client.session()?;
            let workspace = find_workspace(&session, workspace_id)?;
            Ok(json!({ "workspace": workspace_info(workspace) }))
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
            Ok(json!({ "workspace": workspace_info(workspace) }))
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
            Ok(json!({ "workspace": workspace_info(workspace) }))
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
                None => caller_workspace(&before).ok_or_else(|| {
                    CliError::new(
                        "workspace_not_found",
                        "not running in a Condr Pane; pass --workspace",
                    )
                })?,
            };
            // The new shell starts where the caller is, when the caller is in that Workspace.
            let cwd_from = caller_pane_in(&before).filter(|pane_id| {
                before
                    .workspace_for_pane(*pane_id)
                    .is_some_and(|workspace| workspace.id() == workspace_id)
            });
            let LayoutResult::TabCreated { tab_id, .. } = apply(
                client,
                LayoutCommand::CreateTab {
                    workspace_id,
                    name: label,
                    cwd_from,
                },
            )?
            else {
                return Err(CliError::new(
                    "tab_create_failed",
                    "the Server returned no created Tab ID",
                ));
            };
            if focus {
                apply(client, LayoutCommand::ActivateTab { tab_id })?;
            }
            let session = client.session()?;
            let (workspace, tab) = find_tab(&session, tab_id.as_u64())?;
            Ok(json!({
                "tab": tab_info(workspace, tab),
                "root_pane": tab
                    .focused_pane()
                    .map(|pane| pane_info(client, workspace, tab, pane.id())),
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
