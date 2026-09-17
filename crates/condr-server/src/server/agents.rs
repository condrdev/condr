use std::collections::HashMap;

use condr_core::protocol::{AgentCommand, AgentError, AgentInfo, AgentResponse};
use condr_core::{AgentKind, AgentState};

use super::*;

const MAX_AGENT_TIMEOUT_MS: u64 = 300_000;

#[derive(Default)]
pub(super) struct AgentControl {
    resumes: HashMap<PaneId, RestoringAgent>,
    names: HashMap<PaneId, ManagedAgent>,
    live: HashMap<PaneId, AgentKind>,
    generations: HashMap<PaneId, u64>,
    pub(super) waiters: HashMap<u64, AgentWaiter>,
}

#[derive(Clone)]
struct RestoringAgent {
    resume: condr_core::AgentResume,
    deadline: Instant,
    submitted: bool,
}

struct ManagedAgent {
    name: String,
    kind: AgentKind,
    instance: u64,
    deadline: Option<Instant>,
    pending: bool,
    observed: bool,
}

pub(super) struct AgentWaiter {
    pane_id: PaneId,
    instance: u64,
    generation: u64,
    kind: AgentKind,
    until: Vec<AgentState>,
    needs_change: bool,
    starting: bool,
    deadline: Instant,
    outbound: ClientWriter,
}

fn error(code: &str, message: impl Into<String>) -> AgentError {
    AgentError {
        code: code.into(),
        message: message.into(),
    }
}

fn reply(outbound: &ClientWriter, result: Result<AgentResponse, AgentError>) {
    let message = ServerMessage::AgentResult { result };
    match frame_message(&message) {
        Ok(data) => {
            let _ = outbound.send_reliable(data);
        }
        Err(err) => {
            queue_message(
                outbound,
                ServerMessage::AgentResult {
                    result: Err(error("agent_response_failed", err.to_string())),
                },
            );
        }
    }
}

/// Prepares a native resume outside the Session lock, then verifies the same Terminal
/// still owns the reservation before enqueueing it once.
pub(super) fn resume_agent(
    state: &Arc<Mutex<RuntimeState>>,
    pane_id: PaneId,
    instance_id: u64,
) -> bool {
    let (resume, probe) = {
        let state = state.lock().expect("server state lock poisoned");
        if !state.terminal_is_current(pane_id, instance_id)
            || state.closing_terminals.contains(&pane_id)
            || state.agent_control.live.contains_key(&pane_id)
        {
            return false;
        }
        let Some(resume) = state.agent_control.resumes.get(&pane_id) else {
            return false;
        };
        (resume.clone(), state.terminals[&pane_id].launch_probe())
    };
    let result = if Instant::now() >= resume.deadline {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "agent resume timed out",
        ))
    } else if resume.submitted || !probe.is_idle() {
        return true;
    } else {
        // Session validation bounds the ID to a plain token; the shell resolves the CLI.
        Ok(format!(
            "{} {}",
            resume.resume.kind.executable(),
            resume.resume.args().join(" "),
        ))
    };
    let mut state = state.lock().expect("server state lock poisoned");
    if !state.terminal_is_current(pane_id, instance_id)
        || state.closing_terminals.contains(&pane_id)
        || state.agent_control.live.contains_key(&pane_id)
        || !state.agent_control.resumes.contains_key(&pane_id)
    {
        return false;
    }
    let result = result.and_then(|command| {
        state.terminals[&pane_id].restore_agent(resume.resume);
        state.terminals[&pane_id].submit(&command)
    });
    match result {
        Ok(()) => {
            state
                .agent_control
                .resumes
                .get_mut(&pane_id)
                .expect("reserved resume")
                .submitted = true;
            true
        }
        Err(error)
            if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < resume.deadline =>
        {
            true
        }
        Err(error) => {
            state.report_resume_failure(pane_id, &error.to_string());
            state.agent_control.resumes.remove(&pane_id);
            false
        }
    }
}

impl RuntimeState {
    pub(super) fn queue_agent_resumes(&mut self) {
        for &pane_id in self.terminals.keys() {
            let Some(resume) = self
                .session
                .pane(pane_id)
                .and_then(|pane| pane.agent_resume())
            else {
                continue;
            };
            self.agent_control.resumes.insert(
                pane_id,
                RestoringAgent {
                    resume: resume.clone(),
                    deadline: Instant::now() + Duration::from_secs(30),
                    submitted: false,
                },
            );
        }
    }

