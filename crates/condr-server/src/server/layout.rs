use super::*;

pub(super) struct LayoutEffect {
    pub(super) started_terminals: Vec<StartedTerminal>,
    pub(super) removed_terminals: Vec<TerminalRuntime>,
}

pub(super) enum ExternalLayoutPlan {
    CreateWorkspace {
        root: PathBuf,
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
    },
    CreateWorktree {
        parent_workspace_id: WorkspaceId,
        parent_root: PathBuf,
        parent: GitRepository,
        child: GitRepository,
        runtime: Box<TerminalRuntime>,
        updates: mpsc::Receiver<TerminalUpdate>,
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
        LayoutCommand::CreateWorkspace { root_directory } => ExternalLayoutPlan::CreateWorkspace {
            root: root_directory.clone(),
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
    shell: Option<&str>,
) -> Result<PreparedExternalLayout, String> {
    match plan {
        ExternalLayoutPlan::CreateWorkspace { root } => {
            validate_root_directory(&root)?;
            Ok(PreparedExternalLayout::CreateWorkspace {
                git: discover_repository(&root).ok().flatten(),
                root,
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
            let mut runtime = match TerminalRuntime::spawn_shell(
                child.root(),
                TerminalSize::new(24, 80),
                shell,
            ) {
                Ok(runtime) => runtime,
                Err(error) => {
                    return match remove_worktree(&parent, &child) {
                        Ok(()) => Err(format!("failed to start terminal: {error}")),
                        Err(cleanup_error) => Err(format!(
                            "failed to start terminal: {error}; failed to remove the prepared worktree: {cleanup_error}"
                        )),
                    };
                }
            };
            let updates = runtime
                .take_updates()
                .expect("new Terminal update receiver exists");
            Ok(PreparedExternalLayout::CreateWorktree {
                parent_workspace_id,
                parent_root,
                parent,
                child,
                runtime: Box::new(runtime),
                updates,
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
    shell: Option<&str>,
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
                        match TerminalRuntime::spawn_shell(&cwd, spec.size, shell) {
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
    if let PreparedExternalLayout::CreateWorktree {
        parent,
        child,
        runtime,
        updates,
        ..
    } = prepared
    {
        drop(updates);
        drop(runtime);
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

    match command {
        LayoutCommand::CreateWorkspace { root_directory } => {
            let workspace_id = candidate
                .create_workspace(root_directory)
                .ok_or_else(|| "Session Workspace limit reached".to_string())?;
            new_pane = Some(
                candidate
                    .workspace(workspace_id)
                    .expect("new Workspace exists")
                    .active_tab()
                    .focused_pane()
                    .id(),
            );
        }
        LayoutCommand::CreateWorktree { .. }
        | LayoutCommand::OpenWorktree { .. }
        | LayoutCommand::RemoveWorktree { .. } => {
            return Err("worktree command was not prepared".into());
        }
        LayoutCommand::CreateTab { workspace_id } => {
            let tab_id = candidate
                .create_tab(workspace_id)
                .ok_or_else(|| "unknown Workspace or Session Tab limit reached".to_string())?;
            new_pane = Some(
                candidate
                    .tab(tab_id)
                    .expect("new Tab exists")
                    .focused_pane()
                    .id(),
            );
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
        LayoutCommand::SplitPane { pane_id, direction } => {
            new_pane = Some(
                candidate
                    .split_pane(pane_id, direction, 0.5)
                    .ok_or_else(|| {
                        "unknown Pane or Session Pane/layout depth limit reached".to_string()
                    })?,
            );
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
    }

    let started = if let Some(pane_id) = new_pane {
        let cwd = candidate
            .pane(pane_id)
            .and_then(|pane| pane.cwd())
            .ok_or_else(|| "new Pane has no working directory".to_string())?;
        let mut runtime = TerminalRuntime::spawn_shell(
            cwd,
            TerminalSize::new(24, 80),
            Some(state.settings.shell.as_str()),
        )
        .map_err(|error| format!("failed to start terminal: {error}"))?;
        let updates = runtime
            .take_updates()
            .expect("new Terminal update receiver exists");
        Some((pane_id, runtime, updates))
    } else {
        None
    };

    commit_layout_candidate(state, candidate, closed, started)
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
    let mut removed_terminals = Vec::new();
    if let Some(closed) = closed {
        for pane_id in closed.panes() {
            state.exited_terminals.remove(pane_id);
            state.closing_terminals.remove(pane_id);
            state.agents.remove(pane_id);
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
        started_terminals: started_terminals.into_iter().collect(),
        removed_terminals,
    })
}

pub(super) fn apply_prepared_external_layout(
    state: &mut RuntimeState,
    prepared: PreparedExternalLayout,
) -> Result<LayoutEffect, String> {
    match prepared {
        PreparedExternalLayout::CreateWorkspace { root, git } => {
            let effect = apply_layout_command(
                state,
                LayoutCommand::CreateWorkspace {
                    root_directory: root,
                },
            )?;
            let workspace_id = state
                .session
                .active_workspace_id()
                .expect("created Workspace is active");
            set_workspace_git(state, workspace_id, git);
            Ok(effect)
        }
        PreparedExternalLayout::CreateWorktree {
            parent_workspace_id,
            parent_root,
            parent,
            child,
            runtime,
            updates,
        } => {
            let mut candidate = state.session.clone();
            if candidate
                .workspace(parent_workspace_id)
                .is_none_or(|workspace| workspace.root_directory() != parent_root)
            {
                drop(updates);
                drop(runtime);
                return Err(prepared_worktree_failure(
                    &parent,
                    &child,
                    "parent Workspace changed while creating its worktree".into(),
                ));
            }
            let Some(workspace_id) = candidate.create_workspace(child.root().to_path_buf()) else {
                drop(updates);
                drop(runtime);
                return Err(prepared_worktree_failure(
                    &parent,
                    &child,
                    "Session Workspace limit reached".into(),
                ));
            };
            if !candidate.associate_worktree(workspace_id, parent_workspace_id, parent_root, true) {
                drop(updates);
                drop(runtime);
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
                .id();
            let snapshot = candidate.snapshot();
            if let Err(error) = validate_persistable_snapshot(&snapshot) {
                drop(updates);
                drop(runtime);
                return Err(prepared_worktree_failure(&parent, &child, error));
            }
            let effect = match commit_layout_candidate(
                state,
                candidate,
                None,
                Some((pane_id, *runtime, updates)),
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
                    .id();
                let mut runtime = TerminalRuntime::spawn_shell(
                    child.root(),
                    TerminalSize::new(24, 80),
                    Some(state.settings.shell.as_str()),
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

pub(super) fn set_workspace_git(
    state: &mut RuntimeState,
    workspace_id: WorkspaceId,
    git: Option<GitRepository>,
) {
    state
        .workspace_git_scanned_at
        .insert(workspace_id, Instant::now());
    match git {
        Some(repository) => {
            state.workspace_git.insert(workspace_id, repository);
        }
        None => {
            state.workspace_git.remove(&workspace_id);
        }
    }
}

pub(super) fn layout_authority_error(
    state: &RuntimeState,
    client_id: u64,
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
    } else if state.active_controller != Some(client_id) {
        Some("acquire Session control before mutating layout")
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
