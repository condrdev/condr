use super::*;

impl RuntimeState {
    pub(super) fn recover(
        socket_path: &std::path::Path,
        snapshot_path: Option<PathBuf>,
        config_path: Option<PathBuf>,
    ) -> io::Result<(Self, Vec<StartedTerminal>)> {
        let settings = ServerSettings {
            shell: load_shell(config_path.as_deref()),
            default_shell: condr_core::default_shell_program(),
        };
        let launch = ShellLaunch {
            shell: settings.shell.clone(),
            socket_path: socket_path.to_string_lossy().into_owned(),
        };
        let persistence = snapshot_path.map(SnapshotPersistence::open).transpose()?;
        let mut session = Session::new();
        let mut restored = false;
        if let Some(persistence) = persistence.as_ref() {
            match persistence.load() {
                SnapshotLoad::Missing => {}
                SnapshotLoad::Loaded(snapshot) => match validate_persistable_snapshot(&snapshot)
                    .and_then(|()| validate_snapshot_root_paths(&snapshot))
                {
                    Ok(()) => match Session::restore(snapshot) {
                        Ok(loaded) => {
                            session = loaded;
                            restored = true;
                        }
                        Err(error) => eprintln!(
                            "condr-server: ignoring invalid Session Snapshot at {}: {error}",
                            persistence.path().display()
                        ),
                    },
                    Err(error) => eprintln!(
                        "condr-server: ignoring invalid Session Snapshot at {}: {error}",
                        persistence.path().display()
                    ),
                },
                SnapshotLoad::Rejected(reason) => eprintln!(
                    "condr-server: ignoring invalid Session Snapshot at {}: {reason}",
                    persistence.path().display()
                ),
            }
        }

        let repaired_worktrees = if restored {
            clear_invalid_restored_worktrees(&mut session)
        } else {
            0
        };

        let launch_specs = session
            .workspaces()
            .iter()
            .flat_map(|workspace| {
                workspace.tabs().iter().flat_map(move |tab| {
                    tab.panes().iter().map(move |pane| {
                        (
                            pane.id(),
                            pane.cwd().map(PathBuf::from),
                            workspace.root_directory().to_path_buf(),
                        )
                    })
                })
            })
            .collect::<Vec<_>>();
        let mut started_runtimes = Vec::new();
        let mut failed_panes = Vec::new();
        let mut repaired_cwds = 0;
        for (pane_id, saved_cwd, workspace_root) in launch_specs {
            let requested_cwd = saved_cwd
                .as_deref()
                .filter(|cwd| cwd.is_absolute())
                .unwrap_or(workspace_root.as_path());
            let started = match TerminalRuntime::spawn_shell(
                requested_cwd,
                TerminalSize::new(24, 80),
                launch.shell(),
                Some(&launch.pane_environment(pane_id)),
            ) {
                Ok(runtime) => Some((runtime, requested_cwd.to_path_buf())),
                Err(error) if requested_cwd != workspace_root.as_path() => {
                    eprintln!(
                        "condr-server: fresh shell failed for Pane {} in saved cwd {}; retrying Workspace Root {}: {error}",
                        pane_id.as_u64(),
                        requested_cwd.display(),
                        workspace_root.display()
                    );
                    match TerminalRuntime::spawn_shell(
                        &workspace_root,
                        TerminalSize::new(24, 80),
                        launch.shell(),
                        Some(&launch.pane_environment(pane_id)),
                    ) {
                        Ok(runtime) => Some((runtime, workspace_root)),
                        Err(fallback_error) => {
                            eprintln!(
                                "condr-server: pruning Pane {} after fresh shell also failed in Workspace Root: {fallback_error}",
                                pane_id.as_u64()
                            );
                            None
                        }
                    }
                }
                Err(error) => {
                    eprintln!(
                        "condr-server: pruning Pane {} after fresh shell failed in {}: {error}",
                        pane_id.as_u64(),
                        requested_cwd.display()
                    );
                    None
                }
            };
            let Some((mut runtime, started_cwd)) = started else {
                failed_panes.push(pane_id);
                continue;
            };
            if saved_cwd.is_some()
                && saved_cwd.as_deref() != Some(started_cwd.as_path())
                && session.set_pane_cwd(pane_id, Some(started_cwd))
            {
                repaired_cwds += 1;
            }
            let updates = runtime
                .take_updates()
                .expect("new Terminal update receiver exists");
            started_runtimes.push((pane_id, runtime, updates));
        }
        for pane_id in &failed_panes {
            session
                .close_pane(*pane_id)
                .expect("restored Pane remains until it is pruned");
        }

        let mut state =
            Self::with_session(socket_path, session, persistence, settings, config_path);
        let workspace_roots = state
            .session
            .workspaces()
            .iter()
            .map(|workspace| (workspace.id(), workspace.root_directory().to_path_buf()))
            .collect::<Vec<_>>();
        for (workspace_id, root) in workspace_roots {
            set_workspace_git(
                &mut state,
                workspace_id,
                discover_repository(root).ok().flatten(),
            );
        }
        let startup_terminals = started_runtimes
            .into_iter()
            .map(|(pane_id, runtime, updates)| state.install_terminal(pane_id, runtime, updates))
            .collect();
        if restored && (!failed_panes.is_empty() || repaired_worktrees != 0 || repaired_cwds != 0) {
            state.schedule_snapshot(state.session.snapshot());
        }
        Ok((state, startup_terminals))
    }

