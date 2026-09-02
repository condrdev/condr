use super::*;

pub(super) fn monitor_terminal(monitor: TerminalMonitor) {
    let TerminalMonitor {
        pane_id,
        instance_id,
        updates,
        agent_probe,
        cwd_probe,
        notice_probe,
        view_source,
        state,
        lifecycle,
    } = monitor;
    // Frames must never wait on process enumeration or `git` subprocesses, so the agent,
    // cwd, and Git probes run on their own thread, nudged (coalesced) by terminal activity.
    let (activity, activity_rx) = mpsc::sync_channel::<()>(1);
    probe_terminal(
        pane_id,
        instance_id,
        activity_rx,
        agent_probe,
        cwd_probe.clone(),
        Arc::clone(&state),
    );
    thread::spawn(move || {
        let mut last_view_publish = Instant::now()
            .checked_sub(TERMINAL_FRAME_INTERVAL)
            .unwrap_or_else(Instant::now);
        while let Ok(update) = updates.recv() {
            let update = coalesce_terminal_update(
                update,
                &updates,
                last_view_publish + TERMINAL_FRAME_INTERVAL,
            );
            let notices = notice_probe.take();
            if !notices.is_empty() {
                let mut state = state.lock().expect("server state lock poisoned");
                if !state.terminal_is_current(pane_id, instance_id) {
                    break;
                }
                apply_terminal_notices(&mut state, pane_id, notices);
            }
            match update {
                TerminalUpdate::View(_) => {
                    // Full means a nudge is already pending; the probe reads the latest state.
                    let _ = activity.try_send(());
                    let Some(frame) = view_source.take_frame() else {
                        continue;
                    };
                    if !publish_terminal_frame(&state, &view_source, pane_id, instance_id, frame) {
                        break;
                    }
                    last_view_publish = Instant::now();
                }
                TerminalUpdate::Exited => {
                    let Some(_operation) = lifecycle.begin_operation() else {
                        break;
                    };
                    let cwd_before_shutdown = cwd_probe.observe();
                    let runtime = {
                        let mut state = state.lock().expect("server state lock poisoned");
                        if !state.terminal_is_current(pane_id, instance_id) {
                            None
                        } else {
                            state.terminal_instances.remove(&pane_id);
                            state.closing_terminals.insert(pane_id);
                            state.terminals.remove(&pane_id)
                        }
                    };
                    let Some(mut runtime) = runtime else {
                        break;
                    };
                    if let Err(error) = runtime.close() {
                        eprintln!(
                            "condr-server: failed to reap Terminal for Pane {} after PTY EOF: {error}",
                            pane_id.as_u64()
                        );
                    }
                    let cwd = shutdown_cwd(cwd_before_shutdown, cwd_probe.observe());
                    let final_view = view_source.view();
                    let clients = {
                        let mut state = state.lock().expect("server state lock poisoned");
                        if state.session.pane(pane_id).is_none()
                            || state.terminals.contains_key(&pane_id)
                        {
                            break;
                        }
                        state.terminals.insert(pane_id, runtime);
                        state.terminal_instances.insert(pane_id, instance_id);
                        state.closing_terminals.remove(&pane_id);
                        let clients = state
                            .publish_terminal(pane_id, TerminalViewFrame::Full(final_view))
                            .unwrap_or_default();
                        if let Some(cwd) = cwd {
                            state.record_terminal_cwds([(pane_id, cwd)]);
                        }
                        state.exited_terminals.insert(pane_id);
                        state.publish_background(SessionEvent::TerminalExited { pane_id });
                        state.clear_terminal_title(pane_id);
                        if state.agents.remove(&pane_id).is_some() {
                            state.publish_background(SessionEvent::AgentChanged {
                                pane_id,
                                agent: None,
                            });
                        }
                        clients
                    };
                    for client_id in clients {
                        flush_terminal_render(&state, client_id);
                    }
                    break;
                }
            }
        }
        // Dropping `activity` ends the probe thread.
    });
}

