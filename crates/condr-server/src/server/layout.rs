use super::*;

pub(super) struct LayoutEffect {
    pub(super) result: LayoutResult,
    pub(super) started_terminals: Vec<StartedTerminal>,
    pub(super) removed_terminals: Vec<TerminalRuntime>,
}

pub(super) enum ExternalLayoutPlan {
    CreateWorkspace {
        root: PathBuf,
        name: Option<String>,
        focus: bool,
    },
    CreateWorktree {
        parent_workspace_id: WorkspaceId,
        parent_root: PathBuf,
        branch: String,
        worktree_root: PathBuf,
    },
    OpenWorktree {
        parent_workspace_id: WorkspaceId,
        parent_root: PathBuf,
        root: PathBuf,
    },
    RemoveWorktree {
        workspace_id: WorkspaceId,
        parent_root: PathBuf,
        root: PathBuf,
    },
}

pub(super) struct TerminalRestartSpec {
    pub(super) pane_id: PaneId,
    pub(super) cwd: PathBuf,
    pub(super) size: TerminalSize,
}

pub(super) enum PreparedExternalLayout {
    CreateWorkspace {
        root: PathBuf,
        git: Option<GitRepository>,
        name: Option<String>,
        focus: bool,
    },
    CreateWorktree {
        parent_workspace_id: WorkspaceId,
        parent_root: PathBuf,
        parent: GitRepository,
        child: GitRepository,
    },
    OpenWorktree {
        parent_workspace_id: WorkspaceId,
        parent_root: PathBuf,
        child: GitRepository,
    },
    RemoveWorktreeReady {
        workspace_id: WorkspaceId,
        parent: GitRepository,
        child: GitRepository,
        restart_specs: Vec<TerminalRestartSpec>,
        stopped_terminals: Vec<(PaneId, TerminalRuntime)>,
    },
    RemoveWorktree {
        workspace_id: WorkspaceId,
    },
}

pub(super) fn external_layout_plan(
    state: &RuntimeState,
    command: &LayoutCommand,
) -> Result<Option<ExternalLayoutPlan>, String> {
    let plan = match command {
        LayoutCommand::CreateWorkspace {
            root_directory,
            name,
            focus,
        } => ExternalLayoutPlan::CreateWorkspace {
            root: root_directory.clone(),
            name: name.clone(),
            focus: *focus,
        },
        LayoutCommand::CreateWorktree {
            parent_workspace_id,
            branch,
        } => {
            let parent_root = state
                .session
                .workspace(*parent_workspace_id)
                .ok_or_else(|| "unknown parent Workspace".to_string())?
                .root_directory()
                .to_path_buf();
            let worktree_root = state.worktree_root.clone().ok_or_else(|| {
                "cannot determine the managed worktree directory for this user".to_string()
            })?;
            ExternalLayoutPlan::CreateWorktree {
                parent_workspace_id: *parent_workspace_id,
                parent_root,
                branch: branch.clone(),
                worktree_root,
            }
        }
        LayoutCommand::OpenWorktree {
            parent_workspace_id,
            root_directory,
        } => {
            let parent_root = state
                .session
                .workspace(*parent_workspace_id)
                .ok_or_else(|| "unknown parent Workspace".to_string())?
                .root_directory()
                .to_path_buf();
            ExternalLayoutPlan::OpenWorktree {
                parent_workspace_id: *parent_workspace_id,
                parent_root,
                root: root_directory.clone(),
            }
        }
        LayoutCommand::RemoveWorktree { workspace_id } => {
            let workspace = state
                .session
                .workspace(*workspace_id)
                .ok_or_else(|| "unknown Workspace".to_string())?;
            let association = workspace
                .worktree()
                .filter(|association| association.is_managed())
                .ok_or_else(|| "Condr can only remove worktrees it created".to_string())?;
            ExternalLayoutPlan::RemoveWorktree {
                workspace_id: *workspace_id,
                parent_root: association.parent_root_directory().to_path_buf(),
                root: workspace.root_directory().to_path_buf(),
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(plan))
}

pub(super) fn prepare_external_layout(
    plan: ExternalLayoutPlan,
) -> Result<PreparedExternalLayout, String> {
    match plan {
        ExternalLayoutPlan::CreateWorkspace { root, name, focus } => {
            validate_root_directory(&root)?;
            Ok(PreparedExternalLayout::CreateWorkspace {
                git: discover_repository(&root).ok().flatten(),
                root,
                name,
                focus,
            })
        }
        ExternalLayoutPlan::CreateWorktree {
            parent_workspace_id,
            parent_root,
            branch,
            worktree_root,
        } => {
            let parent = discover_repository(&parent_root)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "parent Workspace is not a Git repository".to_string())?;
            let child = create_worktree(&parent, &branch, &worktree_root)
                .map_err(|error| error.to_string())?;
            // The shell starts in `apply_prepared_external_layout`, once the Pane exists
            // and its id can go into the shell's environment.
            Ok(PreparedExternalLayout::CreateWorktree {
                parent_workspace_id,
                parent_root,
                parent,
                child,
            })
        }
        ExternalLayoutPlan::OpenWorktree {
            parent_workspace_id,
            parent_root,
            root,
        } => {
            validate_root_directory(&root)?;
            let parent = discover_repository(&parent_root)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "parent Workspace is not a Git repository".to_string())?;
            let child = open_worktree(&parent, root).map_err(|error| error.to_string())?;
            Ok(PreparedExternalLayout::OpenWorktree {
                parent_workspace_id,
                parent_root,
                child,
            })
        }
        ExternalLayoutPlan::RemoveWorktree {
            workspace_id,
            parent_root,
            root,
        } => {
            let parent = discover_repository(parent_root)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "parent Workspace is not a Git repository".to_string())?;
            let child = discover_repository(root)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "Workspace is not a Git worktree".to_string())?;
            validate_worktree_removal(&parent, &child).map_err(|error| error.to_string())?;
            Ok(PreparedExternalLayout::RemoveWorktreeReady {
                workspace_id,
                parent,
                child,
                restart_specs: Vec::new(),
                stopped_terminals: Vec::new(),
            })
        }
    }
}

