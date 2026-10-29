use super::*;
use crate::agent::program_status::{BlockedKind, ProgramState};
use condr_ghostty_vt::{
    BlockedKind as VtBlockedKind, Event, ProgramState as VtProgramState,
    ProgressState as VtProgressState,
};

/// Out-of-band signals the VT emits alongside grid changes: the OSC 0/2 title, BEL and
/// OSC 52 copies. The monitor drains them with [`TerminalNoticeProbe::take`]; bells are
/// counted, not queued, and only the latest clipboard payload is retained.
#[derive(Debug, Default)]
pub(super) struct TerminalNotices {
    pub(super) title: Option<String>,
    pub(super) title_changed: bool,
    pub(super) bells: u64,
    pub(super) clipboard: Option<String>,
    pub(super) progress: Option<TerminalProgress>,
    pub(super) progress_changed: bool,
    /// The progress came from the program's own OSC 7501 report, not OSC 9;4, so the
    /// report that moves on ends it (ADR 0054).
    pub(super) progress_from_status: bool,
    /// When the program last reported its progress, for [`PROGRESS_TIMEOUT`].
    pub(super) progress_at: Option<Instant>,
    /// OSC 9 and 777 notifications the program sent, oldest first (ADR 0054).
    pub(super) notifications: Vec<TerminalNotification>,
    /// The shell command running now, as its shell marked it (ADR 0055).
    pub(super) command: Option<RunningCommand>,
    pub(super) finished_commands: Vec<FinishedCommand>,
    /// What the agent said since the agent probe last drained them, oldest first.
    pub(super) agent_events: Vec<AgentNotice>,
    pub(super) agent: AgentDetector,
    pub(super) agent_stopping: bool,
}

pub(super) type SharedTerminalNotices = Arc<Mutex<TerminalNotices>>;

/// One thing the PTY reader scanned out for the agent detector, applied on the probe's
/// tick so it is ordered against the process table like a hook event.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum AgentNotice {
    Hook(AgentEvent),
    Status(ProgramStatus),
    PromptStart,
    Reset,
}

impl AgentNotice {
    /// Whether the notice names a known agent, so one arriving while the process table
    /// shows none is worth a fresh look at the table: a hook always does, a report only
    /// through its `app`, and a `brew` that learned the protocol never.
    pub(super) fn names_agent(&self) -> bool {
        match self {
            Self::Hook(_) => true,
            Self::Status(status) => status
                .app
                .as_deref()
                .is_some_and(|app| AgentKind::parse_label(app).is_some()),
            Self::PromptStart | Self::Reset => false,
        }
    }

    pub(super) fn apply(&self, agent: &mut AgentDetector) -> AgentPublish {
        match self {
            Self::Hook(event) => agent.observe_event(event),
            Self::Status(status) => agent.observe_status(status),
            Self::PromptStart => agent.observe_prompt_start(),
            Self::Reset => agent.observe_reset(),
        }
    }
}

/// How long a progress lasts without a new report, as in Ghostty (ADR 0054).
pub(super) const PROGRESS_TIMEOUT: Duration = Duration::from_secs(15);

/// How long a shell command runs before its end is worth a notification (ADR 0055).
pub const LONG_COMMAND: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug)]
pub(super) struct RunningCommand {
    pub(super) started: Instant,
    /// Whether an Agent was seen in the Pane while it ran.
    pub(super) agent: bool,
}

/// A notification's text beyond this is cut; an OS shows a few lines of it anyway.
const MAX_NOTIFICATION_BODY_CHARS: usize = 1024;
/// Notifications a program sends faster than the monitor drains them; the oldest go.
const MAX_QUEUED_NOTIFICATIONS: usize = 8;

/// More queued notices than this means nobody is draining them; the oldest go.
const MAX_QUEUED_AGENT_EVENTS: usize = 64;

impl TerminalNotices {
    pub(super) fn record_agent_event(&mut self, event: AgentNotice) {
        if self.agent_events.len() == MAX_QUEUED_AGENT_EVENTS {
            self.agent_events.remove(0);
        }
        self.agent_events.push(event);
    }

    fn record_title(&mut self, title: Option<String>) {
        if self.title != title {
            self.title = title;
            self.title_changed = true;
        }
    }