    fn report_resume_failure(&self, pane_id: PaneId, reason: &str) {
        let Some(resume) = self
            .session
            .pane(pane_id)
            .and_then(|pane| pane.agent_resume())
        else {
            return;
        };
        let message = format!(
            "Could not resume {}: {reason}. Retry with: {} {}",
            resume.kind.label(),
            resume.kind.executable(),
            resume.args().join(" "),
        );
        tracing::info!("Pane {}: {message}", pane_id.as_u64());
        if let Some(terminal) = self.terminals.get(&pane_id) {
            terminal.print_notice(&message);
        }
    }

    pub(super) fn record_agent_resume(
        &mut self,
        pane_id: PaneId,
        resume: Option<condr_core::AgentResume>,
    ) {
        // A final hook drain cannot revive a Terminal whose EOF was already handled.
        let resume = resume.filter(|_| !self.exited_terminals.contains(&pane_id));
        let previous = self
            .session
            .pane(pane_id)
            .and_then(|pane| pane.agent_resume())
            .cloned();
        if self.session.set_pane_agent_resume(pane_id, resume) {
            let snapshot = self.session.snapshot();
            match validate_persistable_snapshot(&snapshot) {
                Ok(()) => self.schedule_snapshot(snapshot),
                Err(error) => {
                    self.session.set_pane_agent_resume(pane_id, previous);
                    tracing::warn!(
                        "cannot persist agent resume for Pane {}: {error}",
                        pane_id.as_u64()
                    );
                }
            }
        }
    }

    pub(super) fn handle_agent(
        &mut self,
        client_id: u64,
        outbound: &ClientWriter,
        command: AgentCommand,
        installations: Vec<condr_core::agent_discovery::AgentInstallation>,
        launch: Option<(u64, io::Result<String>)>,
    ) {
        if self.agent_control.waiters.contains_key(&client_id) {
            reply(
                outbound,
                Err(error(
                    "agent_wait_pending",
                    "this connection already has an agent wait",
                )),
            );
            return;
        }
        match self.agent_request(client_id, outbound, command, installations, launch) {
            Ok(Some(response)) => reply(outbound, Ok(response)),
            Ok(None) => self.complete_agent_waits(),
            Err(error) => reply(outbound, Err(error)),
        }
    }

