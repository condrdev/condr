use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AgentKind {
    Claude,
    Codex,
}

impl AgentKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AgentState {
    Unknown,
    Idle,
    Working,
    Blocked,
}

impl AgentState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentSnapshot {
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

pub fn identify_agent_process(name: &str, argv: &[String]) -> Option<AgentKind> {
    identify_name(name)
        .or_else(|| argv.first().and_then(|argument| identify_name(argument)))
        .or_else(|| {
            let runtime = normalized_name(name);
            matches!(
                runtime.as_str(),
                "node" | "bun" | "python" | "python3" | "pwsh" | "powershell" | "cmd"
            )
            .then(|| {
                argv.iter()
                    .skip(1)
                    .find_map(|argument| identify_name(argument))
            })
            .flatten()
        })
}

pub fn classify_agent(kind: AgentKind, bottom_text: &str, previous: AgentState) -> AgentState {
    let text = bottom_text.to_lowercase();
    if is_transcript_viewer(kind, &text) {
        return previous;
    }
    if is_blocked(kind, &text) {
        AgentState::Blocked
    } else if is_working(kind, &text) {
        AgentState::Working
    } else {
        AgentState::Idle
    }
}

fn identify_name(value: &str) -> Option<AgentKind> {
    match normalized_name(value).as_str() {
        "claude" | "claude-code" => Some(AgentKind::Claude),
        "codex" => Some(AgentKind::Codex),
        _ => None,
    }
}

fn normalized_name(value: &str) -> String {
    let file_name = Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(value)
        .to_ascii_lowercase();
    let without_script = [".exe", ".cmd", ".bat", ".ps1", ".js"]
        .into_iter()
        .find_map(|suffix| file_name.strip_suffix(suffix))
        .unwrap_or(&file_name);
    without_script.to_owned()
}

fn is_transcript_viewer(kind: AgentKind, text: &str) -> bool {
    match kind {
        AgentKind::Claude => text.contains("showing detailed transcript"),
        AgentKind::Codex => text.contains("pgup/pgdn to") && text.contains("home/end to jump"),
    }
}

fn is_blocked(kind: AgentKind, text: &str) -> bool {
    let common = [
        "[y/n]",
        "do you want to",
        "would you like to",
        "waiting for permission",
    ];
    common.iter().any(|pattern| text.contains(pattern))
        || match kind {
            AgentKind::Claude => {
                text.contains("do you want to proceed?")
                    || (text.contains("esc to cancel")
                        && (text.contains("enter to confirm") || text.contains("enter to select")))
            }
            AgentKind::Codex => {
                text.contains("action required")
                    || text.contains("allow command?")
                    || text.contains("press enter to confirm or esc to cancel")
                    || text.contains("enter to submit answer")
                    || text.contains("enter to submit all")
            }
        }
}

fn is_working(_kind: AgentKind, text: &str) -> bool {
    text.contains("esc to interrupt") && !text.contains("conversation interrupted")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_identity_handles_native_and_script_wrapped_agents() {
        assert_eq!(
            identify_agent_process("claude.exe", &[]),
            Some(AgentKind::Claude)
        );
        assert_eq!(
            identify_agent_process("node", &["node".into(), "/opt/openai/bin/codex.js".into()]),
            Some(AgentKind::Codex)
        );
        assert_eq!(identify_agent_process("pwsh", &["pwsh".into()]), None);
        assert_eq!(
            identify_agent_process("sleep", &["codex".into(), "10".into()]),
            Some(AgentKind::Codex)
        );
    }

    #[test]
    fn classification_prioritizes_blockers_and_preserves_transcript_state() {
        assert_eq!(
            classify_agent(
                AgentKind::Codex,
                "Allow command?\npress enter to confirm or esc to cancel",
                AgentState::Working,
            ),
            AgentState::Blocked
        );
        assert_eq!(
            classify_agent(
                AgentKind::Claude,
                "Working (12s · esc to interrupt)",
                AgentState::Idle,
            ),
            AgentState::Working
        );
        assert_eq!(
            classify_agent(
                AgentKind::Codex,
                "pgup/pgdn to scroll, home/end to jump",
                AgentState::Working,
            ),
            AgentState::Working
        );
        assert_eq!(
            classify_agent(AgentKind::Claude, "❯", AgentState::Working),
            AgentState::Idle
        );
    }

    #[test]
    fn unseen_idle_completion_is_presented_as_done_until_seen() {
        let mut tracker = AgentTracker::new(AgentState::Working);
        tracker.update(AgentState::Idle, false);
        assert_eq!(tracker.display_state(), AgentDisplayState::Done);
        tracker.mark_seen();
        assert_eq!(tracker.display_state(), AgentDisplayState::Idle);

        tracker.update(AgentState::Working, false);
        tracker.update(AgentState::Idle, true);
        assert_eq!(tracker.display_state(), AgentDisplayState::Idle);
    }
}
