use super::*;

/// The regular poll cadence.
pub const AGENT_POLL_INTERVAL: Duration = Duration::from_millis(300);

/// An `Unidentified` probe may be a transient table read failure; only this many in a
/// row mean the agent is gone. A bare shell means it at once.
const AGENT_MISS_CONFIRMATION_ATTEMPTS: u8 = 6;

const PROCESS_RECHECK_IDENTIFIED: Duration = Duration::from_secs(5);

const PROCESS_RECHECK_UNIDENTIFIED: Duration = Duration::from_millis(500);

/// How long after output or a foreground change an unidentified terminal keeps the fast
/// cadence; after that a quiet shell is only re-enumerated on the slow fallback.
const PROCESS_ACQUISITION_WINDOW: Duration = Duration::from_secs(8);

const PROCESS_RECHECK_QUIET: Duration = Duration::from_secs(30);

/// Activity after this much silence opens a new acquisition window.
const PROCESS_ACQUISITION_IDLE_RESET: Duration = Duration::from_secs(2);

/// What the process probe saw in the terminal's job.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessProbeResult {
    /// The job runs a known agent.
    Agent(AgentKind),
    /// Only the shell itself is left: an agent that was here has exited.
    ShellOnly,
    /// Other processes run, none of them a known agent (or the table was unreadable).
    Unidentified,
}

/// What the detector wants published after a tick.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentPublish {
    Nothing,
    Snapshot(AgentSnapshot),
    /// The agent is gone; the Pane is a plain shell again.
    Cleared,
}

/// Per-terminal agent tracking: which agent the process table shows, the state it last
/// reported, and when to look at the process table again.
#[derive(Debug, Default)]
pub struct AgentDetector {
    agent: Option<AgentKind>,
    state: Option<AgentState>,
    session_id: Option<String>,
    prompt_id: Option<String>,
    detail: Option<String>,
    /// The agent reports through the Program Status Protocol (ADR 0044): from its first
    /// report on, its hooks only name its conversation, and the protocol's lifetime
    /// rules apply to its state.
    program_status: bool,
    /// No observation yet / cleared / the current native conversation. Kept alongside
    /// the live state so shutdown can read a hook before its monitor commits it.
    resume: Option<Option<AgentResume>>,
    restoring: Option<AgentResume>,
    /// The agent was named by its own events, not the process table: a launcher Condr
    /// cannot see through is running it. Only a bare shell clears it then.
    seeded: bool,
    last_probe: Option<ProcessProbeResult>,
    consecutive_misses: u8,
    last_process_probe: Option<Instant>,
    last_activity: Option<Instant>,
    acquisition_started: Option<Instant>,
}

