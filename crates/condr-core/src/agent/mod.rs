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
}

impl AgentKind {
    pub const ALL: [Self; 3] = [Self::Claude, Self::Codex, Self::OpenCode];

    /// The canonical id: the CLI `--kind` value, the hook slug and the executable name.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }

    /// What the GUI shows for the agent.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
        }
    }

    /// Whether the agent's hooks say anything before its first turn. Codex (0.146 and
    /// 0.153) fires `SessionStart` together with the first `UserPromptSubmit`, so a fresh
    /// Codex is `Unknown` until prompted; `agent start` and the first `agent prompt`
    /// treat that `Unknown` as ready for it.
    pub const fn reports_at_startup(self) -> bool {
        !matches!(self, Self::Codex)
    }

    /// Resolves a program name, alias or path to an agent.
    pub fn parse_label(value: &str) -> Option<Self> {
        match normalized_lookup_name(path_basename(value)).as_str() {
            "claude" | "claude-code" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "opencode" => Some(Self::OpenCode),
            _ => None,
        }
    }

    /// The npm package the agent's node entry point lives under, for launchers that
    /// run `node .../node_modules/<package>/...` without the agent's name in argv.
    const fn package_path(self) -> &'static str {
        match self {
            Self::Claude => "@anthropic-ai/claude-code",
            Self::Codex => "@openai/codex",
            Self::OpenCode => "opencode-ai",
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
        let flag = match self.kind {
            AgentKind::Claude => "--resume",
            AgentKind::Codex => "resume",
            AgentKind::OpenCode => "--session",
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