    fn agent_request(
        &mut self,
        client_id: u64,
        outbound: &ClientWriter,
        command: AgentCommand,
        installations: Vec<condr_core::agent_discovery::AgentInstallation>,
        launch: Option<(u64, io::Result<String>)>,
    ) -> Result<Option<AgentResponse>, AgentError> {
        match command {
            AgentCommand::Available => Ok(Some(AgentResponse::Available(installations))),
            // Answered by the connection before the Session lock is taken.
            AgentCommand::Hooks { .. } => Err(error(
                "invalid_agent_request",
                "hooks are handled outside the Session",
            )),
            AgentCommand::List => {
                let mut panes = self
                    .agent_control
                    .live
                    .keys()
                    .chain(self.agent_control.names.keys())
                    .copied()
                    .collect::<Vec<_>>();
                panes.sort_by_key(|pane| pane.as_u64());
                panes.dedup();
                Ok(Some(AgentResponse::List(
                    panes
                        .into_iter()
                        .filter_map(|pane| self.agent_info(pane))
                        .collect(),
                )))
            }
            AgentCommand::Start {
                name,
                kind,
                pane_id,
                args,
                timeout_ms,
            } => {
                if !valid_name(&name) {
                    return Err(error(
                        "invalid_agent_name",
                        "agent name must match [a-z][a-z0-9_-]{0,31}",
                    ));
                }
                let deadline = deadline(timeout_ms)?;
                if timeout_ms <= 3_000 {
                    return Err(error(
                        "invalid_agent_timeout",
                        "start timeout must exceed the 3000 ms detection grace",
                    ));
                }
                if args.iter().map(String::len).sum::<usize>() > 128 * 1024
                    || args.iter().any(|arg| arg.chars().any(char::is_control))
                {
                    return Err(error(
                        "invalid_agent_args",
                        "agent arguments exceed 128 KiB or contain control characters",
                    ));
                }
                self.expire_agent_operations(Instant::now());
                if self
                    .agent_control
                    .names
                    .values()
                    .any(|agent| agent.name == name)
                {
                    return Err(error(
                        "agent_name_in_use",
                        format!("agent name {name} is already in use"),
                    ));
                }
                let instance = self.agent_terminal_instance(pane_id)?;
                if self.agent_control.names.contains_key(&pane_id)
                    || self.agent_control.live.contains_key(&pane_id)
                    || self.agent_control.resumes.contains_key(&pane_id)
                {
                    return Err(error(
                        "pane_busy",
                        "Pane already contains an agent or a pending launch",
                    ));
                }
                let (launch_instance, prepared) = launch.ok_or_else(|| {
                    error(
                        "agent_unavailable",
                        format!(
                            "{} is not executable on the Server's PATH",
                            kind.executable()
                        ),
                    )
                })?;
                if launch_instance != instance {
                    return Err(error(
                        "pane_not_running",
                        "Terminal changed during launch preparation",
                    ));
                }
                prepared
                    .and_then(|command| self.terminals[&pane_id].submit(&command))
                    .map_err(|err| {
                        error(
                            if err.kind() == io::ErrorKind::WouldBlock {
                                "pane_busy"
                            } else {
                                "agent_start_failed"
                            },
                            err.to_string(),
                        )
                    })?;
                // This lock covers the single enqueue and name reservation; a second launch
                // cannot enter before the first has become visible to every client.
                self.agent_control.names.insert(
                    pane_id,
                    ManagedAgent {
                        name,
                        kind,
                        instance,
                        deadline: Some(deadline),
                        pending: true,
                        observed: false,
                    },
                );
                self.add_agent_wait(
                    client_id,
                    outbound,
                    pane_id,
                    kind,
                    ready_states(kind),
                    false,
                    true,
                    deadline,
                )?;
                Ok(None)
            }
            AgentCommand::Prompt {
                target,
                text,
                until,
                timeout_ms,
            } => {
                let deadline = deadline(timeout_ms)?;
                validate_until(until.as_deref())?;
                if text.len() > 1024 * 1024
                    || text
                        .chars()
                        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
                {
                    return Err(error(
                        "invalid_agent_prompt",
                        "prompt exceeds 1 MiB or contains terminal control characters",
                    ));
                }
                let info = self.resolve_agent(&target)?;
                // An agent whose hooks are silent until its first turn is prompted blind
                // once; from then on its state is reported.
                let first_turn = info.agent.state == AgentState::Unknown
                    && !info.agent.kind.reports_at_startup();
                if info.launch_pending || (info.agent.state != AgentState::Idle && !first_turn) {
                    return Err(error(
                        "agent_not_ready",
                        "agent must be idle before accepting a prompt",
                    ));
                }
                let runtime = &self.terminals[&info.pane_id];
                if runtime.running_agent() != Some(info.agent.kind) {
                    return Err(error(
                        "agent_not_running",
                        "agent process changed before prompt submission",
                    ));
                }
                runtime
                    .submit(&text)
                    .map_err(|err| error("agent_prompt_failed", err.to_string()))?;
                if let Some(until) = until {
                    self.add_agent_wait(
                        client_id,
                        outbound,
                        info.pane_id,
                        info.agent.kind,
                        until,
                        true,
                        false,
                        deadline,
                    )?;
                    Ok(None)
                } else {
                    Ok(Some(AgentResponse::Ready(info)))
                }
            }
            AgentCommand::Wait {
                target,
                until,
                timeout_ms,
            } => {
                let deadline = deadline(timeout_ms)?;
                validate_until(Some(&until))?;
                let info = self.resolve_agent(&target)?;
                self.add_agent_wait(
                    client_id,
                    outbound,
                    info.pane_id,
                    info.agent.kind,
                    until,
                    false,
                    false,
                    deadline,
                )?;
                Ok(None)
            }
        }
    }

    fn agent_terminal_instance(&self, pane_id: PaneId) -> Result<u64, AgentError> {
        self.terminal_instances
            .get(&pane_id)
            .copied()
            .filter(|_| {
                self.session.pane(pane_id).is_some()
                    && !self.exited_terminals.contains(&pane_id)
                    && !self.closing_terminals.contains(&pane_id)
            })
            .ok_or_else(|| {
                error(
                    "pane_not_running",
                    format!("Pane {} has no live Terminal", pane_id.as_u64()),
                )
            })
    }

