//! Which agent runs in a terminal and what it is doing.
//!
//! The process table names the agent; the agent's own hooks report its state (ADR 0014).
//! Condr never classifies the screen. Until an agent reports, it is [`AgentState::Unknown`],
//! and it stays that way rather than being guessed at.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
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

// ---------------------------------------------------------------------------
// Process identification
// ---------------------------------------------------------------------------

/// What the process table knows about one candidate.
#[derive(Clone, Copy, Debug)]
pub struct ProcessInfo<'a> {
    pub name: &'a str,
    pub argv: Option<&'a [String]>,
}

/// The agent a single process is, if any. A process named after the agent (or whose
/// argv[0] is) matches directly; a runtime or shell (node, bun, python, sh, cmd, pwsh)
/// is looked through to the script or command it runs.
pub fn identify_agent_process(process: ProcessInfo<'_>) -> Option<AgentKind> {
    let argv0 = process
        .argv
        .and_then(|argv| argv.first())
        .map(String::as_str);
    if let Some(agent) =
        AgentKind::parse_label(process.name).or_else(|| argv0.and_then(AgentKind::parse_label))
    {
        return Some(agent);
    }
    let argv = process.argv?;
    if !is_runtime_or_shell(argv0.unwrap_or(process.name)) {
        return None;
    }
    wrapped_command(argv).and_then(agent_from_path_token)
}

/// The best agent among several processes of one job: a process named after the agent
/// beats one only found through a wrapper.
pub fn identify_agent_among<'a>(
    processes: impl IntoIterator<Item = ProcessInfo<'a>>,
) -> Option<AgentKind> {
    let mut wrapped = None;
    for process in processes {
        let Some(agent) = identify_agent_process(process) else {
            continue;
        };
        if AgentKind::parse_label(process.name).is_some() {
            return Some(agent);
        }
        wrapped.get_or_insert(agent);
    }
    wrapped
}

/// The first thing a runtime or shell was asked to run: its script argument, or the
/// first token of a `-c` / `/c` / `-Command` string. Flags are skipped; an inline
/// program (`-e`, `-m`) is nothing Condr can name.
fn wrapped_command(argv: &[String]) -> Option<&str> {
    let mut args = argv.iter().skip(1).map(String::as_str);
    while let Some(arg) = args.next() {
        let flag = arg.trim_matches('"').to_ascii_lowercase();
        match flag.as_str() {
            "-c" | "/c" | "/k" | "-command" | "/command" => {
                return args.next().and_then(|command| {
                    command
                        .trim_start()
                        .trim_start_matches(['&', '.'])
                        .split_whitespace()
                        .next()
                });
            }
            "-e" | "--eval" | "-p" | "--print" | "-m" | "-encodedcommand" | "-enc" => {
                return None;
            }
            "-file" | "-f" => return args.next(),
            "--" => return args.next(),
            // A `cmd`/`pwsh` switch, not an absolute Unix path.
            _ if flag.starts_with('-') || (flag.starts_with('/') && !flag[1..].contains('/')) => {}
            _ => return Some(arg),
        }
    }
    None
}

fn agent_from_path_token(token: &str) -> Option<AgentKind> {
    let path = token.trim_matches(['"', '\'']);
    if path.is_empty() {
        return None;
    }
    if let Some(agent) = AgentKind::parse_label(path) {
        return Some(agent);
    }
    let lower = path.replace('\\', "/").to_ascii_lowercase();
    if let Some(agent) = AgentKind::ALL
        .into_iter()
        .find(|agent| lower.contains(&format!("/node_modules/{}/", agent.package_path())))
    {
        return Some(agent);
    }
    // A launcher (a symlink in `~/.local/bin`, say) may resolve to the real agent.
    let resolved = std::fs::canonicalize(path).ok()?;
    AgentKind::parse_label(resolved.file_name()?.to_str()?)
}

fn is_runtime_or_shell(name: &str) -> bool {
    let name = normalized_lookup_name(path_basename(name));
    name.starts_with("python")
        || matches!(
            name.as_str(),
            "node" | "bun" | "sh" | "bash" | "zsh" | "fish" | "cmd" | "powershell" | "pwsh"
        )
}