pub(super) fn validate_root_directory(root: &std::path::Path) -> Result<(), String> {
    if !root.is_absolute() {
        return Err(format!(
            "root directory must be an absolute path: {}",
            root.display()
        ));
    }
    let metadata = std::fs::metadata(root)
        .map_err(|error| format!("cannot access root directory {}: {error}", root.display()))?;
    if !metadata.is_dir() {
        return Err(format!(
            "root directory is not a directory: {}",
            root.display()
        ));
    }
    Ok(())
}

pub(super) fn approve_external_layout(
    state: &mut RuntimeState,
    prepared: &mut PreparedExternalLayout,
) -> Result<(), String> {
    match prepared {
        PreparedExternalLayout::CreateWorktree {
            parent_workspace_id,
            parent_root,
            ..
        }
        | PreparedExternalLayout::OpenWorktree {
            parent_workspace_id,
            parent_root,
            ..
        } => {
            state
                .session
                .workspace(*parent_workspace_id)
                .filter(|workspace| workspace.root_directory() == parent_root)
                .ok_or_else(|| {
                    "parent Workspace changed while preparing its worktree".to_string()
                })?;
        }
        PreparedExternalLayout::RemoveWorktreeReady {
            workspace_id,
            child,
            restart_specs,
            stopped_terminals,
            ..
        } => {
            let workspace = state
                .session
                .workspace(*workspace_id)
                .filter(|workspace| {
                    workspace.root_directory() == child.root()
                        && workspace
                            .worktree()
                            .is_some_and(|association| association.is_managed())
                })
                .ok_or_else(|| "managed Workspace changed while preparing removal".to_string())?;
            let root = workspace.root_directory().to_path_buf();
            let pane_ids = workspace
                .tabs()
                .iter()
                .flat_map(|tab| tab.panes())
                .map(|pane| pane.id())
                .collect::<Vec<_>>();
            for pane_id in pane_ids {
                if let Some(runtime) = state.terminals.remove(&pane_id) {
                    if !state.exited_terminals.contains(&pane_id) {
                        let cwd = state
                            .session
                            .pane(pane_id)
                            .and_then(|pane| pane.cwd())
                            .unwrap_or(&root)
                            .to_path_buf();
                        restart_specs.push(TerminalRestartSpec {
                            pane_id,
                            cwd,
                            size: runtime.size(),
                        });
                    }
                    state.terminal_instances.remove(&pane_id);
                    state.closing_terminals.insert(pane_id);
                    stopped_terminals.push((pane_id, runtime));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) struct ExternalLayoutFinishError {
    pub(super) message: String,
    pub(super) restarted: Vec<(PaneId, TerminalRuntime, mpsc::Receiver<TerminalUpdate>)>,
    pub(super) exited: Vec<(PaneId, TerminalRuntime)>,
    pub(super) cwds: Vec<(PaneId, PathBuf)>,
}

pub(super) fn finish_external_layout(
    prepared: PreparedExternalLayout,
    launch: &ShellLaunch,
) -> Result<PreparedExternalLayout, ExternalLayoutFinishError> {
    match prepared {
        PreparedExternalLayout::RemoveWorktreeReady {
            workspace_id,
            parent,
            child,
            restart_specs,
            mut stopped_terminals,
            ..
        } => {
            let mut close_errors = Vec::new();
            let mut final_cwds = std::collections::HashMap::new();
            for (pane_id, runtime) in &mut stopped_terminals {
                let cwd_probe = runtime.cwd_probe();
                let before_close = cwd_probe.observe();
                if let Err(error) = runtime.close() {
                    close_errors.push(format!("Pane {}: {error}", pane_id.as_u64()));
                }
                if let Some(cwd) = shutdown_cwd(before_close, cwd_probe.observe()) {
                    final_cwds.insert(*pane_id, cwd);
                }
            }
            let removal = if close_errors.is_empty() {
                remove_worktree(&parent, &child).map_err(|error| error.to_string())
            } else {
                Err(format!(
                    "failed to close terminals before removing the worktree: {}",
                    close_errors.join(", ")
                ))
            };
            match removal {
                Ok(()) => Ok(PreparedExternalLayout::RemoveWorktree { workspace_id }),
                Err(mut message) => {
                    let mut restarted = Vec::new();
                    let mut exited = Vec::new();
                    let mut restart_errors = Vec::new();
                    for spec in restart_specs {
                        let cwd = final_cwds.get(&spec.pane_id).cloned().unwrap_or(spec.cwd);
                        match TerminalRuntime::spawn_shell(
                            &cwd,
                            spec.size,
                            launch.shell(),
                            Some(&launch.pane_environment(spec.pane_id)),
                        ) {
                            Ok(mut runtime) => {
                                let updates = runtime
                                    .take_updates()
                                    .expect("new Terminal update receiver exists");
                                restarted.push((spec.pane_id, runtime, updates));
                                if let Some(index) = stopped_terminals
                                    .iter()
                                    .position(|(pane_id, _)| *pane_id == spec.pane_id)
                                {
                                    stopped_terminals.swap_remove(index);
                                }
                            }
                            Err(restart_error) => {
                                restart_errors.push(format!(
                                    "Pane {} in {}: {restart_error}",
                                    spec.pane_id.as_u64(),
                                    cwd.display()
                                ));
                                if let Some(index) = stopped_terminals
                                    .iter()
                                    .position(|(pane_id, _)| *pane_id == spec.pane_id)
                                {
                                    exited.push(stopped_terminals.swap_remove(index));
                                }
                            }
                        }
                    }
                    exited.extend(stopped_terminals);
                    if !restart_errors.is_empty() {
                        message.push_str("; failed to restart terminals after removal failed: ");
                        message.push_str(&restart_errors.join(", "));
                    }
                    Err(ExternalLayoutFinishError {
                        message,
                        restarted,
                        exited,
                        cwds: final_cwds.into_iter().collect(),
                    })
                }
            }
        }
        prepared => Ok(prepared),
    }
}

pub(super) fn cancel_prepared_external_layout(
    prepared: PreparedExternalLayout,
) -> Result<(), String> {
    if let PreparedExternalLayout::CreateWorktree { parent, child, .. } = prepared {
        remove_worktree(&parent, &child).map_err(|error| {
            format!("failed to remove the prepared worktree during rollback: {error}")
        })?;
    }
    Ok(())
}

pub(super) fn prepared_worktree_failure(
    parent: &GitRepository,
    child: &GitRepository,
    message: String,
) -> String {
    match remove_worktree(parent, child) {
        Ok(()) => message,
        Err(error) => format!("{message}; failed to remove the prepared worktree: {error}"),
    }
}

pub(super) fn apply_layout_command(
    state: &mut RuntimeState,
    command: LayoutCommand,
) -> Result<LayoutEffect, String> {
    let mut candidate = state.session.clone();
    let mut new_pane = None;
    let mut closed = None;
    let mut result = LayoutResult::Changed;
    let mut preserve_selection = false;

    match command {
        LayoutCommand::CreateWorkspace {
            root_directory,
            name,
            focus,
        } => {
            let workspace_id = candidate
                .create_workspace(root_directory)
                .ok_or_else(|| "Session Workspace limit reached".to_string())?;
            if let Some(name) = name
                && !candidate.rename_workspace(workspace_id, name)
            {
                return Err("empty Workspace name".into());
            }
            let tab = candidate
                .workspace(workspace_id)
                .expect("new Workspace exists")
                .active_tab();
            let pane_id = tab
                .focused_pane()
                .expect("a new Workspace opens on a terminal Tab")
                .id();
            result = LayoutResult::WorkspaceCreated {
                workspace_id,
                tab_id: tab.id(),
                pane_id,
            };
            new_pane = Some(pane_id);
            preserve_selection = !focus;
        }
        LayoutCommand::CreateWorktree { .. }
        | LayoutCommand::OpenWorktree { .. }
        | LayoutCommand::RemoveWorktree { .. } => {
            return Err("worktree command was not prepared".into());
        }
        LayoutCommand::CreateTab {
            workspace_id,
            name,
            focus,
        } => {
            let tab_id = candidate
                .create_tab(workspace_id)
                .ok_or_else(|| "unknown Workspace or Session Tab limit reached".to_string())?;
            new_pane = Some(
                candidate
                    .tab(tab_id)
                    .expect("new Tab exists")
                    .focused_pane()
                    .expect("a new Tab is a terminal Tab")
                    .id(),
            );
            if let Some(name) = name
                && !candidate.rename_tab(tab_id, name)
            {
                return Err("empty Tab name".into());
            }
            result = LayoutResult::TabCreated {
                tab_id,
                pane_id: new_pane.unwrap(),
            };
            preserve_selection = !focus;
        }
        LayoutCommand::RenameWorkspace { workspace_id, name } => {
            if !candidate.rename_workspace(workspace_id, name) {
                return Err("unknown Workspace or empty name".into());
            }
        }
        LayoutCommand::RenameTab { tab_id, name } => {
            if !candidate.rename_tab(tab_id, name) {
                return Err("unknown Tab or empty name".into());
            }
        }
        LayoutCommand::ActivateWorkspace { workspace_id } => {
            if !candidate.activate_workspace(workspace_id) {
                return Err("unknown Workspace".into());
            }
        }
        LayoutCommand::ActivateTab { tab_id } => {
            if !candidate.activate_tab(tab_id) {
                return Err("unknown Tab".into());
            }
        }
        LayoutCommand::MoveWorkspace {
            workspace_id,
            target_index,
        } => {
            if candidate.workspace(workspace_id).is_none() {
                return Err("unknown Workspace".into());
            }
            if target_index as usize >= candidate.workspaces().len() {
                return Err("Workspace target index is out of range".into());
            }
            candidate.move_workspace(workspace_id, target_index as usize);
        }
        LayoutCommand::MoveTab {
            tab_id,
            target_index,
        } => {
            let Some(workspace) = candidate
                .workspaces()
                .iter()
                .find(|workspace| workspace.tabs().iter().any(|tab| tab.id() == tab_id))
            else {
                return Err("unknown Tab".into());
            };
            if target_index as usize >= workspace.tabs().len() {
                return Err("Tab target index is out of range".into());
            }
            candidate.move_tab(tab_id, target_index as usize);
        }
        LayoutCommand::SplitPane {
            pane_id,
            direction,
            focus,
        } => {
            new_pane = Some(
                candidate
                    .split_pane(pane_id, direction, 0.5)
                    .ok_or_else(|| {
                        "unknown Pane or Session Pane/layout depth limit reached".to_string()
                    })?,
            );
            result = LayoutResult::PaneCreated {
                pane_id: new_pane.unwrap(),
            };
            preserve_selection = !focus;
        }
        LayoutCommand::FocusPane { pane_id } => {
            if !candidate.focus_pane(pane_id) {
                return Err("unknown Pane".into());
            }
        }
        LayoutCommand::FocusPaneDirection { pane_id, direction } => {
            if candidate.pane(pane_id).is_none() {
                return Err("unknown Pane".into());
            }
            candidate.focus_pane_in_direction(pane_id, direction);
        }
        LayoutCommand::ResizePane {
            pane_id,
            direction,
            amount,
        } => {
            if candidate.pane(pane_id).is_none() {
                return Err("unknown Pane".into());
            }
            if !amount.is_finite() || amount <= 0.0 {
                return Err("Pane resize amount must be a positive finite number".into());
            }
            candidate.resize_pane(pane_id, direction, amount);
        }
        LayoutCommand::SetSplitRatios { tab_id, ratios } => {
            let tab = candidate
                .tab(tab_id)
                .ok_or_else(|| "unknown Tab".to_string())?;
            if tab.layout().is_none() {
                return Err("a viewer Tab has no Panes to size".into());
            }
            if ratios.iter().any(|ratio| !ratio.is_finite()) {
                return Err("split ratios must be finite".into());
            }
            if ratios.len() != tab.panes().len().saturating_sub(1) {
                return Err("split ratio count does not match the Tab layout".into());
            }
            candidate.set_tab_split_ratios(tab_id, &ratios);
        }
        LayoutCommand::SwapPane { pane_id, direction } => {
            if candidate.pane(pane_id).is_none() {
                return Err("unknown Pane".into());
            }
            candidate.swap_pane(pane_id, direction);
        }
        LayoutCommand::MovePane {
            pane_id,
            target_pane_id,
            side,
        } => {
            if candidate.pane(pane_id).is_none() || candidate.pane(target_pane_id).is_none() {
                return Err("unknown Pane".into());
            }
            if !candidate.move_pane(pane_id, target_pane_id, side) {
                return Err(
                    "Panes must differ, share a Tab, and leave the Tab another Pane".into(),
                );
            }
        }
        LayoutCommand::TogglePaneZoom { pane_id } => {
            if candidate.pane(pane_id).is_none() {
                return Err("unknown Pane".into());
            }
            candidate.toggle_pane_zoom(pane_id);
        }
        LayoutCommand::ClosePane { pane_id } => {
            closed = Some(
                candidate
                    .close_pane(pane_id)
                    .ok_or_else(|| "unknown Pane".to_string())?,
            );
        }
        LayoutCommand::CloseTab { tab_id } => {
            closed = Some(
                candidate
                    .close_tab(tab_id)
                    .ok_or_else(|| "unknown Tab".to_string())?,
            );
        }
        LayoutCommand::CloseWorkspace { workspace_id } => {
            closed = Some(
                candidate
                    .close_workspace(workspace_id)
                    .ok_or_else(|| "unknown Workspace".to_string())?,
            );
        }
        LayoutCommand::ShowDiff { workspace_id, path } => {
            let tab_id = candidate.show_diff(workspace_id, path).ok_or_else(|| {
                "unknown Workspace, invalid path, or Session Tab limit reached".to_string()
            })?;
            result = LayoutResult::DiffShown { tab_id };
        }
        LayoutCommand::ShowFile { workspace_id, path } => {
            let tab_id = candidate.show_file(workspace_id, path).ok_or_else(|| {
                "unknown Workspace, invalid path, or Session Tab limit reached".to_string()
            })?;
            result = LayoutResult::FileShown { tab_id };
        }
    }

    if preserve_selection {
        candidate.preserve_selection_from(&state.session);
    }
    let started = if let Some(pane_id) = new_pane {
        let cwd = candidate
            .pane(pane_id)
            .and_then(|pane| pane.cwd())
            .ok_or_else(|| "new Pane has no working directory".to_string())?;
        let launch = state.shell_launch();
        let mut runtime = TerminalRuntime::spawn_shell(
            cwd,
            TerminalSize::new(24, 80),
            launch.shell(),
            Some(&launch.pane_environment(pane_id)),
        )
        .map_err(|error| format!("failed to start terminal: {error}"))?;
        let updates = runtime
            .take_updates()
            .expect("new Terminal update receiver exists");
        Some((pane_id, runtime, updates))
    } else {
        None
    };

    let mut effect = commit_layout_candidate(state, candidate, closed, started)?;
    effect.result = result;
    Ok(effect)
}

pub(super) fn layout_command_needs_cwd_observation(command: &LayoutCommand) -> bool {
    matches!(
        command,
        LayoutCommand::CreateTab { .. } | LayoutCommand::SplitPane { .. }
    )
}

pub(super) fn commit_layout_candidate(
    state: &mut RuntimeState,
    candidate: Session,
    closed: Option<condr_core::CloseOutcome>,
    started: Option<(PaneId, TerminalRuntime, mpsc::Receiver<TerminalUpdate>)>,
) -> Result<LayoutEffect, String> {
    let next_snapshot = candidate.snapshot();
    validate_persistable_snapshot(&next_snapshot)?;
    let durable_changed = next_snapshot != state.session.snapshot();
    state.session = candidate;
    let session = &state.session;
    state
        .workspace_git
        .retain(|workspace_id, _| session.workspace(*workspace_id).is_some());
    state
        .workspace_git_scanned_at
        .retain(|workspace_id, _| session.workspace(*workspace_id).is_some());
    state
        .workspace_git_heads
        .retain(|workspace_id, _| session.workspace(*workspace_id).is_some());
    state.unwatch_closed_workspaces();
    let mut removed_terminals = Vec::new();
    if let Some(closed) = closed {
        for pane_id in closed.panes() {
            state.exited_terminals.remove(pane_id);
            state.closing_terminals.remove(pane_id);
            state.agents.remove(pane_id);
            state.forget_agent_control(*pane_id);
            state.terminal_titles.remove(pane_id);
            state.pending_terminal_bells.remove(pane_id);
            state.terminal_views.remove(pane_id);
            state.terminal_instances.remove(pane_id);
            if let Some(runtime) = state.terminals.remove(pane_id) {
                removed_terminals.push(runtime);
            }
        }
    }
    let started_terminals = started
        .map(|(pane_id, runtime, updates)| state.install_terminal(pane_id, runtime, updates));
    if durable_changed {
        state.schedule_snapshot(next_snapshot);
    }
    Ok(LayoutEffect {
        result: LayoutResult::Changed,
        started_terminals: started_terminals.into_iter().collect(),
        removed_terminals,
    })
}

pub(super) fn apply_prepared_external_layout(
    state: &mut RuntimeState,
    prepared: PreparedExternalLayout,
) -> Result<LayoutEffect, String> {
    match prepared {
        PreparedExternalLayout::CreateWorkspace {
            root,
            git,
            name,
            focus,
        } => {
            let effect = apply_layout_command(
                state,
                LayoutCommand::CreateWorkspace {
                    root_directory: root,
                    name,
                    focus,
                },
            )?;
            let LayoutResult::WorkspaceCreated { workspace_id, .. } = effect.result else {
                unreachable!("created Workspace returns its identity")
            };
            set_workspace_git(state, workspace_id, git);
            Ok(effect)
        }
        PreparedExternalLayout::CreateWorktree {
            parent_workspace_id,
            parent_root,
            parent,
            child,
        } => {
            let mut candidate = state.session.clone();
            if candidate
                .workspace(parent_workspace_id)
                .is_none_or(|workspace| workspace.root_directory() != parent_root)
            {
                return Err(prepared_worktree_failure(
                    &parent,
                    &child,
                    "parent Workspace changed while creating its worktree".into(),
                ));
            }
            let Some(workspace_id) = candidate.create_workspace(child.root().to_path_buf()) else {
                return Err(prepared_worktree_failure(
                    &parent,
                    &child,
                    "Session Workspace limit reached".into(),
                ));
            };
            if !candidate.associate_worktree(workspace_id, parent_workspace_id, parent_root, true) {
                return Err(prepared_worktree_failure(
                    &parent,
                    &child,
                    "failed to associate the created worktree".into(),
                ));
            }
            let pane_id = candidate
                .workspace(workspace_id)
                .expect("created Workspace exists")
                .active_tab()
                .focused_pane()
                .expect("a new Workspace opens on a terminal Tab")
                .id();
            let snapshot = candidate.snapshot();
            if let Err(error) = validate_persistable_snapshot(&snapshot) {
                return Err(prepared_worktree_failure(&parent, &child, error));
            }
            let launch = state.shell_launch();
            let mut runtime = match TerminalRuntime::spawn_shell(
                child.root(),
                TerminalSize::new(24, 80),
                launch.shell(),
                Some(&launch.pane_environment(pane_id)),
            ) {
                Ok(runtime) => runtime,
                Err(error) => {
                    return Err(prepared_worktree_failure(
                        &parent,
                        &child,
                        format!("failed to start terminal: {error}"),
                    ));
                }
            };
            let updates = runtime
                .take_updates()
                .expect("new Terminal update receiver exists");
            let effect = match commit_layout_candidate(
                state,
                candidate,
                None,
                Some((pane_id, runtime, updates)),
            ) {
                Ok(effect) => effect,
                Err(error) => {
                    return Err(prepared_worktree_failure(&parent, &child, error));
                }
            };
            set_workspace_git(state, workspace_id, Some(child));
            Ok(effect)
        }
        PreparedExternalLayout::OpenWorktree {
            parent_workspace_id,
            parent_root,
            child,
        } => {
            let mut candidate = state.session.clone();
            if candidate
                .workspace(parent_workspace_id)
                .is_none_or(|workspace| workspace.root_directory() != parent_root)
            {
                return Err("parent Workspace changed while opening its worktree".into());
            }
            let (workspace_id, started) = if let Some(workspace_id) = candidate
                .workspace_by_root(child.root())
                .map(|workspace| workspace.id())
            {
                if candidate
                    .workspace(workspace_id)
                    .is_some_and(|workspace| workspace.worktree().is_none())
                {
                    candidate.associate_worktree(
                        workspace_id,
                        parent_workspace_id,
                        parent_root,
                        false,
                    );
                }
                candidate.activate_workspace(workspace_id);
                (workspace_id, None)
            } else {
                let workspace_id = candidate
                    .create_workspace(child.root().to_path_buf())
                    .ok_or_else(|| "Session Workspace limit reached".to_string())?;
                candidate.associate_worktree(workspace_id, parent_workspace_id, parent_root, false);
                let pane_id = candidate
                    .workspace(workspace_id)
                    .expect("created Workspace exists")
                    .active_tab()
                    .focused_pane()
                    .expect("a new Workspace opens on a terminal Tab")
                    .id();
                let launch = state.shell_launch();
                let mut runtime = TerminalRuntime::spawn_shell(
                    child.root(),
                    TerminalSize::new(24, 80),
                    launch.shell(),
                    Some(&launch.pane_environment(pane_id)),
                )
                .map_err(|error| format!("failed to start terminal: {error}"))?;
                let updates = runtime
                    .take_updates()
                    .expect("new Terminal update receiver exists");
                (workspace_id, Some((pane_id, runtime, updates)))
            };
            let effect = commit_layout_candidate(state, candidate, None, started)?;
            set_workspace_git(state, workspace_id, Some(child));
            Ok(effect)
        }
        PreparedExternalLayout::RemoveWorktreeReady { .. } => {
            Err("worktree removal was not finalized".into())
        }
        PreparedExternalLayout::RemoveWorktree { workspace_id } => {
            let mut candidate = state.session.clone();
            candidate
                .workspace(workspace_id)
                .filter(|workspace| {
                    workspace
                        .worktree()
                        .is_some_and(|association| association.is_managed())
                })
                .ok_or_else(|| {
                    "managed Workspace changed while removing its worktree".to_string()
                })?;
            let closed = candidate
                .close_workspace(workspace_id)
                .expect("validated Workspace exists");
            commit_layout_candidate(state, candidate, Some(closed), None)
        }
    }
}

/// Layout does not require Session control: control guards terminal input, while any
/// client, the CLI in a Pane included, may change structure (ADR 0009).
pub(super) fn layout_authority_error(
    state: &RuntimeState,
    _client_id: u64,
    server_id: ServerId,
    session_id: SessionId,
    request_id: u64,
    stopping: bool,
) -> Option<ServerMessage> {
    let reason = if stopping {
        Some("Server is stopping")
    } else if server_id != state.server_id {
        Some("unknown Server")
    } else if session_id != state.session_id {
        Some("unknown Session")
    } else {
        None
    };
    reason.map(|reason| ServerMessage::LayoutRejected {
        server_id: state.server_id,
        session_id: state.session_id,
        request_id,
        reason: reason.into(),
    })
}

pub(super) fn plan_client_external_layout(
    state: &mut RuntimeState,
    client_id: u64,
    server_id: ServerId,
    session_id: SessionId,
    request_id: u64,
    stopping: bool,
    command: &LayoutCommand,
) -> Result<ExternalLayoutPlan, Box<ServerMessage>> {
    if let Some(error) = layout_authority_error(
        state, client_id, server_id, session_id, request_id, stopping,
    ) {
        return Err(Box::new(error));
    }
    let plan = external_layout_plan(state, command)
        .map_err(|reason| {
            Box::new(ServerMessage::LayoutRejected {
                server_id: state.server_id,
                session_id: state.session_id,
                request_id,
                reason,
            })
        })?
        .expect("external Layout command has a plan");
    Ok(plan)
}