    fn agent_info(&self, pane_id: PaneId) -> Option<AgentInfo> {
        self.agent_terminal_instance(pane_id).ok()?;
        let managed = self.agent_control.names.get(&pane_id);
        let kind = self
            .agent_control
            .live
            .get(&pane_id)
            .copied()
            .or_else(|| managed.map(|agent| agent.kind))?;
        let agent = self
            .agents
            .get(&pane_id)
            .cloned()
            .filter(|agent| agent.kind == kind)
            .unwrap_or(AgentSnapshot {
                session_id: None,
                kind,
                state: AgentState::Unknown,
            });
        Some(AgentInfo {
            pane_id,
            name: managed.map(|agent| agent.name.clone()),
            agent,
            launch_pending: managed.is_some_and(|agent| agent.pending),
        })
    }

    fn resolve_agent(&self, target: &str) -> Result<AgentInfo, AgentError> {
        let pane_id = target
            .parse::<u64>()
            .ok()
            .map(PaneId::from_u64)
            .or_else(|| {
                self.agent_control
                    .names
                    .iter()
                    .find_map(|(pane, agent)| (agent.name == target).then_some(*pane))
            });
        pane_id
            .and_then(|pane| self.agent_info(pane))
            .ok_or_else(|| {
                error(
                    "agent_not_found",
                    format!("no agent named {target} or in that Pane"),
                )
            })
    }

    #[allow(clippy::too_many_arguments)]
    fn add_agent_wait(
        &mut self,
        client_id: u64,
        outbound: &ClientWriter,
        pane_id: PaneId,
        kind: AgentKind,
        until: Vec<AgentState>,
        needs_change: bool,
        starting: bool,
        deadline: Instant,
    ) -> Result<(), AgentError> {
        let instance = self.agent_terminal_instance(pane_id)?;
        self.agent_control.waiters.insert(
            client_id,
            AgentWaiter {
                pane_id,
                instance,
                generation: self
                    .agent_control
                    .generations
                    .get(&pane_id)
                    .copied()
                    .unwrap_or_default(),
                kind,
                until,
                needs_change,
                starting,
                deadline,
                outbound: outbound.clone(),
            },
        );
        Ok(())
    }

    /// Called with process evidence before publishing the display snapshot. Exit Idle
    /// remains available to the GUI but cannot satisfy an orchestration wait.
    pub(super) fn agent_process_update(&mut self, pane_id: PaneId, next: Option<AgentKind>) {
        let previous = self.agent_control.live.get(&pane_id).copied();
        if next.is_some() {
            self.agent_control.resumes.remove(&pane_id);
        }
        match next {
            Some(kind) => {
                self.agent_control.live.insert(pane_id, kind);
            }
            None => {
                self.agent_control.live.remove(&pane_id);
            }
        }
        if previous.is_some() && previous != next {
            *self.agent_control.generations.entry(pane_id).or_default() += 1;
        }
        if let Some(managed) = self.agent_control.names.get_mut(&pane_id) {
            if (managed.observed && next != Some(managed.kind))
                || next.is_some_and(|kind| kind != managed.kind)
            {
                self.agent_control.names.remove(&pane_id);
            } else if next == Some(managed.kind) {
                managed.observed = true;
            }
        }
    }

    pub(super) fn agent_state_changed(&mut self, pane_id: PaneId) {
        for wait in self
            .agent_control
            .waiters
            .values_mut()
            .filter(|wait| wait.pane_id == pane_id)
        {
            wait.needs_change = false;
        }
        if let Some(managed) = self.agent_control.names.get_mut(&pane_id)
            && managed.observed
            && self
                .agents
                .get(&pane_id)
                .is_some_and(|agent| ready_states(agent.kind).contains(&agent.state))
        {
            managed.pending = false;
            managed.deadline = None;
        }
        self.complete_agent_waits();
    }

