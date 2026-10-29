use super::*;
use crate::agent::program_status::{BlockedKind, ProgramState};
use condr_ghostty_vt::{BlockedKind as VtBlockedKind, Event, ProgramState as VtProgramState};

/// Out-of-band signals the VT emits alongside grid changes: the OSC 0/2 title, BEL and
/// OSC 52 copies. The monitor drains them with [`TerminalNoticeProbe::take`]; bells are
/// counted, not queued, and only the latest clipboard payload is retained.
#[derive(Debug, Default)]
pub(super) struct TerminalNotices {
    pub(super) title: Option<String>,
    pub(super) title_changed: bool,
    pub(super) bells: u64,
    pub(super) clipboard: Option<String>,
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
            Event::Notification { title, body } => {
                let payload = format!("{AGENT_EVENT_OSC_PREFIX}{body}");
                if format!("777;notify;{title};") == AGENT_EVENT_OSC_PREFIX
                    && let Some(event) = AgentEvent::decode(payload.as_bytes())
                {
                    notices.record_agent_event(AgentNotice::Hook(event));
                }
            }
            Event::ProgramStatus(status) => {
                notices.record_agent_event(AgentNotice::Status(program_status(status)));
            }
            // A prompt or a reset only matters to records the protocol keeps; a plain
            // shell's prompt marks must not wake the process probe.
            Event::PromptStart => {
                if notices.agent.reports_program_status() {
                    notices.record_agent_event(AgentNotice::PromptStart);
                }
            }
            Event::Reset => {
                // RIS clears the title without a title change of its own.
                notices.record_title(None);
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