/// Runs the agent, cwd, and Git probes for one Terminal off the frame path. Each nudge marks
/// activity; agent scans are rate-limited to AGENT_SCAN_INTERVAL and Git scans go through
/// `reserve_workspace_git_scan`, exactly as before, but a slow `git` or process enumeration
/// now only delays the next probe, never a frame.
fn probe_terminal(
    pane_id: PaneId,
    instance_id: u64,
    activity: mpsc::Receiver<()>,
    agent_probe: TerminalAgentProbe,
    cwd_probe: TerminalCwdProbe,
    state: Arc<Mutex<RuntimeState>>,
) {
    thread::spawn(move || {
        let mut last_agent_scan = Instant::now();
        let mut agent_scan_pending = false;
        let mut git_scan_pending: Option<Instant> = None;
        loop {
            let nudged = if agent_scan_pending || git_scan_pending.is_some() {
                match activity.recv_timeout(AGENT_SCAN_INTERVAL) {
                    Ok(()) => true,
                    Err(mpsc::RecvTimeoutError::Timeout) => false,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            } else {
                match activity.recv() {
                    Ok(()) => true,
                    Err(_) => break,
                }
            };
            if nudged {
                agent_scan_pending = true;
                git_scan_pending = Some(Instant::now());
            }

            let now = Instant::now();
            if agent_scan_pending && now.duration_since(last_agent_scan) >= AGENT_SCAN_INTERVAL {
                let previous = {
                    let state = state.lock().expect("server state lock poisoned");
                    if !state.terminal_is_current(pane_id, instance_id) {
                        break;
                    }
                    state.agents.get(&pane_id).copied()
                };
                let next = agent_probe.snapshot(previous);
                let cwd = cwd_probe.cwd();
                let mut state = state.lock().expect("server state lock poisoned");
                if !state.terminal_is_current(pane_id, instance_id) {
                    break;
                }
                if let Some(cwd) = cwd {
                    state.record_terminal_cwds([(pane_id, cwd)]);
                }
                apply_agent_refresh(&mut state, pane_id, next);
                last_agent_scan = now;
                agent_scan_pending = false;
            }
            if let Some(activity_at) = git_scan_pending {
                let scan = {
                    let mut state = state.lock().expect("server state lock poisoned");
                    if !state.terminal_is_current(pane_id, instance_id) {
                        break;
                    }
                    reserve_workspace_git_scan(&mut state, pane_id, activity_at, now)
                };
                match scan {
                    WorkspaceGitScan::Waiting => {}
                    WorkspaceGitScan::Covered => git_scan_pending = None,
                    WorkspaceGitScan::Ready { workspace_id, root } => {
                        git_scan_pending = None;
                        if let Ok(next) = discover_repository(&root) {
                            let mut state = state.lock().expect("server state lock poisoned");
                            if !state.terminal_is_current(pane_id, instance_id) {
                                break;
                            }
                            apply_workspace_git_refresh(&mut state, workspace_id, &root, next);
                        }
                    }
                }
            }
        }
    });
}

pub(super) fn publish_terminal_frame(
    state: &Arc<Mutex<RuntimeState>>,
    view_source: &TerminalViewSource,
    pane_id: PaneId,
    instance_id: u64,
    frame: TerminalViewFrame,
) -> bool {
    let clients = {
        let mut state = state.lock().expect("server state lock poisoned");
        if !state.terminal_is_current(pane_id, instance_id) {
            return false;
        }
        state.publish_terminal(pane_id, frame)
    };
    let clients = match clients {
        Some(clients) => clients,
        None => {
            let full = TerminalViewFrame::Full(view_source.view());
            let mut state = state.lock().expect("server state lock poisoned");
            if !state.terminal_is_current(pane_id, instance_id) {
                return false;
            }
            state
                .publish_terminal(pane_id, full)
                .expect("a full terminal frame establishes a retained view")
        }
    };
    for client_id in clients {
        flush_terminal_render(state, client_id);
    }
    true
}

pub(super) fn coalesce_terminal_update(
    first: TerminalUpdate,
    updates: &mpsc::Receiver<TerminalUpdate>,
    publish_at: Instant,
) -> TerminalUpdate {
    let TerminalUpdate::View(mut revision) = first else {
        return first;
    };

    loop {
        let next = if let Some(remaining) = publish_at.checked_duration_since(Instant::now()) {
            match updates.recv_timeout(remaining) {
                Ok(update) => Some(update),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return TerminalUpdate::View(revision),
            }
        } else {
            updates.try_recv().ok()
        };

        match next {
            Some(TerminalUpdate::View(next_revision)) => revision = next_revision,
            Some(TerminalUpdate::Exited) => return TerminalUpdate::Exited,
            None => return TerminalUpdate::View(revision),
        }
    }
}

pub(super) fn apply_agent_refresh(
    state: &mut RuntimeState,
    pane_id: PaneId,
    next: Option<AgentSnapshot>,
) {
    let previous = state.agents.get(&pane_id).copied();
    if previous == next {
        return;
    }
    match next {
        Some(agent) => {
            state.agents.insert(pane_id, agent);
        }
        None => {
            state.agents.remove(&pane_id);
        }
    }
    state.publish_background(SessionEvent::AgentChanged {
        pane_id,
        agent: next,
    });
}

pub(super) fn apply_terminal_notices(
    state: &mut RuntimeState,
    pane_id: PaneId,
    notices: TerminalNoticeBatch,
) {
    if let Some(title) = notices.title {
        let previous = match &title {
            Some(title) => state.terminal_titles.insert(pane_id, title.clone()),
            None => state.terminal_titles.remove(&pane_id),
        };
        if previous != title {
            state.publish_background(SessionEvent::TerminalTitleChanged { pane_id, title });
        }
    }
    if notices.bells > 0
        && state.active_controller.is_some()
        && state.focused_terminal != Some(pane_id)
        && state.pending_terminal_bells.insert(pane_id)
    {
        state.publish_background(SessionEvent::TerminalAttentionChanged {
            pane_id,
            attention: true,
        });
    }
    if let Some(text) = notices.clipboard {
        state.broadcast_clipboard(pane_id, text);
    }
}

pub(super) enum WorkspaceGitScan {
    Waiting,
    Covered,
    Ready {
        workspace_id: WorkspaceId,
        root: PathBuf,
    },
}

pub(super) fn reserve_workspace_git_scan(
    state: &mut RuntimeState,
    pane_id: PaneId,
    activity: Instant,
    now: Instant,
) -> WorkspaceGitScan {
    let Some(workspace) = state.session.workspace_for_pane(pane_id) else {
        return WorkspaceGitScan::Covered;
    };
    let workspace_id = workspace.id();
    let root = workspace.root_directory().to_path_buf();
    if let Some(scanned_at) = state.workspace_git_scanned_at.get(&workspace_id) {
        if *scanned_at >= activity {
            return WorkspaceGitScan::Covered;
        }
        if now.duration_since(*scanned_at) < GIT_SCAN_INTERVAL {
            return WorkspaceGitScan::Waiting;
        }
    }
    state.workspace_git_scanned_at.insert(workspace_id, now);
    WorkspaceGitScan::Ready { workspace_id, root }
}

pub(super) fn apply_workspace_git_refresh(
    state: &mut RuntimeState,
    workspace_id: WorkspaceId,
    root: &std::path::Path,
    next: Option<GitRepository>,
) {
    if state
        .session
        .workspace(workspace_id)
        .is_none_or(|workspace| workspace.root_directory() != root)
    {
        return;
    }
    if state.workspace_git.get(&workspace_id) == next.as_ref() {
        return;
    }
    let git = match next {
        Some(repository) => {
            let snapshot = workspace_git_snapshot(workspace_id, &repository);
            state.workspace_git.insert(workspace_id, repository);
            Some(snapshot)
        }
        None => {
            state.workspace_git.remove(&workspace_id);
            None
        }
    };
    state.publish_background(SessionEvent::WorkspaceGitChanged { workspace_id, git });
}

pub(super) fn workspace_git_snapshot(
    workspace_id: WorkspaceId,
    repository: &GitRepository,
) -> WorkspaceGitSnapshot {
    WorkspaceGitSnapshot {
        workspace_id,
        branch: repository.branch().map(str::to_owned),
        linked_worktree: repository.is_linked_worktree(),
    }
}
