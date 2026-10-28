//! Which agent runs in a terminal and what it is doing.
//!
//! The process table names the agent; the agent itself reports its state, through the
//! Program Status Protocol (OSC 7501, ADR 0044) or the hooks Condr installs (ADR 0014).
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
#[cfg(feature = "runtime")]
mod process;
pub mod program_status;

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub use detector::{AgentDetector, AgentPublish, ProcessProbeResult};
#[cfg(feature = "runtime")]
pub(crate) use event::AGENT_EVENT_OSC_PREFIX;
pub use event::{AgentEvent, AgentEventKind};
#[cfg(feature = "runtime")]
pub(crate) use process::is_shell;
#[cfg(feature = "runtime")]
pub use process::{ProcessInfo, identify_agent_among, identify_agent_process};
#[cfg(feature = "runtime")]
pub(crate) use program_status::ProgramStatusReport;
pub use program_status::{BlockedKind, ProgramState, ProgramStatus};

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
    /// Whether it reads kitty keyboard reports it never negotiated. On a Windows Server
    /// such an agent still receives Cmd+C, which has no legacy encoding, when its probe
    /// for the protocol failed: under ConPTY, conhost answers the probe's closing DA1
    /// before Condr answers `CSI ? u`. The process table cannot tell when an editor the
    /// agent started has the terminal, so that editor receives the report too; set it
    /// only for an agent seen to decode one.
    pub reads_kitty_keys: bool,
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
    reads_kitty_keys: false,
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

/// What an agent last said it is doing (ADR 0014, ADR 0044). The Program Status
/// Protocol's five states map one to one; hooks report `Done` for a turn that ended,
/// `Error` for one that failed, and `Idle` for a session start or an interrupt.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AgentState {
    /// Plain shell or unrecognized program, or the agent has not reported yet.
    Unknown,
    /// Resting at its prompt, awaiting the next command.
    Idle,
    /// Agent is actively working.
    Working,
    /// Agent needs human input.
    Blocked,
    /// A turn finished and the agent has not seen the user come back for the result.
    Done,
    /// A turn failed and the agent halted.
    Error,
}

impl AgentState {
    /// What `agent wait` waits for unless told otherwise: the agent sits at its prompt
    /// or waits for the user.
    pub const SETTLED: [Self; 4] = [Self::Idle, Self::Done, Self::Blocked, Self::Error];

    /// The agent sits at its prompt, where a new prompt goes in.
    pub const fn at_prompt(self) -> bool {
        matches!(self, Self::Idle | Self::Done | Self::Error)
    }

    /// The wire spelling, which the CLI prints and takes for `--until`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentSnapshot {
    /// The native CLI conversation identifier, reported by a hook or retained for resume.
    pub session_id: Option<String>,
    pub kind: AgentKind,
    pub state: AgentState,
    /// What a `Blocked` agent waits for, or why an `Error` one halted, from the report
    /// that put it there (ADR 0024, ADR 0044); `None` in every other state or when the
    /// agent had nothing to say.
    pub detail: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentDisplayState {
    Unknown,
    Idle,
    Working,
    Blocked,
    Done,
    Error,
}

impl AgentDisplayState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Error => "error",
        }
    }

    /// How an Agent shows (ADR 0052): its state, except that a `Done` someone has looked
    /// at, on any Client, shows as `Idle`. The Server keeps whether anyone has, so a
    /// Client that restarts or connects later sees the same.
    pub const fn of(state: AgentState, unseen: bool) -> Self {
        match (state, unseen) {
            (AgentState::Done, false) | (AgentState::Idle, _) => Self::Idle,
            (AgentState::Done, true) => Self::Done,
            (AgentState::Unknown, _) => Self::Unknown,
            (AgentState::Working, _) => Self::Working,
            (AgentState::Blocked, _) => Self::Blocked,
            (AgentState::Error, _) => Self::Error,
        }
    }
}