    fn agent_wait_result(&self, wait: &AgentWaiter) -> Option<Result<AgentResponse, AgentError>> {
        if self.agent_terminal_instance(wait.pane_id).ok() != Some(wait.instance)
            || self
                .agent_control
                .generations
                .get(&wait.pane_id)
                .copied()
                .unwrap_or_default()
                != wait.generation
        {
            return Some(Err(error(
                "agent_not_running",
                "agent or Terminal exited or was replaced",
            )));
        }
        let Some(info) = self.agent_info(wait.pane_id) else {
            return Some(Err(error(
                "agent_not_running",
                "agent exited while waiting",
            )));
        };
        if info.agent.kind != wait.kind {
            return Some(Err(error(
                "agent_changed",
                "a different agent occupies the Pane",
            )));
        }
        if !wait.needs_change && !info.launch_pending && wait.until.contains(&info.agent.state) {
            if self.terminals[&wait.pane_id].running_agent() != Some(wait.kind) {
                return Some(Err(error(
                    "agent_not_running",
                    "agent process exited before becoming ready",
                )));
            }
            if wait.starting && info.agent.state == AgentState::Blocked {
                return Some(Err(error(
                    "agent_not_ready",
                    "agent started but needs input; its name remains available",
                )));
            }
            return Some(Ok(AgentResponse::Ready(info)));
        }
        None
    }

    pub(super) fn complete_agent_waits(&mut self) {
        let completed = self
            .agent_control
            .waiters
            .iter()
            .filter_map(|(client_id, wait)| {
                self.agent_wait_result(wait)
                    .map(|result| (*client_id, result))
            })
            .collect::<Vec<_>>();
        for (client_id, result) in completed {
            let wait = self
                .agent_control
                .waiters
                .remove(&client_id)
                .expect("registered agent wait");
            reply(&wait.outbound, result);
        }
    }

    pub(super) fn expire_agent_operations(&mut self, now: Instant) {
        let expired = self
            .agent_control
            .waiters
            .iter()
            .filter_map(|(client_id, wait)| {
                if self.agent_terminal_instance(wait.pane_id).ok() != Some(wait.instance) {
                    Some((
                        *client_id,
                        error("agent_not_running", "Terminal closed while waiting"),
                    ))
                } else if now >= wait.deadline {
                    Some((
                        *client_id,
                        error(
                            "agent_timeout",
                            format!("timed out waiting for Pane {}", wait.pane_id.as_u64()),
                        ),
                    ))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for (client_id, error) in expired {
            let wait = self
                .agent_control
                .waiters
                .remove(&client_id)
                .expect("registered agent wait");
            reply(&wait.outbound, Err(error));
        }
        let stale = self
            .agent_control
            .names
            .iter()
            .filter_map(|(pane_id, agent)| {
                (self.agent_terminal_instance(*pane_id).ok() != Some(agent.instance)
                    || agent.deadline.is_some_and(|deadline| now >= deadline))
                .then_some(*pane_id)
            })
            .collect::<Vec<_>>();
        for pane_id in stale {
            self.agent_control.names.remove(&pane_id);
        }
    }

    pub(super) fn stop_agent_waits(&mut self) {
        for (_, wait) in self.agent_control.waiters.drain() {
            reply(
                &wait.outbound,
                Err(error("server_stopping", "Server is stopping")),
            );
        }
    }

    pub(super) fn forget_agent_control(&mut self, pane_id: PaneId) {
        self.agent_control.resumes.remove(&pane_id);
        self.agent_control.names.remove(&pane_id);
        self.agent_control.live.remove(&pane_id);
        self.agent_control.generations.remove(&pane_id);
    }
}

/// The states that end a launch: the agent reported a turn boundary or a request, or it
/// is one whose hooks say nothing until prompted and its process has been identified.
fn ready_states(kind: AgentKind) -> Vec<AgentState> {
    let mut states = vec![AgentState::Idle, AgentState::Blocked];
    if !kind.reports_at_startup() {
        states.push(AgentState::Unknown);
    }
    states
}

fn valid_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name.as_bytes()[0].is_ascii_lowercase()
        && name
            .bytes()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, b'_' | b'-'))
}

fn deadline(timeout_ms: u64) -> Result<Instant, AgentError> {
    if timeout_ms > MAX_AGENT_TIMEOUT_MS {
        return Err(error(
            "invalid_agent_timeout",
            "agent timeout must be at most 300000 ms",
        ));
    }
    Ok(Instant::now() + Duration::from_millis(timeout_ms))
}

fn validate_until(until: Option<&[AgentState]>) -> Result<(), AgentError> {
    if until.is_some_and(|until| until.is_empty() || until.len() > 4) {
        return Err(error(
            "invalid_agent_state",
            "wait requires between one and four states",
        ));
    }
    Ok(())
}
