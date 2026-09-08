use super::*;

#[derive(Clone)]
pub struct TerminalCwdProbe {
    #[cfg(unix)]
    pub(super) master: Option<Weak<Mutex<Box<dyn MasterPty + Send>>>>,
    pub(super) process: ProcessProbe,
    pub(super) last_known_cwd: Arc<Mutex<Option<PathBuf>>>,
    pub(super) reported_cwd: Arc<Mutex<ReportedCwd>>,
}

impl TerminalCwdProbe {
    pub fn cwd(&self) -> Option<PathBuf> {
        self.observe().0
    }

    pub fn observe(&self) -> (Option<PathBuf>, u64) {
        let mut last_known = self
            .last_known_cwd
            .lock()
            .expect("Terminal cwd lock poisoned");

        #[cfg(unix)]
        let foreground = self
            .master
            .as_ref()
            .and_then(Weak::upgrade)
            .and_then(|master| self.process.foreground_cwd(&master));
        let reported = self
            .reported_cwd
            .lock()
            .expect("Terminal cwd lock poisoned")
            .clone();
        let reported_generation = reported.generation;
        let reported_cwd = reported.cwd.and_then(existing_absolute_directory);

        #[cfg(unix)]
        let observed = foreground.or(reported_cwd).or_else(|| self.process.cwd());
        #[cfg(not(unix))]
        let observed = reported_cwd.or_else(|| self.process.cwd());
        let cwd = if let Some(cwd) = observed {
            *last_known = Some(cwd.clone());
            Some(cwd)
        } else {
            last_known.clone().and_then(existing_absolute_directory)
        };
        (cwd, reported_generation)
    }
}

#[derive(Clone)]
pub struct TerminalNoticeProbe {
    pub(super) notices: SharedTerminalNotices,
}

/// What changed since the previous `take`: `title` is `Some` only when the title changed
/// (to the new title, or `None` when it was reset); `bells` counts BELs, collapsed by the caller.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TerminalNoticeBatch {
    pub title: Option<Option<String>>,
    pub bells: u64,
    /// Latest OSC 52 copy payload, including an empty clipboard clear.
    pub clipboard: Option<String>,
}

impl TerminalNoticeBatch {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.bells == 0 && self.clipboard.is_none()
    }
}

impl TerminalNoticeProbe {
    pub fn take(&self) -> TerminalNoticeBatch {
        let mut notices = self.notices.lock().expect("terminal notices lock poisoned");
        TerminalNoticeBatch {
            title: std::mem::take(&mut notices.title_changed).then(|| notices.title.clone()),
            bells: std::mem::take(&mut notices.bells),
            clipboard: std::mem::take(&mut notices.clipboard),
        }
    }
}

/// Tracks which agent occupies one Terminal: the process probe names it, and its state is
/// `Unknown` until its hooks report (ADR 0014). Poll it every
/// [`TerminalAgentProbe::poll_interval`].
pub struct TerminalAgentProbe {
    #[cfg(unix)]
    pub(super) master: Weak<Mutex<Box<dyn MasterPty + Send>>>,
    pub(super) process: ProcessProbe,
    pub(super) notices: SharedTerminalNotices,
    pub(super) revision: Arc<AtomicU64>,
    /// The terminal revision the last tick saw; new output keeps process probing fast.
    pub(super) activity_revision: Option<u64>,
    #[cfg(unix)]
    pub(super) last_foreground_group: Option<UnixPid>,
}

impl TerminalAgentProbe {
    pub fn poll_interval(&self) -> Duration {
        self.notices
            .lock()
            .expect("terminal notices lock poisoned")
            .agent
            .poll_interval()
    }

    pub fn agent(&self) -> Option<AgentKind> {
        self.notices
            .lock()
            .expect("terminal notices lock poisoned")
            .agent
            .agent()
    }

    pub fn resume(&self) -> Option<Option<crate::AgentResume>> {
        self.notices
            .lock()
            .expect("terminal notices lock poisoned")
            .agent
            .resume()
    }