/// Why an Agent's change calls its person: the desktop notification and the Companion's
/// push fire on the same changes (ADR 0047).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentAlert {
    Finished,
    Failed,
    /// `updated`: it was Blocked already and now waits for something else, which replaces
    /// the earlier alert without asking for attention again.
    NeedsInput {
        updated: bool,
    },
}

impl AgentAlert {
    /// What the change from `previous` to `next` calls for, if anything. An Idle straight
    /// after work is a finish too: an agent that reports no `done` (Codex before 0.164)
    /// ends its turns that way. Blocked counts from anywhere else, since a login prompt
    /// arrives straight from Idle.
    pub fn between(previous: Option<&AgentSnapshot>, next: &AgentSnapshot) -> Option<Self> {
        use AgentState::{Blocked, Done, Error, Idle, Working};
        let previous = previous?;
        match (previous.state, next.state) {
            (Working | Blocked, Done | Idle) => Some(Self::Finished),
            (Working | Blocked, Error) => Some(Self::Failed),
            (Blocked, Blocked) => (next.detail.is_some() && next.detail != previous.detail)
                .then_some(Self::NeedsInput { updated: true }),
            (_, Blocked) => Some(Self::NeedsInput { updated: false }),
            _ => None,
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

pub(crate) fn normalized_lookup_name(name: &str) -> String {
    let mut name = name.trim().to_lowercase();
    for suffix in [".exe", ".cmd", ".bat", ".ps1", ".js"] {
        if name.ends_with(suffix) {
            name.truncate(name.len() - suffix.len());
            break;
        }
    }
    name
}

pub(crate) fn path_basename(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|component| !component.is_empty())
        .unwrap_or(path)
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

    /// The changes that notify the desktop and push to the Companion (ADR 0047).
    #[test]
    fn alerts_fire_on_finish_failure_and_each_new_question() {
        use AgentAlert::{Failed, Finished, NeedsInput};
        use AgentState::{Blocked, Done, Error, Idle, Unknown, Working};
        let agent = |state, detail: Option<&str>| AgentSnapshot {
            session_id: None,
            kind: AgentKind::Claude,
            state,
            detail: detail.map(str::to_owned),
        };
        let asked = Some("Bash: rm -rf target");
        for (previous, next, alert) in [
            (
                Some(agent(Working, None)),
                agent(Done, None),
                Some(Finished),
            ),
            (
                Some(agent(Working, None)),
                agent(Idle, None),
                Some(Finished),
            ),
            (
                Some(agent(Blocked, asked)),
                agent(Done, None),
                Some(Finished),
            ),
            (
                Some(agent(Working, None)),
                agent(Error, asked),
                Some(Failed),
            ),
            (
                Some(agent(Blocked, asked)),
                agent(Error, None),
                Some(Failed),
            ),
            (
                Some(agent(Idle, None)),
                agent(Blocked, asked),
                Some(NeedsInput { updated: false }),
            ),
            (
                Some(agent(Working, None)),
                agent(Blocked, None),
                Some(NeedsInput { updated: false }),
            ),
            (
                Some(agent(Blocked, asked)),
                agent(Blocked, Some("Write: notes.md")),
                Some(NeedsInput { updated: true }),
            ),
            (Some(agent(Blocked, asked)), agent(Blocked, asked), None),
            (Some(agent(Blocked, asked)), agent(Blocked, None), None),
            (None, agent(Blocked, asked), None),
            (Some(agent(Idle, None)), agent(Working, None), None),
            (Some(agent(Unknown, None)), agent(Idle, None), None),
            (Some(agent(Done, None)), agent(Idle, None), None),
            (Some(agent(Idle, None)), agent(Error, asked), None),
        ] {
            assert_eq!(
                AgentAlert::between(previous.as_ref(), &next),
                alert,
                "{:?} -> {:?}",
                previous.as_ref().map(|agent| agent.state),
                next.state
            );
        }
    }
}
