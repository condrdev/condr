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
            if runtime == "cmd" {
                cmd_command(argv).and_then(identify_command_name)
            } else if matches!(
                runtime.as_str(),
                "node" | "bun" | "python" | "python3" | "pwsh" | "powershell"
            ) {
                argv.iter()
                    .skip(1)
                    .find_map(|argument| identify_name(argument))
            } else {
                None
            }
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
    let file_name = value
        .rsplit(['/', '\\'])
        .find(|component| !component.is_empty())
        .unwrap_or(value)
        .trim_matches('"')
        .to_ascii_lowercase();
    let without_script = [".exe", ".cmd", ".bat", ".ps1", ".js"]
        .into_iter()
        .find_map(|suffix| file_name.strip_suffix(suffix))
        .unwrap_or(&file_name);
    without_script.to_owned()
}

fn cmd_command(argv: &[String]) -> Option<&str> {
    let mut arguments = argv.iter().skip(1);
    while let Some(argument) = arguments.next() {
        if matches!(argument.to_ascii_lowercase().as_str(), "/c" | "/k") {
            return arguments.next().map(String::as_str);
        }
    }
    None
}

fn identify_command_name(command: &str) -> Option<AgentKind> {
    let command = command.trim_start();
    let executable = if let Some(quoted) = command.strip_prefix('"') {
        quoted.split_once('"').map(|(executable, _)| executable)?
    } else {
        command.split_whitespace().next()?
    };
    identify_name(executable)
}

fn is_transcript_viewer(kind: AgentKind, text: &str) -> bool {
    let marker = match kind {
        AgentKind::Claude => text.rfind("showing detailed transcript"),
        AgentKind::Codex => (text.contains("pgup/pgdn to") && text.contains("home/end to jump"))
            .then(|| text.rfind("home/end to jump"))
            .flatten(),
    };
    marker.is_some_and(|marker| last_prompt_marker(text).is_none_or(|prompt| marker > prompt))
}

fn is_blocked(kind: AgentKind, text: &str) -> bool {
    let prompt = last_prompt_marker(text);
    let current = prompt.map_or(text, |prompt| &text[prompt..]);
    let output = current
        .lines()
        .enumerate()
        .filter(|(index, _)| prompt.is_none() || *index != 0)
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n");
    if output.contains("[y/n]")
        || output.contains("waiting for permission")
        || ((output.contains("do you want to") || output.contains("would you like to"))
            && output.contains("yes"))
    {
        return true;
    }
    match kind {
        AgentKind::Claude => {
            output.contains("do you want to proceed?")
                || (output.contains("esc to cancel")
                    && (output.contains("enter to confirm") || output.contains("enter to select")))
        }
        AgentKind::Codex => [
            "action required",
            "allow command?",
            "press enter to confirm or esc to cancel",
            "enter to submit answer",
            "enter to submit all",
        ]
        .iter()
        .any(|pattern| output.contains(pattern)),
    }
}

fn is_working(kind: AgentKind, text: &str) -> bool {
    let recent_lines = match kind {
        AgentKind::Claude => 5,
        AgentKind::Codex => 3,
    };
    let lines = text.lines().collect::<Vec<_>>();
    let Some(working) = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .rev()
        .take(recent_lines)
        .find_map(|(index, line)| match kind {
            AgentKind::Claude => (line.contains("esc to interrupt")
                && !line.trim_start().starts_with(['›', '❯']))
            .then_some(index),
            AgentKind::Codex => codex_working_line(line).then_some(index),
        })
    else {
        return false;
    };
    !lines
        .iter()
        .skip(working + 1)
        .any(|line| line.contains("conversation interrupted") || matches!(line.trim(), "›" | "❯"))
}

fn codex_working_line(line: &str) -> bool {
    let line = line.trim_end();
    let Some(status) = line
        .strip_prefix("• working (")
        .or_else(|| line.strip_prefix("◦ working ("))
    else {
        return false;
    };
    let Some(close) = status.rfind(')') else {
        return false;
    };
    let suffix = status[close + 1..].trim_start();
    status[..close].contains("esc to interrupt")
        && (suffix.is_empty() || suffix == "·" || suffix.starts_with("· "))
}

fn last_prompt_marker(text: &str) -> Option<usize> {
    text.match_indices('\n')
        .map(|(index, _)| index + 1)
        .chain(std::iter::once(0))
        .filter(|&start| is_user_prompt_at(text, start))
        .max()
}