    /// One tick. `Some(None)` means the agent left; `Some(Some(_))` is the agent and state
    /// to publish. Hook events queued since the last tick are applied after the process
    /// table has named the agent; events arriving before that force the probe.
    pub fn poll(&mut self) -> Option<Option<AgentSnapshot>> {
        let now = Instant::now();
        let foreground_changed = self.foreground_changed();
        let revision = self.revision.load(Ordering::Acquire);
        let output_changed = self.activity_revision != Some(revision);
        self.activity_revision = Some(revision);
        let (probe, event_count) = {
            let mut notices = self.notices.lock().expect("terminal notices lock poisoned");
            if notices.agent_stopping {
                return None;
            }
            let force = foreground_changed
                || (!notices.agent_events.is_empty() && notices.agent.agent().is_none());
            (
                notices
                    .agent
                    .wants_process_probe(now, force, output_changed),
                notices.agent_events.len(),
            )
        };
        // OS work never holds the notice lock, so shutdown can freeze immediately.
        let process = probe.then(|| self.probe_process());
        let mut notices = self.notices.lock().expect("terminal notices lock poisoned");
        if notices.agent_stopping {
            return None;
        }
        let mut publish = AgentPublish::Nothing;
        if let Some(process) = process.filter(|p| *p != ProcessProbeResult::ShellOnly) {
            publish = notices.agent.observe_process(process, now);
        }
        // Hooks arriving during the OS probe belong to the next observation: the
        // foreground job may have changed after we sampled it.
        let count = event_count.min(notices.agent_events.len());
        let events = notices.agent_events.drain(..count).collect::<Vec<_>>();
        for event in events {
            match notices.agent.observe_event(&event) {
                AgentPublish::Nothing => {}
                next => publish = next,
            }
        }
        // An exit-tail hook belongs to the job that just left, not to the bare shell.
        if process == Some(ProcessProbeResult::ShellOnly) {
            match notices
                .agent
                .observe_process(ProcessProbeResult::ShellOnly, now)
            {
                AgentPublish::Nothing => {}
                next => publish = next,
            }
        }
        match publish {
            AgentPublish::Nothing => None,
            AgentPublish::Snapshot(snapshot) => Some(Some(snapshot)),
            AgentPublish::Cleared => Some(None),
        }
    }

    /// A foreground group change (Unix) forces an early process probe.
    fn foreground_changed(&mut self) -> bool {
        #[cfg(unix)]
        {
            let Some(master) = self.master.upgrade() else {
                return false;
            };
            let group = self.process.foreground_group(&master);
            let changed =
                self.last_foreground_group.is_some() && group != self.last_foreground_group;
            self.last_foreground_group = group;
            changed
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    fn probe_process(&self) -> ProcessProbeResult {
        #[cfg(unix)]
        {
            match self.master.upgrade() {
                Some(master) => self.process.probe_agent(&master),
                None => ProcessProbeResult::Unidentified,
            }
        }
        #[cfg(windows)]
        {
            self.process.probe_agent()
        }
    }
}

/// The last `lines` rows that end at the last row with any content, reaching into
/// scrollback when the screen alone holds fewer. Asking for five rows of a fresh shell
/// whose prompt sits at the top therefore returns the prompt, not the blank bottom.
pub(super) fn recent_text(terminal: &Terminal, lines: usize) -> String {
    let top = -(terminal.grid().history_size() as i32);
    let mut last = terminal.screen_lines() as i32 - 1;
    while last >= top && row_text(terminal, Line(last)).is_empty() {
        last -= 1;
    }
    if last < top {
        return String::new();
    }
    let first = (last + 1)
        .saturating_sub(i32::try_from(lines).unwrap_or(i32::MAX))
        .max(top);
    (first..=last)
        .map(|row| row_text(terminal, Line(row)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One grid row as text, wide-character spacers skipped and trailing spaces trimmed.
pub(super) fn row_text(terminal: &Terminal, line: Line) -> String {
    let row = &terminal.grid()[line];
    let mut text = String::new();
    for column in 0..terminal.columns() {
        let cell = &row[Column(column)];
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        if cell.flags.contains(Flags::HIDDEN) {
            text.push(' ');
        } else {
            text.push(cell.c);
            if let Some(zerowidth) = cell.zerowidth() {
                text.extend(zerowidth);
            }
        }
    }
    text.truncate(text.trim_end_matches(' ').len());
    text
}