fn normalized_lookup_name(name: &str) -> String {
    let mut name = name.trim().to_lowercase();
    for suffix in [".exe", ".cmd", ".bat", ".ps1", ".js"] {
        if name.ends_with(suffix) {
            name.truncate(name.len() - suffix.len());
            break;
        }
    }
    name
}

fn path_basename(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|component| !component.is_empty())
        .unwrap_or(path)
}

// ---------------------------------------------------------------------------
// Presence
// ---------------------------------------------------------------------------

/// The regular poll cadence.
pub const AGENT_POLL_INTERVAL: Duration = Duration::from_millis(300);
/// An `Unidentified` probe may be a transient table read failure; only this many in a
/// row mean the agent is gone. A bare shell means it at once.
const AGENT_MISS_CONFIRMATION_ATTEMPTS: u8 = 6;
const PROCESS_RECHECK_IDENTIFIED: Duration = Duration::from_secs(5);
const PROCESS_RECHECK_UNIDENTIFIED: Duration = Duration::from_millis(500);
/// How long after output or a foreground change an unidentified terminal keeps the fast
/// cadence; after that a quiet shell is only re-enumerated on the slow fallback.
const PROCESS_ACQUISITION_WINDOW: Duration = Duration::from_secs(8);
const PROCESS_RECHECK_QUIET: Duration = Duration::from_secs(30);
/// Activity after this much silence opens a new acquisition window.
const PROCESS_ACQUISITION_IDLE_RESET: Duration = Duration::from_secs(2);

/// What the process probe saw in the terminal's job.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessProbeResult {
    /// The job runs a known agent.
    Agent(AgentKind),
    /// Only the shell itself is left: an agent that was here has exited.
    ShellOnly,
    /// Other processes run, none of them a known agent (or the table was unreadable).
    Unidentified,
}

/// What the detector wants published after a tick.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentPublish {
    Nothing,
    Snapshot(AgentSnapshot),
    /// The agent is gone; the Pane is a plain shell again.
    Cleared,
}

/// Per-terminal presence: which agent the process table shows, and when to look again.
#[derive(Debug, Default)]
pub struct AgentDetector {
    agent: Option<AgentKind>,
    consecutive_misses: u8,
    last_process_probe: Option<Instant>,
    last_activity: Option<Instant>,
    acquisition_started: Option<Instant>,
}