    pub(super) fn schedule_snapshot(&self, snapshot: condr_core::SessionSnapshot) {
        if let Some(persistence) = &self.persistence {
            persistence.schedule(snapshot);
        }
    }

    pub(super) fn record_terminal_cwds(
        &mut self,
        cwds: impl IntoIterator<Item = (PaneId, PathBuf)>,
    ) -> bool {
        let mut candidate = self.session.clone();
        let mut updates = Vec::new();
        for (pane_id, cwd) in cwds {
            let Some(pane) = candidate.pane(pane_id) else {
                continue;
            };
            if pane.cwd() == Some(cwd.as_path()) {
                continue;
            }
            if cwd.to_str().is_none() {
                eprintln!(
                    "condr-server: ignoring unpersistable Terminal cwd update for Pane {}: path is not valid UTF-8",
                    pane_id.as_u64()
                );
                continue;
            }
            candidate.set_pane_cwd(pane_id, Some(cwd.clone()));
            updates.push((pane_id, cwd));
        }
        if updates.is_empty() {
            return false;
        }

        let snapshot = candidate.snapshot();
        if validate_persistable_snapshot(&snapshot).is_ok() {
            self.session = candidate;
            self.schedule_snapshot(snapshot);
            return true;
        }

        candidate = self.session.clone();
        let mut accepted = false;
        let mut accepted_snapshot = None;
        for (pane_id, cwd) in updates {
            let previous = candidate
                .pane(pane_id)
                .and_then(|pane| pane.cwd())
                .map(PathBuf::from);
            candidate.set_pane_cwd(pane_id, Some(cwd));
            let snapshot = candidate.snapshot();
            match validate_persistable_snapshot(&snapshot) {
                Ok(()) => {
                    accepted = true;
                    accepted_snapshot = Some(snapshot);
                }
                Err(error) => {
                    candidate.set_pane_cwd(pane_id, previous);
                    eprintln!(
                        "condr-server: ignoring unpersistable Terminal cwd update for Pane {}: {error}",
                        pane_id.as_u64()
                    );
                }
            }
        }
        if accepted {
            self.session = candidate;
            self.schedule_snapshot(
                accepted_snapshot.expect("an accepted cwd update produced a Snapshot"),
            );
            true
        } else {
            false
        }
    }

    pub(super) fn record_terminal_cwd_observations(
        &mut self,
        observations: Vec<(PaneId, u64, Option<PathBuf>)>,
    ) -> bool {
        let cwds = observations
            .into_iter()
            .filter_map(|(pane_id, instance_id, cwd)| {
                (self.terminal_is_current(pane_id, instance_id)
                    && !self.closing_terminals.contains(&pane_id)
                    && !self.exited_terminals.contains(&pane_id))
                .then_some(cwd)
                .flatten()
                .map(|cwd| (pane_id, cwd))
            })
            .collect::<Vec<_>>();
        self.record_terminal_cwds(cwds)
    }
}

pub(super) fn shutdown_cwd(
    before_close: (Option<PathBuf>, u64),
    after_close: (Option<PathBuf>, u64),
) -> Option<PathBuf> {
    let persistable = |cwd: Option<PathBuf>| cwd.filter(|cwd| cwd.to_str().is_some());
    if after_close.1 != before_close.1 {
        return persistable(after_close.0).or_else(|| persistable(before_close.0));
    }

    #[cfg(windows)]
    let candidates = [after_close.0, before_close.0];
    #[cfg(not(windows))]
    let candidates = [before_close.0, after_close.0];
    candidates.into_iter().find_map(persistable)
}

pub(super) fn observe_terminal_cwds(
    probes: Vec<(PaneId, u64, TerminalCwdProbe)>,
) -> Vec<(PaneId, u64, Option<PathBuf>)> {
    probes
        .into_iter()
        .map(|(pane_id, instance_id, probe)| (pane_id, instance_id, probe.cwd()))
        .collect()
}

fn restored_worktree_is_valid(root: &std::path::Path, parent_root: &std::path::Path) -> bool {
    let Ok(Some(parent)) = discover_repository(parent_root) else {
        return false;
    };
    let Ok(child) = open_worktree(&parent, root) else {
        return false;
    };
    let Ok(root) = std::fs::canonicalize(root) else {
        return false;
    };
    let Ok(discovered_root) = std::fs::canonicalize(child.root()) else {
        return false;
    };
    root == discovered_root
}

fn clear_invalid_restored_worktrees(session: &mut Session) -> usize {
    let associations = session
        .workspaces()
        .iter()
        .filter_map(|workspace| {
            workspace.worktree().map(|association| {
                (
                    workspace.id(),
                    workspace.root_directory().to_path_buf(),
                    association.parent_root_directory().to_path_buf(),
                )
            })
        })
        .collect::<Vec<_>>();
    let mut cleared = 0;
    for (workspace_id, root, parent_root) in associations {
        if restored_worktree_is_valid(&root, &parent_root) {
            continue;
        }
        if session.clear_worktree_association(workspace_id) {
            eprintln!(
                "condr-server: clearing stale worktree association for Workspace {} at {}",
                workspace_id.as_u64(),
                root.display()
            );
            cleared += 1;
        }
    }
    cleared
}
