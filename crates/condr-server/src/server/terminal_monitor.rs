use super::*;
use std::panic::{self, AssertUnwindSafe};

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
        // The other span ADR 0019 allows; this thread owns the Pane's terminal runtime.
        let _pane_span = tracing::info_span!("pane", id = pane_id.as_u64()).entered();
        // A panic while publishing ends the Pane, as one on its own threads does (ADR 0053).
        let exited = panic::catch_unwind(AssertUnwindSafe(|| {
            publish_frames(
                &updates,
                &view_source,
                &notice_probe,
                &activity,
                &state,
                pane_id,
                instance_id,
            )
        }))
        .unwrap_or_else(|_| {
            tracing::error!("the Pane's monitor panicked; ending the Pane");
            true
        });
        if exited {
            end_terminal(
                &state,
                &lifecycle,
                &cwd_probe,
                &view_source,
                pane_id,
                instance_id,
            );
        }
        // Dropping `activity` ends the probe thread.
    });
}

/// Publishes the Pane's frames until its terminal exits (`true`) or the Pane stops being
/// this monitor's (`false`).
fn publish_frames(
    updates: &mpsc::Receiver<TerminalUpdate>,
    view_source: &TerminalViewSource,
    notice_probe: &TerminalNoticeProbe,
    activity: &mpsc::SyncSender<()>,
    state: &Arc<Mutex<RuntimeState>>,
    pane_id: PaneId,
    instance_id: u64,
) -> bool {
    let mut last_view_publish = Instant::now()
        .checked_sub(TERMINAL_FRAME_INTERVAL)
        .unwrap_or_else(Instant::now);
    // Output since the scrollback was last compressed to the end.
    let mut compression_pending = true;
    loop {
        // A held cursor move or an open synchronized update times out without new output.
        // An idle Pane looks again only while its scrollback may still be compressed (ADR
        // 0051) or a progress may time out (ADR 0054); otherwise it sleeps until output.
        let wait = view_source.frame_wakeup();
        let idle = compression_pending
            .then_some(condr_core::TERMINAL_IDLE_CHECK)
            .into_iter()
            .chain(notice_probe.progress_timeout())
            .min();
        let received = match wait.or(idle) {
            Some(timeout) => updates.recv_timeout(timeout),
            None => updates
                .recv()
                .map_err(|_| mpsc::RecvTimeoutError::Disconnected),
        };
        let update = match received {
            Ok(update) => {
                compression_pending = true;
                update
            }
            Err(mpsc::RecvTimeoutError::Timeout) if wait.is_some() => {
                TerminalUpdate::View(view_source.revision())
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if compression_pending {
                    compression_pending = view_source.compress_if_idle();
                }
                if !take_notices(notice_probe, state, pane_id, instance_id) {
                    return false;
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return false,
        };
        let update =
            coalesce_terminal_update(update, updates, last_view_publish + TERMINAL_FRAME_INTERVAL);
        if !take_notices(notice_probe, state, pane_id, instance_id) {
            return false;
        }
        match update {
            TerminalUpdate::View(_) => {
                #[cfg(test)]
                RuntimeState::panic_if_injected(state, "monitor", pane_id.as_u64());
                // Full means a nudge is already pending; the probe reads the latest state.
                let _ = activity.try_send(());
                let Some(frame) = view_source.take_frame() else {
                    continue;
                };
                if !publish_terminal_frame(state, view_source, pane_id, instance_id, frame) {
                    return false;
                }
                last_view_publish = Instant::now();
            }
            TerminalUpdate::Exited => return true,
        }
    }
}

/// Applies what the Pane's terminal reported besides its grid; `false` once the Pane is
/// no longer this monitor's.
fn take_notices(
    notice_probe: &TerminalNoticeProbe,
    state: &Arc<Mutex<RuntimeState>>,
    pane_id: PaneId,
    instance_id: u64,
) -> bool {
    let notices = notice_probe.take();
    if notices.is_empty() {
        return true;
    }
    let mut state = state.lock().or_abort();
    if !state.terminal_is_current(pane_id, instance_id) {
        return false;
    }
    apply_terminal_notices(&mut state, pane_id, notices);
    true
}

/// Reaps the Pane's ended terminal and keeps it, exited, with its final view.
fn end_terminal(
    state: &Arc<Mutex<RuntimeState>>,
    lifecycle: &Arc<ServerLifecycle>,
    cwd_probe: &TerminalCwdProbe,
    view_source: &TerminalViewSource,
    pane_id: PaneId,
    instance_id: u64,
) {
    let Some(_operation) = lifecycle.begin_operation() else {
        return;
    };
    let cwd_before_shutdown = cwd_probe.observe();
    let runtime = {
        let mut state = state.lock().or_abort();
        if !state.terminal_is_current(pane_id, instance_id) {
            None
        } else {
            state.terminal_instances.remove(&pane_id);
            state.closing_terminals.insert(pane_id);
            state.terminals.remove(&pane_id)
        }
    };
    let Some(mut runtime) = runtime else {
        return;
    };
    if let Err(error) = runtime.close() {
        tracing::warn!("failed to reap Terminal after PTY EOF: {error}");
    }
    let cwd = shutdown_cwd(cwd_before_shutdown, cwd_probe.observe());
    let final_view = view_source.view();
    let clients = {
        let mut state = state.lock().or_abort();
        if state.session.pane(pane_id).is_none() || state.terminals.contains_key(&pane_id) {
            return;
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
        state.record_agent_resume(pane_id, None);
        state.forget_agent_control(pane_id);
        state.publish_background(SessionEvent::TerminalExited { pane_id });
        state.clear_terminal_reports(pane_id);
        state.forget_agent(pane_id);
        clients
    };
    for client_id in clients {
        flush_terminal_render(state, client_id);
    }
}

/// Runs agent/cwd probes and sends Git refresh intents off the frame path. The agent
/// probe ticks on its own cadence (300 ms) whether or not the terminal printed anything,
/// since processes exit without output; nudges feed cwd and the shared Git scheduler.
/// Slow Git scans cannot delay agent-state or cwd detection.
fn probe_terminal(
    pane_id: PaneId,
    instance_id: u64,
    activity: mpsc::Receiver<()>,
    agent_probe: TerminalAgentProbe,
    cwd_probe: TerminalCwdProbe,
    state: Arc<Mutex<RuntimeState>>,
) {
    let probing = Arc::clone(&state);
    thread::spawn(move || {
        // A probe that panics leaves the Pane running, its Agent shown as Unknown since
        // what it last reported may no longer hold (ADR 0053).
        let probed = panic::catch_unwind(AssertUnwindSafe(move || {
            probe_until_closed(
                pane_id,
                instance_id,
                &activity,
                agent_probe,
                &cwd_probe,
                &state,
            )
        }));
        if probed.is_err() {
            tracing::error!("the Pane's probe panicked; its Agent shows as Unknown");
            let mut state = probing.lock().or_abort();
            if state.terminal_is_current(pane_id, instance_id)
                && let Some(agent) = state.agents.get(&pane_id).cloned()
            {
                let unknown = AgentSnapshot {
                    state: condr_core::AgentState::Unknown,
                    detail: None,
                    ..agent
                };
                apply_agent_refresh(&mut state, pane_id, Some(unknown));
            }
        }
    });
}

fn probe_until_closed(
    pane_id: PaneId,
    instance_id: u64,
    activity: &mpsc::Receiver<()>,
    mut agent_probe: TerminalAgentProbe,
    cwd_probe: &TerminalCwdProbe,
    state: &Arc<Mutex<RuntimeState>>,
) {
    let mut resume_pending = true;
    let mut resume_retry_at = Instant::now();
    let mut cwd_scan_due: Option<Instant> = None;
    loop {
        match activity.recv_timeout(agent_probe.poll_interval()) {
            Ok(()) => {
                let now = Instant::now();
                cwd_scan_due = Some(now + CWD_SCAN_INTERVAL);
                let state = state.lock().or_abort();
                if !state.terminal_is_current(pane_id, instance_id) {
                    break;
                }
                state.request_terminal_git_refresh(pane_id, now);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        #[cfg(test)]
        RuntimeState::panic_if_injected(state, "probe", pane_id.as_u64());

        let now = Instant::now();
        if resume_pending && now >= resume_retry_at {
            resume_pending = agents::resume_agent(state, pane_id, instance_id);
            resume_retry_at = now + RESUME_RETRY_INTERVAL;
        }
        let agent_update = agent_probe.poll();
        let agent_resume = agent_probe.resume();
        // Debounced behind output: a burst must not turn into a process cwd read per
        // chunk, and the shutdown path records the final cwd on its own.
        let cwd = if cwd_scan_due.is_some_and(|due| now >= due) {
            cwd_scan_due = None;
            cwd_probe.cwd()
        } else {
            None
        };
        if agent_update.is_some() || cwd.is_some() || agent_resume.is_some() {
            let mut state = state.lock().or_abort();
            if !state.terminal_is_current(pane_id, instance_id) {
                break;
            }
            if let Some(cwd) = cwd {
                state.record_terminal_cwds([(pane_id, cwd)]);
            }
            if let Some(resume) = agent_resume {
                state.record_agent_resume(pane_id, resume);
            }
            if let Some(next) = agent_update {
                state.agent_process_update(pane_id, agent_probe.agent());
                apply_agent_refresh(&mut state, pane_id, next);
            }
        }
    }
}

pub(super) fn publish_terminal_frame(
    state: &Arc<Mutex<RuntimeState>>,
    view_source: &TerminalViewSource,
    pane_id: PaneId,
    instance_id: u64,
    frame: TerminalViewFrame,
) -> bool {
    let clients = {
        let mut state = state.lock().or_abort();
        if !state.terminal_is_current(pane_id, instance_id) {
            return false;
        }
        state.publish_terminal(pane_id, frame)
    };
    let clients = match clients {
        Some(clients) => clients,
        None => {
            let full = TerminalViewFrame::Full(view_source.view());
            let mut state = state.lock().or_abort();
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
    let previous = state.agents.get(&pane_id);
    if previous == next.as_ref() {
        state.complete_agent_waits();
        return;
    }
    // The one place that sees both states, so pushes are found here (ADR 0047).
    let alert = next
        .as_ref()
        .and_then(|next| condr_core::AgentAlert::between(previous, next));
    let is_done = |agent: Option<&AgentSnapshot>| {
        agent.is_some_and(|agent| agent.state == condr_core::AgentState::Done)
    };
    // A Done arriving while no Client has the Pane focused waits for someone to look
    // (ADR 0052); a repeated Done keeps whatever it had.
    let unseen = match (is_done(previous), is_done(next.as_ref())) {
        (_, false) => Some(false),
        (false, true) => Some(!state.is_focused(pane_id)),
        (true, true) => None,
    };
    match &next {
        Some(agent) => {
            state.agents.insert(pane_id, agent.clone());
        }
        None => {
            state.agents.remove(&pane_id);
        }
    }
    // Only ConPTY loses the agent's kitty probe. Elsewhere the agent negotiates, and no
    // kitty mode means it handed the terminal to something else, such as its editor.
    if let Some(terminal) = state.terminals.get(&pane_id) {
        terminal.set_reads_kitty_keys(
            cfg!(windows)
                && next
                    .as_ref()
                    .is_some_and(|agent| agent.kind.spec().reads_kitty_keys),
        );
    }
    if let (Some(alert), Some(agent)) = (alert, &next) {
        state.push_agent_alert(pane_id, alert, agent);
    }
    state.publish_background(SessionEvent::AgentChanged {
        pane_id,
        agent: next,
    });
    if let Some(unseen) = unseen {
        state.set_agent_unseen(pane_id, unseen);
    }
    state.agent_state_changed(pane_id);
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
        && !state.is_focused(pane_id)
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
    // An Agent that reports its state is notified of by Condr already; its own "turn
    // complete" would say it twice (ADR 0054).
    let reporting_agent = state
        .agents
        .get(&pane_id)
        .is_some_and(|agent| agent.state != condr_core::AgentState::Unknown);
    if !reporting_agent {
        for notification in notices.notifications {
            state.publish_background(SessionEvent::TerminalNotification {
                pane_id,
                title: notification.title,
                body: notification.body,
            });
        }
    }
    for command in notices.finished_commands {
        state.publish_background(SessionEvent::CommandFinished {
            pane_id,
            exit_code: command.exit_code,
            duration_secs: command.duration.as_secs(),
        });
    }
    if let Some(progress) = notices.progress {
        let previous = match progress {
            Some(progress) => state.terminal_progress.insert(pane_id, progress),
            None => state.terminal_progress.remove(&pane_id),
        };
        if previous != progress {
            state.publish_background(SessionEvent::TerminalProgressChanged { pane_id, progress });
        }
    }
}