impl AgentDetector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn agent(&self) -> Option<AgentKind> {
        self.agent
    }

    /// How long the caller should wait before the next tick.
    pub fn poll_interval(&self) -> Duration {
        AGENT_POLL_INTERVAL
    }

    /// Whether this tick should read the process table. Identified agents are
    /// re-checked every 5 s (a job change may force an earlier check through
    /// `force`); an unidentified terminal is checked every 500 ms while output or a
    /// foreground change (`activity`) is recent, then every 30 s until the next one.
    pub fn wants_process_probe(&mut self, now: Instant, force: bool, activity: bool) -> bool {
        if force || activity || self.last_activity.is_none() {
            // Continuous output does not slide the window; only activity after a
            // quiet spell opens a new one.
            let quiet = self
                .last_activity
                .is_none_or(|at| now.duration_since(at) >= PROCESS_ACQUISITION_IDLE_RESET);
            // A foreground-group change is a new job, so it always opens a window.
            if quiet || force {
                self.acquisition_started = Some(now);
            }
            self.last_activity = Some(now);
        }
        if force {
            return true;
        }
        let interval = if self.agent.is_some() {
            PROCESS_RECHECK_IDENTIFIED
        } else if self
            .acquisition_started
            .is_some_and(|at| now.duration_since(at) < PROCESS_ACQUISITION_WINDOW)
        {
            PROCESS_RECHECK_UNIDENTIFIED
        } else {
            PROCESS_RECHECK_QUIET
        };
        self.last_process_probe
            .is_none_or(|checked| now.duration_since(checked) >= interval)
    }

    /// Feeds a process probe. A fresh agent is announced as `Unknown`; an exited one is
    /// cleared. A different kind replacing the current one is a fresh agent.
    pub fn observe_process(&mut self, result: ProcessProbeResult, now: Instant) -> AgentPublish {
        self.last_process_probe = Some(now);
        match result {
            ProcessProbeResult::Agent(agent) => {
                self.consecutive_misses = 0;
                if self.agent == Some(agent) {
                    return AgentPublish::Nothing;
                }
                self.agent = Some(agent);
                AgentPublish::Snapshot(AgentSnapshot {
                    kind: agent,
                    state: AgentState::Unknown,
                })
            }
            ProcessProbeResult::ShellOnly | ProcessProbeResult::Unidentified => {
                if self.agent.is_none() {
                    self.consecutive_misses = 0;
                    return AgentPublish::Nothing;
                }
                let exited = match result {
                    ProcessProbeResult::ShellOnly => true,
                    _ => {
                        self.consecutive_misses = self.consecutive_misses.saturating_add(1);
                        self.consecutive_misses >= AGENT_MISS_CONFIRMATION_ATTEMPTS
                    }
                };
                if !exited {
                    return AgentPublish::Nothing;
                }
                self.agent = None;
                self.consecutive_misses = 0;
                // The shell is back in front: a replacement may start any moment.
                self.acquisition_started = Some(now);
                AgentPublish::Cleared
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info<'a>(name: &'a str, argv: &'a [String]) -> ProcessInfo<'a> {
        ProcessInfo {
            name,
            argv: Some(argv),
        }
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn identifies_agents_directly_and_through_wrappers() {
        assert_eq!(
            identify_agent_process(info("claude", &args(&["claude"]))),
            Some(AgentKind::Claude)
        );
        assert_eq!(
            identify_agent_process(info(
                "node",
                &args(&["node", "/usr/lib/node_modules/@openai/codex/bin/codex.js"])
            )),
            Some(AgentKind::Codex)
        );
        assert_eq!(
            identify_agent_process(info(
                "node",
                &args(&[
                    "node",
                    "--no-warnings",
                    "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"
                ])
            )),
            Some(AgentKind::Claude)
        );
        assert_eq!(
            identify_agent_process(info("node", &args(&["node", "/usr/local/bin/opencode"]))),
            Some(AgentKind::OpenCode)
        );
        assert_eq!(
            identify_agent_process(info(
                "cmd.exe",
                &args(&[
                    "cmd.exe",
                    "/d",
                    "/s",
                    "/c",
                    "\"C:\\Users\\me\\AppData\\Roaming\\npm\\claude.cmd\""
                ])
            )),
            Some(AgentKind::Claude)
        );
        assert_eq!(
            identify_agent_process(info(
                "pwsh.exe",
                &args(&["pwsh.exe", "-NoLogo", "-Command", "& codex --model o3"])
            )),
            Some(AgentKind::Codex)
        );
        assert_eq!(
            identify_agent_process(info("pwsh.exe", &args(&["pwsh.exe", "-NoLogo"]))),
            None
        );
        assert_eq!(
            identify_agent_process(info("python3", &args(&["python3", "-m", "claude"]))),
            None
        );
        assert_eq!(
            identify_agent_process(info("node", &args(&["node", "-e", "claude"]))),
            None
        );
        assert_eq!(
            identify_agent_process(info("vim", &args(&["vim", "claude"]))),
            None
        );
        assert_eq!(AgentKind::parse_label("Codex.exe"), Some(AgentKind::Codex));
        assert_eq!(
            AgentKind::parse_label("C:\\tools\\claude-code.cmd"),
            Some(AgentKind::Claude)
        );
    }

    #[test]
    fn the_process_named_after_the_agent_wins_over_wrappers() {
        let node = args(&["node", "/opt/node_modules/@openai/codex/bin/codex.js"]);
        let claude = args(&["claude"]);
        assert_eq!(
            identify_agent_among([info("node", &node), info("claude", &claude)]),
            Some(AgentKind::Claude)
        );
        assert_eq!(
            identify_agent_among([info("node", &node)]),
            Some(AgentKind::Codex)
        );
        assert_eq!(identify_agent_among([info("bash", &args(&["bash"]))]), None);
    }

    #[test]
    fn a_new_agent_is_unknown_until_it_reports_and_a_restart_is_a_new_agent() {
        let mut detector = AgentDetector::new();
        let now = Instant::now();
        assert_eq!(
            detector.observe_process(ProcessProbeResult::Agent(AgentKind::Codex), now),
            AgentPublish::Snapshot(AgentSnapshot {
                kind: AgentKind::Codex,
                state: AgentState::Unknown,
            })
        );
        assert_eq!(
            detector.observe_process(ProcessProbeResult::Agent(AgentKind::Codex), now),
            AgentPublish::Nothing
        );
        assert_eq!(
            detector.observe_process(ProcessProbeResult::ShellOnly, now),
            AgentPublish::Cleared
        );
        assert_eq!(detector.agent(), None);
        assert!(matches!(
            detector.observe_process(ProcessProbeResult::Agent(AgentKind::Codex), now),
            AgentPublish::Snapshot(_)
        ));
        // A different kind in the same job is a fresh agent, not a state change.
        assert_eq!(
            detector.observe_process(ProcessProbeResult::Agent(AgentKind::Claude), now),
            AgentPublish::Snapshot(AgentSnapshot {
                kind: AgentKind::Claude,
                state: AgentState::Unknown,
            })
        );
    }

    #[test]
    fn a_quiet_shell_leaves_the_fast_probe_cadence_until_something_happens() {
        let mut detector = AgentDetector::new();
        let start = Instant::now();
        assert!(detector.wants_process_probe(start, false, false));
        detector.observe_process(ProcessProbeResult::ShellOnly, start);
        let half_second = Duration::from_millis(500);
        assert!(detector.wants_process_probe(start + half_second, false, false));
        detector.observe_process(ProcessProbeResult::ShellOnly, start + half_second);

        let quiet = start + PROCESS_ACQUISITION_WINDOW + half_second;
        detector.observe_process(ProcessProbeResult::ShellOnly, quiet);
        assert!(!detector.wants_process_probe(quiet + half_second, false, false));
        assert!(detector.wants_process_probe(quiet + PROCESS_RECHECK_QUIET, false, false));

        // Output reopens the window; a foreground change probes at once.
        assert!(detector.wants_process_probe(quiet + half_second, false, true));
        detector.observe_process(ProcessProbeResult::ShellOnly, quiet + half_second);
        assert!(detector.wants_process_probe(quiet + half_second * 2, false, false));
        assert!(detector.wants_process_probe(quiet + half_second * 2, true, false));

        // Continuous output does not keep the window open past its 8 s.
        let mut detector = AgentDetector::new();
        let mut at = start;
        while at < start + PROCESS_ACQUISITION_WINDOW + half_second {
            detector.wants_process_probe(at, false, true);
            detector.observe_process(ProcessProbeResult::Unidentified, at);
            at += half_second;
        }
        assert!(!detector.wants_process_probe(at, false, true));
    }

    #[test]
    fn a_cleared_agent_reopens_the_fast_probe_window_for_its_replacement() {
        let mut detector = AgentDetector::new();
        let start = Instant::now();
        detector.wants_process_probe(start, false, false);
        detector.observe_process(ProcessProbeResult::Agent(AgentKind::Codex), start);
        // A long-running agent outlives the acquisition window it was found in.
        let gone = start + PROCESS_ACQUISITION_WINDOW * 4;
        assert!(detector.wants_process_probe(gone, false, false));
        assert_eq!(
            detector.observe_process(ProcessProbeResult::ShellOnly, gone),
            AgentPublish::Cleared
        );

        // A wrapper restarting the agent a second later is still on the fast cadence.
        let half_second = Duration::from_millis(500);
        assert!(detector.wants_process_probe(gone + half_second, false, false));
    }

    #[test]
    fn unidentified_probes_need_six_misses_but_a_bare_shell_needs_one() {
        let mut detector = AgentDetector::new();
        let now = Instant::now();
        detector.observe_process(ProcessProbeResult::Agent(AgentKind::Codex), now);
        for _ in 0..5 {
            assert_eq!(
                detector.observe_process(ProcessProbeResult::Unidentified, now),
                AgentPublish::Nothing
            );
        }
        assert_eq!(
            detector.observe_process(ProcessProbeResult::Unidentified, now),
            AgentPublish::Cleared
        );
    }
}