    fn record_progress(&mut self, progress: Option<TerminalProgress>) {
        self.progress_at = progress.is_some().then(Instant::now);
        if self.progress != progress {
            self.progress = progress;
            self.progress_changed = true;
        }
    }

    fn record_osc_progress(&mut self, progress: Option<TerminalProgress>) {
        self.progress_from_status = false;
        self.record_progress(progress);
    }

    /// A program's root OSC 7501 record with a percentage shows as its progress: working
    /// as progress, blocked as paused. A root report without one ends a progress an
    /// earlier report gave, and leaves an OSC 9;4 progress alone (ADR 0054).
    fn record_status_progress(&mut self, status: &condr_ghostty_vt::ProgramStatus) {
        let state = match status.state {
            VtProgramState::Working => TerminalProgressState::Normal,
            VtProgramState::Blocked => TerminalProgressState::Paused,
            _ => return self.end_status_progress(),
        };
        let Some(percent) = status.progress else {
            return self.end_status_progress();
        };
        self.progress_from_status = true;
        self.record_progress(Some(TerminalProgress {
            state,
            percent: Some(percent),
        }));
    }

    fn end_status_progress(&mut self) {
        if std::mem::take(&mut self.progress_from_status) {
            self.record_progress(None);
        }
    }

    /// The progress the program reported, or `None` once it went [`PROGRESS_TIMEOUT`]
    /// without a report: a program that died mid-task does not keep its bar.
    pub(super) fn take_progress(&mut self) -> Option<Option<TerminalProgress>> {
        if self
            .progress_at
            .is_some_and(|reported| reported.elapsed() >= PROGRESS_TIMEOUT)
        {
            self.record_progress(None);
        }
        std::mem::take(&mut self.progress_changed).then_some(self.progress)
    }

    fn record_notification(&mut self, title: &str, body: &str) {
        let body = body
            .chars()
            .filter(|character| !character.is_control())
            .collect::<String>();
        let body = body
            .trim()
            .chars()
            .take(MAX_NOTIFICATION_BODY_CHARS)
            .collect::<String>();
        let title = sanitize_terminal_title(title);
        if body.is_empty() && title.is_none() {
            return;
        }
        if self.notifications.len() == MAX_QUEUED_NOTIFICATIONS {
            self.notifications.remove(0);
        }
        self.notifications
            .push(TerminalNotification { title, body });
    }

    fn record_command_start(&mut self) {
        self.command = Some(RunningCommand {
            started: Instant::now(),
            agent: self.agent.agent().is_some(),
        });
    }

    fn record_command_end(&mut self, exit_code: Option<i32>) {
        let Some(command) = self.command.take() else {
            return;
        };
        let duration = command.started.elapsed();
        if command.agent || self.agent.agent().is_some() || duration < LONG_COMMAND {
            return;
        }
        if self.finished_commands.len() == MAX_QUEUED_NOTIFICATIONS {
            self.finished_commands.remove(0);
        }
        self.finished_commands.push(FinishedCommand {
            exit_code,
            duration,
        });
    }

    fn record_clipboard(&mut self, text: String) {
        if text.len() <= MAX_PENDING_CLIPBOARD_BYTES {
            self.clipboard = Some(text);
        }
    }
}

/// Drops control characters, trims, and caps the length. Empty becomes `None`. Spinner
/// glyphs stay: Claude Code ("✳ Claude Code") and Codex ("⠼ condr") animate them as
/// their activity indicator, and Windows Terminal shows them as is.
pub(super) fn sanitize_terminal_title(raw: &str) -> Option<String> {
    let title = raw
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    let title = title
        .trim()
        .chars()
        .take(MAX_TERMINAL_TITLE_CHARS)
        .collect::<String>();
    (!title.is_empty()).then_some(title)
}

