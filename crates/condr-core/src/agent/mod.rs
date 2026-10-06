//! Which agent runs in a terminal and what it is doing.
//!
//! The process table names the agent; the agent's own hooks report its state (ADR 0014).
//! Condr never classifies the screen. Until an agent reports, it is [`AgentState::Unknown`],
//! and it stays that way rather than being guessed at.

mod antigravity;
mod claude;
mod codex;
mod copilot;
mod cursor;
mod detector;
mod event;
mod grok;
pub mod hook;
pub mod hooks;
mod kimi;
mod omp;
mod opencode;
mod pi;
mod process;

use process::{normalized_lookup_name, path_basename};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub use detector::{AgentDetector, AgentPublish, ProcessProbeResult};
pub use event::{AGENT_EVENT_OSC_PREFIX, AgentEvent, AgentEventKind};
pub(crate) use process::is_shell;
pub use process::{ProcessInfo, identify_agent_among, identify_agent_process};

/// Everything Condr knows about one supported agent CLI (ADR 0035). Each agent's module
/// holds its one `SPEC`; no field has a default, so an agent added without a thought for
/// one does not compile.
#[derive(Debug)]
pub struct AgentSpec {
    /// The canonical id: the CLI `--kind` value and the hook slug.
    pub id: &'static str,
    /// What the GUI shows for the agent.
    pub label: &'static str,
    /// Its mark's file under the repository's `assets/agents/`.
    pub mark: &'static str,
    /// The colour its published mark carries; `None` follows the surrounding text.
    pub brand_color: Option<u32>,
    /// The interactive command Condr looks for on PATH and launches.
    pub executable: &'static str,
    /// Other names its process goes by, besides its id and executable.
    pub aliases: &'static [&'static str],
    /// The npm packages its node entry point lives under, for launchers that run
    /// `node .../node_modules/<package>/...` without the agent's name in argv.
    pub packages: &'static [&'static str],
    /// The arguments that reopen a conversation, `{id}` standing for its session ID.
    pub resume: &'static [&'static str],
    /// Whether its hooks report before the first turn. One that does not is ready once
    /// its process is identified, and its first prompt goes in blind.
    pub reports_at_startup: bool,
    pub support: AgentSupport,
}

impl AgentSpec {
    /// Its hook integration; `None` for an agent recognized only.
    pub fn hooks(&self) -> Option<&hooks::HookSpec> {
        match &self.support {
            AgentSupport::Full(hooks) => Some(hooks),
            AgentSupport::RecognitionOnly(_) => None,
        }
    }
}

/// How far Condr supports an agent (ADR 0035).
#[derive(Debug)]
pub enum AgentSupport {
    /// Its hooks report a turn starting and the main agent's turn ending, so its
    /// `Working` and `Idle` can be trusted.
    Full(hooks::HookSpec),
    /// The process table names it, but its hooks cannot report a state worth trusting,
    /// so none are installed and it stays `Unknown`. Carries the reason.
    RecognitionOnly(&'static str),
}

/// A kind this build does not know (ADR 0028): nothing detects, launches or resumes it.
const OTHER: AgentSpec = AgentSpec {
    id: "other",
    label: "Agent",
    mark: "",
    brand_color: None,
    executable: "",
    aliases: &[],
    packages: &[],
    resume: &[],
    reports_at_startup: false,
    support: AgentSupport::RecognitionOnly("this build does not know that agent"),
};

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
    /// a generic agent. Nothing detects or launches it, so it is not in [`Self::ALL`]. It
    /// stays last, which the spec table's test relies on.
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

    /// The one `match` on the kind: everything else asks its spec.
    pub const fn spec(self) -> &'static AgentSpec {
        match self {
            Self::Claude => &claude::SPEC,
            Self::Codex => &codex::SPEC,
            Self::OpenCode => &opencode::SPEC,
            Self::Pi => &pi::SPEC,
            Self::Omp => &omp::SPEC,
            Self::Antigravity => &antigravity::SPEC,
            Self::Grok => &grok::SPEC,
            Self::Cursor => &cursor::SPEC,
            Self::Copilot => &copilot::SPEC,
            Self::Kimi => &kimi::SPEC,
            Self::Other => &OTHER,
        }
    }

    /// The canonical id: the CLI `--kind` value and the hook slug.
    pub const fn id(self) -> &'static str {
        self.spec().id
    }

    /// What the GUI shows for the agent.
    pub const fn label(self) -> &'static str {
        self.spec().label
    }

    /// Resolves a program name, alias or path to an agent.
    pub fn parse_label(value: &str) -> Option<Self> {
        let name = normalized_lookup_name(path_basename(value));
        Self::ALL.into_iter().find(|kind| {
            let spec = kind.spec();
            name == spec.id || name == spec.executable || spec.aliases.contains(&name.as_str())
        })
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
        self.kind
            .spec()
            .resume
            .iter()
            .map(|arg| arg.replace("{id}", &self.session_id))
            .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `ALL` lists every kind but `Other`, in declaration order, and each spec names its
    /// kind back and has a mark the GUI can draw.
    #[test]
    fn every_kind_has_a_spec_that_names_it() {
        assert_eq!(AgentKind::ALL.len(), AgentKind::Other as usize);
        let marks = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/agents");
        let mut ids = std::collections::BTreeSet::new();
        for (index, kind) in AgentKind::ALL.into_iter().enumerate() {
            assert_eq!(kind as usize, index, "{kind:?} is out of order in ALL");
            let spec = kind.spec();
            assert!(ids.insert(spec.id), "{} names two kinds", spec.id);
            for name in [spec.id, spec.executable].iter().chain(spec.aliases) {
                assert_eq!(AgentKind::parse_label(name), Some(kind), "{name}");
            }
            assert!(marks.join(spec.mark).is_file(), "{} has no mark", spec.id);
            assert_eq!(
                spec.resume
                    .iter()
                    .filter(|arg| arg.contains("{id}"))
                    .count(),
                1,
                "{} resumes without its session ID",
                spec.id
            );
        }
        assert_eq!(AgentKind::parse_label("other"), None);
    }
}
