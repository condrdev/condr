//! Which agent runs in a terminal and what it is doing.
//!
//! The process table names the agent; the agent's own hooks report its state (ADR 0014).
//! Condr never classifies the screen. Until an agent reports, it is [`AgentState::Unknown`],
//! and it stays that way rather than being guessed at.

mod detector;
mod event;
pub mod hook;
pub mod hooks;
mod process;

use process::{normalized_lookup_name, path_basename};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub use detector::{AgentDetector, AgentPublish, ProcessProbeResult};
pub use event::{AGENT_EVENT_OSC_PREFIX, AgentEvent, AgentEventKind};
pub(crate) use process::is_shell;
pub use process::{ProcessInfo, identify_agent_among, identify_agent_process};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Claude,
    Codex,
    OpenCode,
    Pi,
    Omp,
    Antigravity,
    Grok,
    Cursor,
    Copilot,
    Kimi,
    /// A kind a newer Server reported that this build does not know (ADR 0028): shown as
    /// a generic agent. Nothing detects or launches it, so it is not in [`Self::ALL`].
    Other,
}

impl AgentKind {
    pub const ALL: [Self; 10] = [
        Self::Claude,
        Self::Codex,
        Self::OpenCode,
        Self::Pi,
        Self::Omp,
        Self::Antigravity,
        Self::Grok,
        Self::Cursor,
        Self::Copilot,
        Self::Kimi,
    ];

    /// The canonical id: the CLI `--kind` value and the hook slug.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
            Self::Omp => "omp",
            Self::Antigravity => "antigravity",
            Self::Grok => "grok",
            Self::Cursor => "cursor",
            Self::Copilot => "copilot",
            Self::Kimi => "kimi",
            Self::Other => "other",
        }
    }

    /// What the GUI shows for the agent.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
            Self::Pi => "Pi",
            Self::Omp => "Oh My Pi",
            Self::Antigravity => "Antigravity CLI",
            Self::Grok => "Grok Build",
            Self::Cursor => "Cursor CLI",
            Self::Copilot => "GitHub Copilot",
            Self::Kimi => "Kimi Code",
            Self::Other => "Agent",
        }
    }

    /// Whether native hooks report before the first turn. Codex and Copilot defer
    /// SessionStart until prompted; Cursor omits it on resume; Antigravity has none
    /// in its documented contract. Kimi has no reliable status adapter yet.
    pub const fn reports_at_startup(self) -> bool {
        !matches!(
            self,
            Self::Codex
                | Self::Copilot
                | Self::Cursor
                | Self::Antigravity
                | Self::Kimi
                | Self::Other
        )
    }

    /// Resolves a program name, alias or path to an agent.
    pub fn parse_label(value: &str) -> Option<Self> {
        match normalized_lookup_name(path_basename(value)).as_str() {
            "claude" | "claude-code" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "opencode" => Some(Self::OpenCode),
            "pi" => Some(Self::Pi),
            "omp" | "oh-my-pi" => Some(Self::Omp),
            "agy" | "antigravity" => Some(Self::Antigravity),
            "grok" | "grok-build" => Some(Self::Grok),
            "cursor" | "cursor-agent" => Some(Self::Cursor),
            "copilot" | "github-copilot" => Some(Self::Copilot),
            "kimi" | "kimi-code" => Some(Self::Kimi),
            _ => None,
        }
    }

    /// The npm package the agent's node entry point lives under, for launchers that
    /// run `node .../node_modules/<package>/...` without the agent's name in argv.
    const fn package_paths(self) -> &'static [&'static str] {
        match self {
            Self::Claude => &["@anthropic-ai/claude-code"],
            Self::Codex => &["@openai/codex"],
            Self::OpenCode => &["opencode-ai"],
            Self::Pi => &[
                "@earendil-works/pi-coding-agent",
                "@mariozechner/pi-coding-agent",
            ],
            Self::Omp => &["@oh-my-pi/pi-coding-agent"],
            Self::Copilot => &["@github/copilot"],
            Self::Antigravity | Self::Grok | Self::Cursor | Self::Kimi | Self::Other => &[],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AgentState {
    /// Plain shell or unrecognized program, or the agent has not reported yet.
    Unknown,
    /// Agent finished its turn, nothing happening.
    Idle,
    /// Agent is actively working.
    Working,
    /// Agent needs human input.
    Blocked,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentSnapshot {
    /// The native CLI conversation identifier, reported by a hook or retained for resume.
    pub session_id: Option<String>,
    pub kind: AgentKind,
    pub state: AgentState,
    /// What a `Blocked` agent waits for, from the hook that blocked it (ADR 0024);
    /// `None` in every other state or when the hook had nothing to say.
    pub blocked_on: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentDisplayState {
    Unknown,
    Idle,
    Working,
    Blocked,
    Done,
}

impl AgentDisplayState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::Done => "done",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentTracker {
    state: AgentState,
    unseen_completion: bool,
}

impl AgentTracker {
    pub const fn new(state: AgentState) -> Self {
        Self {
            state,
            unseen_completion: false,
        }
    }

    pub fn update(&mut self, state: AgentState, visible: bool) {
        if matches!(self.state, AgentState::Working | AgentState::Blocked)
            && state == AgentState::Idle
            && !visible
        {
            self.unseen_completion = true;
        }
        self.state = state;
        if visible {
            self.unseen_completion = false;
        }
    }

    pub fn mark_seen(&mut self) {
        self.unseen_completion = false;
    }

    pub const fn display_state(self) -> AgentDisplayState {
        match (self.state, self.unseen_completion) {
            (AgentState::Idle, true) => AgentDisplayState::Done,
            (AgentState::Unknown, _) => AgentDisplayState::Unknown,
            (AgentState::Idle, _) => AgentDisplayState::Idle,
            (AgentState::Working, _) => AgentDisplayState::Working,
            (AgentState::Blocked, _) => AgentDisplayState::Blocked,
        }
    }
}

/// A native conversation to reopen in a fresh shell after a Server restart.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentResume {
    pub kind: AgentKind,
    pub session_id: String,
}

impl AgentResume {
    pub fn args(&self) -> Vec<String> {
        if self.kind == AgentKind::Copilot {
            return vec![format!("--resume={}", self.session_id)];
        }
        let flag = match self.kind {
            AgentKind::Claude => "--resume",
            AgentKind::Codex => "resume",
            AgentKind::OpenCode => "--session",
            AgentKind::Pi | AgentKind::Omp | AgentKind::Kimi => "--session",
            AgentKind::Antigravity => "--conversation",
            AgentKind::Grok | AgentKind::Cursor => "--resume",
            AgentKind::Copilot => unreachable!("handled above"),
            // The wire drops a resume for a kind this build does not know (ADR 0028).
            AgentKind::Other => return Vec::new(),
        };
        vec![flag.into(), self.session_id.clone()]
    }
}

/// Native IDs are opaque tokens, never paths, shell syntax, or CLI options.
pub(crate) fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id.as_bytes()[0].is_ascii_alphanumeric()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