/// Applies what the VT reported while it parsed one chunk and returns its answers for the
/// PTY, in stream order. Runs on the PTY reader, after the terminal's lock is released.
pub(super) fn apply_terminal_events(
    events: Vec<Event>,
    notices: &SharedTerminalNotices,
    reported_cwd: &Mutex<ReportedCwd>,
) -> Vec<u8> {
    let mut replies = Vec::new();
    if events.is_empty() {
        return replies;
    }
    let mut notices = notices.lock().expect("terminal notices lock poisoned");
    let mut events = events.into_iter().peekable();
    while let Some(event) = events.next() {
        match event {
            // RIS reports a clear of every record just ahead of the reset itself; that is
            // the reset, not a program speaking OSC 7501, and must not mark a plain shell
            // as one that does.
            Event::ProgramStatus(status)
                if status.state == VtProgramState::Clear
                    && status.id.is_empty()
                    && matches!(events.peek(), Some(Event::Reset)) => {}
            Event::WritePty(bytes) => replies.extend(bytes),
            Event::Bell => notices.bells = notices.bells.saturating_add(1),
            Event::TitleChanged(title) => notices.record_title(sanitize_terminal_title(&title)),
            Event::PwdChanged(pwd) => {
                if let Some(cwd) = reported_cwd_of(&pwd) {
                    record_reported_cwd(reported_cwd, cwd);
                }
            }
            Event::ClipboardWrite(text) => notices.record_clipboard(text),
            // Condr's own Agent events travel as OSC 777 notifications too.
            Event::Notification { title, body } => {
                let payload = format!("{AGENT_EVENT_OSC_PREFIX}{body}");
                if format!("777;notify;{title};") == AGENT_EVENT_OSC_PREFIX {
                    if let Some(event) = AgentEvent::decode(payload.as_bytes()) {
                        notices.record_agent_event(AgentNotice::Hook(event));
                    }
                } else {
                    notices.record_notification(&title, &body);
                }
            }
            Event::ProgramStatus(status) => {
                if status.id.is_empty() {
                    notices.record_status_progress(&status);
                }
                notices.record_agent_event(AgentNotice::Status(program_status(status)));
            }
            // A prompt or a reset only matters to records the protocol keeps; a plain
            // shell's prompt marks must not wake the process probe.
            Event::PromptStart => {
                if notices.agent.reports_program_status() {
                    notices.record_agent_event(AgentNotice::PromptStart);
                }
                // A prompt ends the program's records, its progress among them.
                notices.end_status_progress();
            }
            Event::CommandStart => notices.record_command_start(),
            Event::CommandEnd(exit_code) => notices.record_command_end(exit_code),
            Event::Progress(progress) => {
                notices.record_osc_progress(progress.map(|progress| TerminalProgress {
                    state: match progress.state {
                        VtProgressState::Normal => TerminalProgressState::Normal,
                        VtProgressState::Error => TerminalProgressState::Error,
                        VtProgressState::Indeterminate => TerminalProgressState::Indeterminate,
                        VtProgressState::Paused => TerminalProgressState::Paused,
                    },
                    percent: progress.percent,
                }))
            }
            Event::Reset => {
                // RIS clears the title and progress without reports of their own.
                notices.record_title(None);
                notices.record_progress(None);
                notices.progress_from_status = false;
                notices.command = None;
                if notices.agent.reports_program_status() {
                    notices.record_agent_event(AgentNotice::Reset);
                }
            }
            Event::RenderHold(_) => {}
        }
    }
    replies
}

fn program_status(status: condr_ghostty_vt::ProgramStatus) -> ProgramStatus {
    let present = |text: String| (!text.is_empty()).then_some(text);
    ProgramStatus {
        state: match status.state {
            VtProgramState::Idle => ProgramState::Idle,
            VtProgramState::Working => ProgramState::Working,
            VtProgramState::Done => ProgramState::Done,
            VtProgramState::Blocked => ProgramState::Blocked,
            VtProgramState::Error => ProgramState::Error,
            VtProgramState::Clear => ProgramState::Clear,
        },
        id: present(status.id),
        kind: status.kind.map(|kind| match kind {
            VtBlockedKind::Permission => BlockedKind::Permission,
            VtBlockedKind::Question => BlockedKind::Question,
            VtBlockedKind::Auth => BlockedKind::Auth,
        }),
        app: present(status.app),
        msg: present(status.message),
    }
}

/// Writes the VT's answers to the program, ahead of queued user input.
pub(super) fn write_terminal_replies(input: &TerminalInput, replies: Vec<u8>) -> io::Result<()> {
    if replies.is_empty() {
        return Ok(());
    }
    input.write_terminal_reply(replies)
}
