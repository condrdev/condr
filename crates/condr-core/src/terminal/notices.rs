use super::*;
use alacritty_terminal::vte::ansi::Rgb;

#[derive(Clone)]
pub(super) struct TerminalEventProxy {
    pub(super) size: Arc<Mutex<TerminalSize>>,
    pub(super) pending_replies: PendingTerminalReplies,
    pub(super) notices: SharedTerminalNotices,
}

/// Out-of-band signals the VT emits alongside grid changes: the OSC 0/2 title, BEL and
/// OSC 52 copies. The monitor drains them with [`TerminalNoticeProbe::take`]; bells are
/// counted, not queued, and only the latest clipboard payload is retained.
#[derive(Debug, Default)]
pub(super) struct TerminalNotices {
    pub(super) title: Option<String>,
    pub(super) title_changed: bool,
    pub(super) bells: u64,
    pub(super) clipboard: Option<String>,
    /// Hook events since the agent probe last drained them, oldest first.
    pub(super) agent_events: Vec<AgentEvent>,
    pub(super) agent: AgentDetector,
    pub(super) agent_stopping: bool,
}

pub(super) type SharedTerminalNotices = Arc<Mutex<TerminalNotices>>;

/// More queued hook events than this means nobody is draining them; the oldest go.
const MAX_QUEUED_AGENT_EVENTS: usize = 64;

impl TerminalNotices {
    pub(super) fn record_agent_event(&mut self, event: AgentEvent) {
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

pub(super) enum PendingTerminalReply {
    Bytes(Vec<u8>),
    Color(usize, Arc<dyn Fn(Rgb) -> String + Sync + Send + 'static>),
}

pub(super) type PendingTerminalReplies = Arc<Mutex<Vec<PendingTerminalReply>>>;

impl TerminalEventProxy {
    pub(super) fn new(
        size: Arc<Mutex<TerminalSize>>,
    ) -> (Self, PendingTerminalReplies, SharedTerminalNotices) {
        let pending_replies = Arc::new(Mutex::new(Vec::new()));
        let notices = SharedTerminalNotices::default();
        (
            Self {
                size,
                pending_replies: Arc::clone(&pending_replies),
                notices: Arc::clone(&notices),
            },
            pending_replies,
            notices,
        )
    }

    fn notices(&self) -> std::sync::MutexGuard<'_, TerminalNotices> {
        self.notices.lock().expect("terminal notices lock poisoned")
    }
}

impl EventListener for TerminalEventProxy {
    fn send_event(&self, event: Event) {
        let reply = match event {
            Event::Title(title) => {
                self.notices().record_title(sanitize_terminal_title(&title));
                return;
            }
            Event::ResetTitle => {
                self.notices().record_title(None);
                return;
            }
            Event::Bell => {
                let mut notices = self.notices();
                notices.bells = notices.bells.saturating_add(1);
                return;
            }
            Event::ClipboardStore(_, text) => {
                self.notices().record_clipboard(text);
                return;
            }
            Event::PtyWrite(text) => Some(PendingTerminalReply::Bytes(text.into_bytes())),
            Event::TextAreaSizeRequest(formatter) => {
                let size = *self.size.lock().expect("terminal size lock poisoned");
                Some(PendingTerminalReply::Bytes(
                    formatter(size.window_size()).into_bytes(),
                ))
            }
            Event::ColorRequest(index, formatter) => {
                Some(PendingTerminalReply::Color(index, formatter))
            }
            _ => None,
        };
        if let Some(reply) = reply {
            self.pending_replies
                .lock()
                .expect("terminal reply queue lock poisoned")
                .push(reply);
        }
    }
}

pub(super) fn flush_terminal_replies(
    terminal: &Arc<Mutex<Terminal>>,
    input: &TerminalInput,
    pending_replies: &PendingTerminalReplies,
) -> io::Result<()> {
    let replies = std::mem::take(
        &mut *pending_replies
            .lock()
            .expect("terminal reply queue lock poisoned"),
    );
    if replies.is_empty() {
        return Ok(());
    }
    let terminal = terminal.lock().expect("terminal state lock poisoned");
    let mut bytes = Vec::new();
    for reply in replies {
        match reply {
            PendingTerminalReply::Bytes(reply) => bytes.extend(reply),
            PendingTerminalReply::Color(index, formatter) => {
                let color = terminal.colors()[index].or_else(|| default_terminal_color(index));
                let Some(color) = color else {
                    continue;
                };
                bytes.extend(formatter(color).bytes());
            }
        }
    }
    drop(terminal);
    input.write_terminal_reply(bytes)
}

fn default_terminal_color(index: usize) -> Option<Rgb> {
    let value = match u8::try_from(index) {
        Ok(index) => default_indexed_color(index),
        Err(_) if index == NamedColor::Foreground as usize => DEFAULT_FOREGROUND_COLOR,
        Err(_) if index == NamedColor::Background as usize => DEFAULT_BACKGROUND_COLOR,
        Err(_) if index == NamedColor::Cursor as usize => DEFAULT_CURSOR_COLOR,
        Err(_) => return None,
    };
    Some(Rgb {
        r: (value >> 16) as u8,
        g: (value >> 8) as u8,
        b: value as u8,
    })
}
