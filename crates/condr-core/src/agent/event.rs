use super::*;

/// The OSC 777 sentinel a hook writes: `ESC ] 777 ; notify ; condr://agent ; {json} BEL`.
pub const AGENT_EVENT_OSC_PREFIX: &str = "777;notify;condr://agent;";

const AGENT_EVENT_WIRE_VERSION: u32 = 1;

/// What an agent's hook reported, in Condr's vocabulary. Each agent's installer maps its
/// native hook names onto these.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentEventKind {
    SessionStart,
    PromptSubmit,
    PermissionRequest,
    QuestionAsked,
    ToolStart,
    ToolComplete,
    Stop,
    Interrupt,
    StopFailure,
}

impl AgentEventKind {
    pub const ALL: [Self; 9] = [
        Self::SessionStart,
        Self::PromptSubmit,
        Self::PermissionRequest,
        Self::QuestionAsked,
        Self::ToolStart,
        Self::ToolComplete,
        Self::Stop,
        Self::Interrupt,
        Self::StopFailure,
    ];

    /// The wire spelling, which is also what `condr agent-hook` takes on its command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::SessionStart => "session-start",
            Self::PromptSubmit => "prompt-submit",
            Self::PermissionRequest => "permission-request",
            Self::QuestionAsked => "question-asked",
            Self::ToolStart => "tool-start",
            Self::ToolComplete => "tool-complete",
            Self::Stop => "stop",
            Self::Interrupt => "interrupt",
            Self::StopFailure => "stop-failure",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentEvent {
    #[serde(rename = "v")]
    version: u32,
    pub agent: AgentKind,
    pub event: AgentEventKind,
    /// Why a session started (`startup`, `resume`, `clear`, `compact`); only `compact`
    /// matters, because it happens mid-turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub session_id: Option<String>,
    /// Native turn identifier, used when a CLI dispatches completion hooks after the
    /// next prompt has already started (Grok).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<String>,
    /// What a `permission-request` or `question-asked` waits for: the tool (and command)
    /// or the question, already bounded by `bound_detail` (ADR 0024). Absent elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl AgentEvent {
    pub fn new(
        agent: AgentKind,
        event: AgentEventKind,
        source: Option<String>,
        session_id: Option<String>,
    ) -> Self {
        Self {
            version: AGENT_EVENT_WIRE_VERSION,
            agent,
            event,
            source,
            session_id,
            prompt_id: None,
            detail: None,
        }
    }

    /// The complete OSC sequence a hook writes to its controlling terminal.
    pub fn encode(&self) -> Vec<u8> {
        let json = serde_json::to_string(self).expect("agent event serializes");
        format!("\x1b]{AGENT_EVENT_OSC_PREFIX}{json}\x07").into_bytes()
    }

    /// Decodes an OSC payload (everything between `ESC ]` and the terminator).
    pub fn decode(payload: &[u8]) -> Option<Self> {
        let json = payload.strip_prefix(AGENT_EVENT_OSC_PREFIX.as_bytes())?;
        let event: Self = serde_json::from_slice(json).ok()?;
        (event.version == AGENT_EVENT_WIRE_VERSION
            && event
                .session_id
                .as_deref()
                .is_none_or(super::valid_session_id)
            && event.prompt_id.as_deref().is_none_or(valid_prompt_id)
            && event.detail.as_deref().is_none_or(valid_detail))
        .then_some(event)
    }

    /// The state after this event, from `state`. Turn boundaries map to Idle and
    /// Working; a request for the user maps to Blocked; a tool starting or completing
    /// maps to Working, which is what ends a Blocked once the user answered. A session
    /// start caused by compaction happens mid-turn and changes nothing.
    pub fn apply(&self, state: AgentState) -> AgentState {
        match self.event {
            AgentEventKind::SessionStart if self.source.as_deref() == Some("compact") => state,
            AgentEventKind::SessionStart
            | AgentEventKind::Stop
            | AgentEventKind::Interrupt
            | AgentEventKind::StopFailure => AgentState::Idle,
            AgentEventKind::PromptSubmit
            | AgentEventKind::ToolStart
            | AgentEventKind::ToolComplete => AgentState::Working,
            AgentEventKind::PermissionRequest | AgentEventKind::QuestionAsked => {
                AgentState::Blocked
            }
        }
    }
}

pub(super) fn valid_prompt_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
}

/// Characters a detail may carry; the bound keeps it, the session ID and the JSON
/// framing together well under the PTY scanner's OSC cap.
pub const MAX_DETAIL_CHARS: usize = 200;

fn valid_detail(detail: &str) -> bool {
    !detail.is_empty()
        && detail.chars().count() <= MAX_DETAIL_CHARS
        && !detail.chars().any(char::is_control)
}

/// Makes native text fit a detail: control characters and runs of whitespace become one
/// space, and anything past `MAX_DETAIL_CHARS` is cut behind an ellipsis. Empty in, none out.
pub fn bound_detail(raw: &str) -> Option<String> {
    let mut detail = String::new();
    let mut space = true;
    for ch in raw.chars() {
        if ch.is_control() || ch.is_whitespace() {
            if !space {
                detail.push(' ');
                space = true;
            }
        } else {
            detail.push(ch);
            space = false;
        }
    }
    let detail = detail.trim_end();
    if detail.is_empty() {
        return None;
    }
    if detail.chars().count() <= MAX_DETAIL_CHARS {
        return Some(detail.to_owned());
    }
    let mut cut: String = detail.chars().take(MAX_DETAIL_CHARS - 1).collect();
    cut.push('…');
    Some(cut)
}