impl AgentDetector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn agent(&self) -> Option<AgentKind> {
        self.agent
    }

    pub fn resume(&self) -> Option<Option<AgentResume>> {
        self.resume.clone()
    }

    /// Whether the agent's state comes from the Program Status Protocol (ADR 0044).
    pub fn reports_program_status(&self) -> bool {
        self.program_status
    }

    /// Seed only the process we are about to resume; what the agent reports remains
    /// authoritative.
    pub fn restore(&mut self, resume: AgentResume) {
        self.resume = Some(Some(resume.clone()));
        self.restoring = Some(resume);
    }

    /// How long the caller should wait before the next tick.
    pub fn poll_interval(&self) -> Duration {
        AGENT_POLL_INTERVAL
    }

    /// Whether this tick should read the process table. Identified agents are
    /// re-checked every 5 s (a job change may force an earlier check through
    /// `force`); an unidentified terminal is checked every 500 ms while output or a
    /// foreground change (`activity`) is recent, then every 30 s until the next one.
    pub fn wants_process_probe(&mut self, now: Instant, force: bool, activity: bool) -> bool {
        if force || activity || self.last_activity.is_none() {
            // Continuous output does not slide the window; only activity after a
            // quiet spell opens a new one.
            let quiet = self
                .last_activity
                .is_none_or(|at| now.duration_since(at) >= PROCESS_ACQUISITION_IDLE_RESET);
            // A foreground-group change is a new job, so it always opens a window.
            if quiet || force {
                self.acquisition_started = Some(now);
            }
            self.last_activity = Some(now);
        }
        if force {
            return true;
        }
        let interval = if self.agent.is_some() {
            PROCESS_RECHECK_IDENTIFIED
        } else if self
            .acquisition_started
            .is_some_and(|at| now.duration_since(at) < PROCESS_ACQUISITION_WINDOW)
        {
            PROCESS_RECHECK_UNIDENTIFIED
        } else {
            PROCESS_RECHECK_QUIET
        };
        self.last_process_probe
            .is_none_or(|checked| now.duration_since(checked) >= interval)
    }

    /// Feeds a process probe. A fresh agent is announced as `Unknown`; an exited one is
    /// cleared. A different kind replacing the current one is a fresh agent.
    pub fn observe_process(&mut self, result: ProcessProbeResult, now: Instant) -> AgentPublish {
        self.last_process_probe = Some(now);
        self.last_probe = Some(result);
        match result {
            ProcessProbeResult::Agent(agent) => {
                self.consecutive_misses = 0;
                if self.agent == Some(agent) {
                    self.seeded = false;
                    return AgentPublish::Nothing;
                }
                self.agent = Some(agent);
                self.prompt_id = None;
                self.detail = None;
                self.program_status = false;
                self.session_id = self
                    .restoring
                    .take()
                    .filter(|resume| resume.kind == agent)
                    .map(|resume| resume.session_id);
                self.resume = Some(self.session_id.as_ref().map(|id| AgentResume {
                    kind: agent,
                    session_id: id.clone(),
                }));
                self.seeded = false;
                self.state = Some(AgentState::Unknown);
                AgentPublish::Snapshot(AgentSnapshot {
                    session_id: self.session_id.clone(),
                    kind: agent,
                    state: AgentState::Unknown,
                    detail: None,
                })
            }
            ProcessProbeResult::ShellOnly | ProcessProbeResult::Unidentified => {
                if self.agent.is_none() {
                    self.consecutive_misses = 0;
                    return AgentPublish::Nothing;
                }
                let exited = match result {
                    ProcessProbeResult::ShellOnly => true,
                    // A seeded agent is expected to be invisible in the table.
                    _ if self.seeded => false,
                    _ => {
                        self.consecutive_misses = self.consecutive_misses.saturating_add(1);
                        self.consecutive_misses >= AGENT_MISS_CONFIRMATION_ATTEMPTS
                    }
                };
                if !exited {
                    return AgentPublish::Nothing;
                }
                self.agent = None;
                self.session_id = None;
                self.prompt_id = None;
                self.detail = None;
                self.program_status = false;
                self.resume = Some(None);
                self.restoring = None;
                self.state = None;
                self.seeded = false;
                self.consecutive_misses = 0;
                // The shell is back in front: a replacement may start any moment.
                self.acquisition_started = Some(now);
                AgentPublish::Cleared
            }
        }
    }

    /// Feeds a hook event. An event naming a different agent than the process table
    /// shows is dropped: it belongs to a nested or forged reporter, not to this Pane's
    /// agent. An event arriving while the table shows a foreground job Condr cannot
    /// name (`mise exec -- claude`, a wrapper script) names the agent instead; a bare
    /// shell (`ShellOnly`) has no agent to name, and the event is dropped. So is every
    /// event from an agent recognized only, whose hooks the person may have configured by
    /// hand: they cannot report a state worth trusting, so it stays `Unknown` (ADR 0035).
    /// Once the agent reports through the Program Status Protocol, its hooks only name
    /// its conversation (ADR 0044).
    pub fn observe_event(&mut self, event: &AgentEvent) -> AgentPublish {
        if event.agent.spec().hooks().is_none() || !self.seed(event.agent) {
            return AgentPublish::Nothing;
        }
        let (Some(agent), Some(state)) = (self.agent, self.state) else {
            return AgentPublish::Nothing;
        };
        if agent != event.agent {
            return AgentPublish::Nothing;
        }
        if agent.spec().hooks().is_some_and(|hooks| hooks.prompt_ids) {
            match event.event {
                AgentEventKind::SessionStart => self.prompt_id = None,
                AgentEventKind::PromptSubmit => self.prompt_id = event.prompt_id.clone(),
                _ if event.prompt_id.is_some()
                    && self.prompt_id.is_some()
                    && (event.prompt_id != self.prompt_id
                        || event
                            .session_id
                            .as_ref()
                            .is_some_and(|id| Some(id) != self.session_id.as_ref())) =>
                {
                    return AgentPublish::Nothing;
                }
                _ => {}
            }
        }
        let session_id = event.session_id.clone().or_else(|| {
            (event.event != AgentEventKind::SessionStart
                || event.source.as_deref() == Some("compact"))
            .then(|| self.session_id.clone())
            .flatten()
        });
        if event.session_id.is_some()
            || (event.event == AgentEventKind::SessionStart
                && event.source.as_deref() != Some("compact"))
        {
            self.restoring = None;
            self.resume = Some(session_id.as_ref().map(|id| AgentResume {
                kind: agent,
                session_id: id.clone(),
            }));
        }
        let (next, detail) = if self.program_status {
            (state, self.detail.clone())
        } else {
            let next = event.apply(state);
            // What blocks is only what the blocking event said; leaving Blocked, or a
            // second Blocked without a detail, clears it (ADR 0024).
            let detail = (next == AgentState::Blocked)
                .then(|| event.detail.clone())
                .flatten();
            (next, detail)
        };
        self.commit(next, session_id, detail)
    }

    /// Feeds a Program Status Protocol report (ADR 0044). The root record is the agent;
    /// a child record (a subagent, a background task) is ignored. A report whose `app`
    /// names another program than the process table shows is a nested reporter's and is
    /// dropped; one without `app` is the Pane's agent's. `clear` deletes the record, and
    /// the agent is `Unknown` again until it reports.
    pub fn observe_status(&mut self, status: &ProgramStatus) -> AgentPublish {
        if status.id.is_some() {
            return AgentPublish::Nothing;
        }
        let app = status.app.as_deref().map(AgentKind::parse_label);
        if let Some(Some(reporter)) = app
            && !self.seed(reporter)
        {
            return AgentPublish::Nothing;
        }
        let Some(agent) = self.agent else {
            return AgentPublish::Nothing;
        };
        if app.is_some_and(|reporter| reporter != Some(agent)) {
            return AgentPublish::Nothing;
        }
        // A state makes the protocol the agent's voice; a bare `clear` from one that
        // never reported leaves its hooks speaking.
        if !self.program_status && status.state != ProgramState::Clear {
            tracing::info!(
                agent = agent.id(),
                "agent reports through the Program Status Protocol"
            );
            self.program_status = true;
        }
        let next = match status.state {
            ProgramState::Idle => AgentState::Idle,
            ProgramState::Working => AgentState::Working,
            ProgramState::Done => AgentState::Done,
            ProgramState::Blocked => AgentState::Blocked,
            ProgramState::Error => AgentState::Error,
            ProgramState::Clear => AgentState::Unknown,
        };
        let detail = match next {
            AgentState::Blocked => status
                .msg
                .clone()
                .or_else(|| status.kind.map(|kind| kind.label().to_owned())),
            AgentState::Error => status.msg.clone(),
            _ => None,
        };
        self.commit(next, self.session_id.clone(), detail)
    }

    /// A new shell prompt (OSC 133 `A`): a `working` or `blocked` record is stale, the
    /// protocol says, while `idle`, `done` and `error` outlive it. Only state the protocol
    /// reported follows its rules; hook state ends with the process (ADR 0014).
    pub fn observe_prompt_start(&mut self) -> AgentPublish {
        if !self.program_status
            || !matches!(self.state, Some(AgentState::Working | AgentState::Blocked))
        {
            return AgentPublish::Nothing;
        }
        self.commit(AgentState::Unknown, self.session_id.clone(), None)
    }

    /// A full reset (`RIS`) removes every record: the agent is `Unknown` until it reports
    /// again, and its hooks count again until it does.
    pub fn observe_reset(&mut self) -> AgentPublish {
        if !self.program_status {
            return AgentPublish::Nothing;
        }
        self.program_status = false;
        self.commit(AgentState::Unknown, self.session_id.clone(), None)
    }

    /// Names the agent from its own report when the process table shows a foreground job
    /// Condr cannot name. Whether there is an agent to attribute the report to afterwards.
    fn seed(&mut self, reporter: AgentKind) -> bool {
        if self.agent.is_none() && self.last_probe == Some(ProcessProbeResult::Unidentified) {
            self.agent = Some(reporter);
            self.seeded = true;
            self.state = Some(AgentState::Unknown);
        }
        self.agent.is_some()
    }

    /// Publishes the snapshot when anything in it changed.
    fn commit(
        &mut self,
        next: AgentState,
        session_id: Option<String>,
        detail: Option<String>,
    ) -> AgentPublish {
        let (Some(agent), Some(state)) = (self.agent, self.state) else {
            return AgentPublish::Nothing;
        };
        if next == state && session_id == self.session_id && detail == self.detail {
            return AgentPublish::Nothing;
        }
        self.state = Some(next);
        self.session_id = session_id;
        self.detail = detail;
        AgentPublish::Snapshot(AgentSnapshot {
            session_id: self.session_id.clone(),
            kind: agent,
            state: next,
            detail: self.detail.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_quiet_shell_leaves_the_fast_probe_cadence_until_something_happens() {
        let mut detector = AgentDetector::new();
        let start = Instant::now();
        assert!(detector.wants_process_probe(start, false, false));
        detector.observe_process(ProcessProbeResult::ShellOnly, start);
        let half_second = Duration::from_millis(500);
        assert!(detector.wants_process_probe(start + half_second, false, false));
        detector.observe_process(ProcessProbeResult::ShellOnly, start + half_second);

        let quiet = start + PROCESS_ACQUISITION_WINDOW + half_second;
        detector.observe_process(ProcessProbeResult::ShellOnly, quiet);
        assert!(!detector.wants_process_probe(quiet + half_second, false, false));
        assert!(detector.wants_process_probe(quiet + PROCESS_RECHECK_QUIET, false, false));

        // Output reopens the window; a foreground change probes at once.
        assert!(detector.wants_process_probe(quiet + half_second, false, true));
        detector.observe_process(ProcessProbeResult::ShellOnly, quiet + half_second);
        assert!(detector.wants_process_probe(quiet + half_second * 2, false, false));
        assert!(detector.wants_process_probe(quiet + half_second * 2, true, false));

        // Continuous output does not keep the window open past its 8 s.
        let mut detector = AgentDetector::new();
        let mut at = start;
        while at < start + PROCESS_ACQUISITION_WINDOW + half_second {
            detector.wants_process_probe(at, false, true);
            detector.observe_process(ProcessProbeResult::Unidentified, at);
            at += half_second;
        }
        assert!(!detector.wants_process_probe(at, false, true));
    }

    #[test]
    fn a_cleared_agent_reopens_the_fast_probe_window_for_its_replacement() {
        let mut detector = AgentDetector::new();
        let start = Instant::now();
        detector.wants_process_probe(start, false, false);
        detector.observe_process(ProcessProbeResult::Agent(AgentKind::Codex), start);
        // A long-running agent outlives the acquisition window it was found in.
        let gone = start + PROCESS_ACQUISITION_WINDOW * 4;
        assert!(detector.wants_process_probe(gone, false, false));
        assert_eq!(
            detector.observe_process(ProcessProbeResult::ShellOnly, gone),
            AgentPublish::Cleared
        );

        // A wrapper restarting the agent a second later is still on the fast cadence.
        let half_second = Duration::from_millis(500);
        assert!(detector.wants_process_probe(gone + half_second, false, false));
    }
}