fn is_user_prompt_at(text: &str, start: usize) -> bool {
    let Some(line) = text[start..].lines().next() else {
        return false;
    };
    if !line.trim_start().starts_with(['›', '❯']) {
        return false;
    }
    let preceding = &text[..start];
    let following = &text[start + line.len()..];
    let choice = line
        .trim_start()
        .strip_prefix(['›', '❯'])
        .map(str::trim_start)
        .is_some_and(|choice| {
            choice
                .chars()
                .next()
                .is_some_and(|first| first.is_ascii_digit())
                || ["yes", "no"].into_iter().any(|word| {
                    choice.strip_prefix(word).is_some_and(|rest| {
                        rest.chars()
                            .next()
                            .is_none_or(|next| !next.is_ascii_alphanumeric() && next != '_')
                    })
                })
        });
    let preceding_question = preceding
        .lines()
        .rev()
        .take(3)
        .any(|line| line.contains("do you want to proceed?"));
    let permission_menu = (following.contains("esc to cancel")
        && (following.contains("enter to confirm") || following.contains("enter to select")))
        || (choice && preceding_question);
    !permission_menu
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
            identify_agent_process(
                "cmd.exe",
                &[
                    "cmd.exe".into(),
                    "/D".into(),
                    "/C".into(),
                    r#"C:\Users\condr\AppData\Roaming\npm\codex.cmd --model gpt-5"#.into(),
                ],
            ),
            Some(AgentKind::Codex)
        );
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
    fn classification_ignores_state_markers_before_the_current_prompt() {
        assert_eq!(
            classify_agent(
                AgentKind::Codex,
                "◦ Working (12s · esc to interrupt)\n\n›\ngpt-5.6-sol",
                AgentState::Working,
            ),
            AgentState::Idle
        );
        assert_eq!(
            classify_agent(
                AgentKind::Codex,
                "Allow command?\npress enter to confirm or esc to cancel\n\n› Use /skills",
                AgentState::Blocked,
            ),
            AgentState::Idle
        );
        assert_eq!(
            classify_agent(
                AgentKind::Codex,
                "pgup/pgdn to scroll, home/end to jump\n\n› Use /skills",
                AgentState::Working,
            ),
            AgentState::Idle
        );
        assert_eq!(
            classify_agent(
                AgentKind::Codex,
                "■ Conversation interrupted\n◦ Working (1s · esc to interrupt)",
                AgentState::Idle,
            ),
            AgentState::Working
        );
        assert_eq!(
            classify_agent(
                AgentKind::Claude,
                "❯ Do you want to refactor this?\n◦ Working (1s · esc to interrupt)",
                AgentState::Idle,
            ),
            AgentState::Working
        );
        assert_eq!(
            classify_agent(
                AgentKind::Codex,
                "› Explain what [y/n] means\ngpt-5.6-sol",
                AgentState::Blocked,
            ),
            AgentState::Idle
        );
    }

    #[test]
    fn classification_keeps_live_blockers_and_rejects_prompt_text_as_working() {
        assert_eq!(
            classify_agent(
                AgentKind::Codex,
                "◦ Working (4s · esc to interrupt)\n› Run the command\ndo you want to continue? [y/n]",
                AgentState::Working,
            ),
            AgentState::Blocked
        );
        assert_eq!(
            classify_agent(
                AgentKind::Claude,
                "Do you want to proceed?\n❯ 1. Yes\n  2. No",
                AgentState::Idle,
            ),
            AgentState::Blocked
        );
        assert_eq!(
            classify_agent(
                AgentKind::Claude,
                "Do you want to proceed?\n❯ 1. Yes\n  2. No\nesc to cancel",
                AgentState::Idle,
            ),
            AgentState::Blocked
        );
        assert_eq!(
            classify_agent(
                AgentKind::Claude,
                "Do you want to proceed?\n❯ Yes\n  No\nesc to cancel\nenter to confirm",
                AgentState::Idle,
            ),
            AgentState::Blocked
        );
        assert_eq!(
            classify_agent(
                AgentKind::Codex,
                "› Explain the text ◦ Working (1m · esc to interrupt)\ngpt-5.6-sol",
                AgentState::Working,
            ),
            AgentState::Idle
        );
        assert_eq!(
            classify_agent(
                AgentKind::Codex,
                "› Do you want to refactor this?\n◦ Working (1s · esc to interrupt)\ngpt-5.6-sol",
                AgentState::Idle,
            ),
            AgentState::Working
        );
        assert_eq!(
            classify_agent(
                AgentKind::Claude,
                "❯ Yes, do you want to refactor this?\n◦ Working (1s · esc to interrupt)",
                AgentState::Idle,
            ),
            AgentState::Working
        );
        assert_eq!(
            classify_agent(
                AgentKind::Claude,
                "❯ 1. Explain what [y/n] means\nclaude ready",
                AgentState::Blocked,
            ),
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
