//! Which agent runs in a terminal and what it is doing.
//!
//! Identification and the detection engine are ported from herdr (`src/detect/mod.rs`,
//! `src/pane/agent_detection.rs` and the presence logic in `src/pane.rs`, commit
//! 5158adab, Apache-2.0). Condr keeps them behind the same shape it always had: a
//! process probe names the agent, a manifest classifies the bottom of the screen plus
//! the OSC title, and [`AgentDetector`] applies herdr's hysteresis before a state is
//! published. See `docs/research/herdr-agent-detection-v0.8.2.md`.

pub mod manifest;

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub use manifest::DetectionInput;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AgentKind {
    Pi,
    Claude,
    Codex,
    Gemini,
    Cursor,
    Devin,
    Antigravity,
    Cline,
    Omp,
    Mastracode,
    OpenCode,
    GithubCopilot,
    Kimi,
    Kiro,
    Droid,
    Amp,
    Grok,
    Hermes,
    Kilo,
    Qodercli,
    Qwen,
    Maki,
    Muse,
}

impl AgentKind {
    pub const ALL: [Self; 23] = [
        Self::Pi,
        Self::Claude,
        Self::Codex,
        Self::Gemini,
        Self::Cursor,
        Self::Devin,
        Self::Antigravity,
        Self::Cline,
        Self::Omp,
        Self::Mastracode,
        Self::OpenCode,
        Self::GithubCopilot,
        Self::Kimi,
        Self::Kiro,
        Self::Droid,
        Self::Amp,
        Self::Grok,
        Self::Hermes,
        Self::Kilo,
        Self::Qodercli,
        Self::Qwen,
        Self::Maki,
        Self::Muse,
    ];

    /// Agents with a bundled detection manifest; the rest are identified only.
    pub const SCREEN_MANIFEST_AGENTS: [Self; 21] = [
        Self::Pi,
        Self::Claude,
        Self::Codex,
        Self::Gemini,
        Self::Cursor,
        Self::Devin,
        Self::Antigravity,
        Self::Cline,
        Self::OpenCode,
        Self::GithubCopilot,
        Self::Kimi,
        Self::Kiro,
        Self::Droid,
        Self::Amp,
        Self::Grok,
        Self::Hermes,
        Self::Kilo,
        Self::Qodercli,
        Self::Qwen,
        Self::Maki,
        Self::Muse,
    ];

    /// herdr's canonical id: the manifest file name and the `agent-detection` override.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Cursor => "cursor",
            Self::Devin => "devin",
            Self::Antigravity => "agy",
            Self::Cline => "cline",
            Self::Omp => "omp",
            Self::Mastracode => "mastracode",
            Self::OpenCode => "opencode",
            Self::GithubCopilot => "copilot",
            Self::Kimi => "kimi",
            Self::Kiro => "kiro",
            Self::Droid => "droid",
            Self::Amp => "amp",
            Self::Grok => "grok",
            Self::Hermes => "hermes",
            Self::Kilo => "kilo",
            Self::Qodercli => "qodercli",
            Self::Qwen => "qwen",
            Self::Maki => "maki",
            Self::Muse => "muse",
        }
    }

    /// What the GUI shows for the agent.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pi => "Pi",
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Gemini => "Gemini",
            Self::Cursor => "Cursor",
            Self::Devin => "Devin",
            Self::Antigravity => "Antigravity",
            Self::Cline => "Cline",
            Self::Omp => "Omp",
            Self::Mastracode => "Mastra Code",
            Self::OpenCode => "OpenCode",
            Self::GithubCopilot => "Copilot",
            Self::Kimi => "Kimi",
            Self::Kiro => "Kiro",
            Self::Droid => "Droid",
            Self::Amp => "Amp",
            Self::Grok => "Grok",
            Self::Hermes => "Hermes",
            Self::Kilo => "Kilo",
            Self::Qodercli => "Qoder",
            Self::Qwen => "Qwen",
            Self::Maki => "Maki",
            Self::Muse => "Muse",
        }
    }

    /// Resolves a program name, alias or path to an agent (herdr `parse_agent_label`).
    pub fn parse_label(value: &str) -> Option<Self> {
        let name = normalized_lookup_name(value);
        lookup_agent(&name)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AgentState {
    /// Plain shell or unrecognized program, or the agent has not been classified yet.
    Unknown,
    /// Agent finished, prompt visible, nothing happening.
    Idle,
    /// Agent is actively working.
    Working,
    /// Agent needs human input.
    Blocked,
}

/// A manifest verdict for one screen. The `visible_*` flags say the state came from
/// live UI chrome rather than scrollback text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentDetection {
    pub state: AgentState,
    /// The screen is an agent-owned viewer (transcript, picker); keep the old state.
    pub skip_state_update: bool,
    pub visible_idle: bool,
    pub visible_blocker: bool,
    pub visible_working: bool,
}

impl AgentDetection {
    /// A known agent whose manifest matched nothing is idle.
    pub const fn idle_fallback() -> Self {
        Self {
            state: AgentState::Idle,
            skip_state_update: false,
            visible_idle: false,
            visible_blocker: false,
            visible_working: false,
        }
    }

    /// What an exited agent process reports before its identity is dropped.
    pub const fn process_exited() -> Self {
        Self {
            state: AgentState::Idle,
            skip_state_update: false,
            visible_idle: true,
            visible_blocker: false,
            visible_working: false,
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

/// The GUI's view of one agent: the Server state plus whether a completion happened
/// while the Pane was not being looked at.
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

/// Classifies one screen for a known agent.
pub fn detect_agent(kind: AgentKind, input: DetectionInput<'_>) -> AgentDetection {
    manifest::detect(kind, input)
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

/// The agent a single process is, if any (herdr `normalized_process_name` +
/// `identify_agent`). Wrapper runtimes (node, python, cmd, pwsh, sh) are looked
/// through to the script or command they run.
pub fn identify_agent_process(process: ProcessInfo<'_>) -> Option<AgentKind> {
    AgentKind::parse_label(&normalized_process_name(process))
}

/// The best agent among several processes of one job (herdr `identify_agent_in_job`
/// after the leader check): a process whose own name is the agent beats one only
/// found through a wrapper, which beats a generic runtime.
pub fn identify_agent_among<'a>(
    processes: impl IntoIterator<Item = ProcessInfo<'a>>,
) -> Option<AgentKind> {
    let mut best: Option<(u8, AgentKind)> = None;
    for process in processes {
        let candidate = normalized_process_name(process);
        let Some(agent) = AgentKind::parse_label(&candidate) else {
            continue;
        };
        let score = process_priority(process, &candidate);
        if best.is_none_or(|(best_score, _)| best_score < score) {
            best = Some((score, agent));
        }
    }
    best.map(|(_, agent)| agent)
}

fn lookup_agent(name: &str) -> Option<AgentKind> {
    let name = path_basename(name);
    match name {
        "pi" => Some(AgentKind::Pi),
        "claude" | "claude-code" => Some(AgentKind::Claude),
        "codex" => Some(AgentKind::Codex),
        "gemini" => Some(AgentKind::Gemini),
        "cursor" | "cursor-agent" => Some(AgentKind::Cursor),
        "devin" | "devin-cli" | "devin cli" => Some(AgentKind::Devin),
        "agy" | "antigravity" | "antigravity-cli" => Some(AgentKind::Antigravity),
        "cline" => Some(AgentKind::Cline),
        "omp" => Some(AgentKind::Omp),
        "mastracode" | "mastra-code" | "mastra code" => Some(AgentKind::Mastracode),
        "opencode" | "opencode2" | "open-code" => Some(AgentKind::OpenCode),
        "copilot" | "github-copilot" | "ghcs" => Some(AgentKind::GithubCopilot),
        "kimi" | "kimi-code" | "kimi code" => Some(AgentKind::Kimi),
        "kiro" | "kiro-cli" => Some(AgentKind::Kiro),
        "droid" => Some(AgentKind::Droid),
        "amp" | "amp-local" => Some(AgentKind::Amp),
        "grok" | "grok-build" => Some(AgentKind::Grok),
        "hermes" | "hermes-agent" => Some(AgentKind::Hermes),
        "kilo" | "kilo-code" | "kilo code" => Some(AgentKind::Kilo),
        "qodercli" | "qoderclicn" | "qoder" | "qodercn" => Some(AgentKind::Qodercli),
        "qwen" | "qwen-code" | "qwen code" => Some(AgentKind::Qwen),
        "maki" => Some(AgentKind::Maki),
        "muse" | "muse-code" | "muse-cli" => Some(AgentKind::Muse),
        _ if is_muse_versioned_binary(name) => Some(AgentKind::Muse),
        _ => None,
    }
}

/// Muse's launcher execs `muse-bin-<version>`; require a digit after the prefix so
/// `muse-binary` stays unmatched.
fn is_muse_versioned_binary(name: &str) -> bool {
    path_basename(name)
        .strip_prefix("muse-bin-")
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
}

fn normalized_process_name(process: ProcessInfo<'_>) -> String {
    let argv0 = process
        .argv
        .and_then(|argv| argv.first().map(String::as_str));
    let effective = argv0.unwrap_or(process.name);
    let lower_effective = effective.to_lowercase();

    if is_generic_runtime_or_shell(&lower_effective)
        && let Some(wrapped) = wrapped_agent_name_from_runtime_argv(&lower_effective, process.argv)
    {
        return wrapped;
    }

    if AgentKind::parse_label(effective).is_some() {
        return effective.to_string();
    }

    if let Some(runtime) = argv0 {
        let runtime_name = normalized_lookup_name(path_basename(runtime));
        if matches!(runtime_name.as_str(), "node" | "bun")
            && let Some(wrapped) = wrapped_agent_name_from_runtime_argv(runtime, process.argv)
            && AgentKind::parse_label(&wrapped) == Some(AgentKind::Qwen)
        {
            return wrapped;
        }
    }

    if let Some(wrapped) = argv0.and_then(agent_name_from_path_token) {
        return wrapped;
    }

    effective.to_string()
}

fn wrapped_agent_name_from_runtime_argv(runtime: &str, argv: Option<&[String]>) -> Option<String> {
    let argv = argv?;
    let runtime_name = normalized_lookup_name(path_basename(runtime));
    match runtime_name.as_str() {
        "node" => cursor_agent_name_from_bundled_node_argv(argv)
            .or_else(|| script_arg_agent_name(argv, &["-e", "--eval", "-p", "--print"], &[])),
        "bun" => script_arg_agent_name(argv, &["-e", "--eval", "-p", "--print"], &[]),
        name if is_python_runtime(name) => script_arg_agent_name(argv, &["-c"], &["-m"]),
        "sh" | "bash" | "zsh" | "fish" => script_arg_agent_name(argv, &["-c"], &[]),
        "cmd" => windows_cmd_arg_agent_name(argv),
        "powershell" | "pwsh" => powershell_arg_agent_name(argv),
        _ => None,
    }
}

fn cursor_agent_name_from_bundled_node_argv(argv: &[String]) -> Option<String> {
    let (runtime_parent, runtime_name) = path_parent_and_basename(argv.first()?)?;
    let (script_parent, script_name) = path_parent_and_basename(argv.get(1)?)?;
    if !runtime_name.eq_ignore_ascii_case("node.exe")
        || !script_name.eq_ignore_ascii_case("index.js")
        || !runtime_parent.eq_ignore_ascii_case(script_parent)
    {
        return None;
    }
    let mut tail = runtime_parent
        .rsplit(['/', '\\'])
        .filter(|component| !component.is_empty());
    let (Some(version), Some(versions), Some(package)) = (tail.next(), tail.next(), tail.next())
    else {
        return None;
    };
    (package.eq_ignore_ascii_case("cursor-agent")
        && versions.eq_ignore_ascii_case("versions")
        && !version.trim().is_empty())
    .then(|| AgentKind::Cursor.id().to_string())
}

fn path_parent_and_basename(path: &str) -> Option<(&str, &str)> {
    let split = path.rfind(['/', '\\'])?;
    let parent = path[..split].trim_end_matches(['/', '\\']);
    let basename = &path[split + 1..];
    (!parent.is_empty() && !basename.is_empty()).then_some((parent, basename))
}

fn windows_cmd_arg_agent_name(argv: &[String]) -> Option<String> {
    let mut args = argv.iter().skip(1);
    while let Some(arg) = args.next() {
        let flag = arg.trim_matches('"').to_lowercase();
        match flag.as_str() {
            "/c" | "/k" => {
                return args
                    .next()
                    .and_then(|command| command_text_agent_name(command));
            }
            "/d" | "/s" | "/q" | "/a" | "/u" | "/e:on" | "/e:off" | "/f:on" | "/f:off"
            | "/v:on" | "/v:off" => continue,
            _ => {}
        }
    }
    None
}

fn powershell_arg_agent_name(argv: &[String]) -> Option<String> {
    let mut args = argv.iter().skip(1);
    while let Some(arg) = args.next() {
        let flag = arg.trim_matches('"').to_lowercase();
        match flag.as_str() {
            "-file" | "-f" | "/file" => {
                return args
                    .next()
                    .and_then(|path| agent_name_from_path_token(path));
            }
            "-command" | "-c" | "/command" | "/c" => {
                return args
                    .next()
                    .and_then(|command| command_text_agent_name(command));
            }
            "-encodedcommand" | "-enc" | "/encodedcommand" | "/enc" => return None,
            "-configurationname" | "-executionpolicy" | "-outputformat" | "-psconsolefile"
            | "-version" | "-windowstyle" | "-workingdirectory" => {
                let _ = args.next();
            }
            _ if flag.starts_with('-') || flag.starts_with('/') => {}
            _ => return agent_name_from_path_token(arg),
        }
    }
    None
}

fn command_text_agent_name(command: &str) -> Option<String> {
    let mut rest = command;
    while let Some((token, next)) = command_text_token(rest) {
        let token = token.trim();
        if token.eq_ignore_ascii_case("&")
            || token.eq_ignore_ascii_case(".")
            || token.eq_ignore_ascii_case("call")
        {
            rest = next;
            continue;
        }
        return agent_name_from_path_token(token);
    }
    None
}

fn command_text_token(input: &str) -> Option<(&str, &str)> {
    let input = input.trim_start();
    let first = input.chars().next()?;
    if first == '"' || first == '\'' {
        let start = first.len_utf8();
        if let Some(end) = input[start..].find(first) {
            let end = start + end;
            return Some((&input[start..end], &input[end + first.len_utf8()..]));
        }
        return Some((&input[start..], ""));
    }
    let end = input.find(char::is_whitespace).unwrap_or(input.len());
    Some((&input[..end], &input[end..]))
}

fn script_arg_agent_name(
    argv: &[String],
    eval_flags: &[&str],
    module_flags: &[&str],
) -> Option<String> {
    let mut args = argv.iter().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--" {
            return args
                .next()
                .and_then(|token| agent_name_from_path_token(token));
        }
        if flag_matches(arg, eval_flags) || flag_matches(arg, module_flags) {
            return None;
        }
        if arg.starts_with('-') {
            if option_takes_value(arg) {
                let _ = args.next();
            }
            continue;
        }
        return agent_name_from_path_token(arg);
    }
    None
}

fn flag_matches(arg: &str, flags: &[&str]) -> bool {
    flags
        .iter()
        .any(|flag| arg == *flag || short_flag_payload(arg, flag) || long_flag_value(arg, flag))
}

fn short_flag_payload(arg: &str, flag: &str) -> bool {
    flag.starts_with('-')
        && !flag.starts_with("--")
        && arg.starts_with(flag)
        && arg.len() > flag.len()
}

fn long_flag_value(arg: &str, flag: &str) -> bool {
    flag.starts_with("--")
        && arg
            .strip_prefix(flag)
            .is_some_and(|rest| rest.starts_with('='))
}

fn option_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "-r" | "--require"
            | "--loader"
            | "--import"
            | "--experimental-loader"
            | "--inspect-port"
            | "-W"
            | "-X"
            | "-S"
            | "-L"
            | "-o"
    )
}

fn agent_name_from_path_token(token: &str) -> Option<String> {
    let trimmed = token.trim_matches(|c| matches!(c, '"' | '\''));
    if trimmed.is_empty() || trimmed.starts_with('-') {
        return None;
    }
    agent_name_from_basename(path_basename(trimmed))
        .or_else(|| agent_name_from_known_package_path(trimmed))
        .or_else(|| resolved_agent_name_from_path_token(trimmed))
}

fn agent_name_from_known_package_path(path: &str) -> Option<String> {
    let raw_components: Vec<&str> = path
        .split(['/', '\\'])
        .filter(|component| !component.is_empty())
        .collect();
    let ends_with = |suffix: &[&str]| {
        raw_components.len() >= suffix.len()
            && raw_components[raw_components.len() - suffix.len()..]
                .iter()
                .zip(suffix)
                .all(|(actual, expected)| actual.eq_ignore_ascii_case(expected))
    };
    if ends_with(&[
        "node_modules",
        "@earendil-works",
        "pi-coding-agent",
        "dist",
        "cli.js",
    ]) || ends_with(&[
        "node_modules",
        "@earendil-works",
        "pi-coding-agent",
        "dist",
        "bundle",
        "cli.js",
    ]) {
        return Some(AgentKind::Pi.id().to_string());
    }
    let components: Vec<String> = raw_components
        .into_iter()
        .map(normalized_lookup_name)
        .collect();
    for window in components.windows(5) {
        if window == ["node_modules", "@qwen-code", "qwen-code", "dist", "index"] {
            return Some(AgentKind::Qwen.id().to_string());
        }
    }
    for window in components.windows(4) {
        if window == ["node_modules", "mastracode", "dist", "cli"] {
            return Some(AgentKind::Mastracode.id().to_string());
        }
    }
    None
}

/// A launcher path (a symlink in `~/.local/bin`, say) may resolve to the real agent.
fn resolved_agent_name_from_path_token(token: &str) -> Option<String> {
    let path = std::path::Path::new(token);
    if path.components().count() < 2 {
        return None;
    }
    let resolved = std::fs::canonicalize(path).ok()?;
    let basename = resolved.file_name()?.to_str()?;
    agent_name_from_basename(basename)
}

fn agent_name_from_basename(basename: &str) -> Option<String> {
    AgentKind::parse_label(basename).map(|agent| agent.id().to_string())
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

fn process_priority(process: ProcessInfo<'_>, normalized_name: &str) -> u8 {
    let lower_name = normalized_name.to_lowercase();
    if lower_name != process.name.to_lowercase() {
        return 3;
    }
    if !is_generic_runtime_or_shell(&lower_name) {
        return 2;
    }
    1
}

fn is_generic_runtime_or_shell(name: &str) -> bool {
    let name = normalized_lookup_name(path_basename(name));
    is_python_runtime(&name)
        || matches!(
            name.as_str(),
            "sh" | "bash"
                | "zsh"
                | "fish"
                | "tmux"
                | "node"
                | "bun"
                | "cmd"
                | "powershell"
                | "pwsh"
        )
}

fn is_python_runtime(name: &str) -> bool {
    name == "python"
        || name.strip_prefix("python").is_some_and(|version| {
            !version.is_empty()
                && version
                    .split('.')
                    .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
        })
}

// ---------------------------------------------------------------------------
// Detection state machine (herdr `pane/agent_detection.rs` + presence logic)
// ---------------------------------------------------------------------------

/// How often the detector wants to be polled while a Working → Idle transition is held.
pub const AGENT_PENDING_IDLE_RECHECK: Duration = Duration::from_millis(100);
/// The regular poll cadence.
pub const AGENT_POLL_INTERVAL: Duration = Duration::from_millis(300);
const AGENT_PENDING_IDLE_CONFIRMATIONS: u8 = 3;
const AGENT_PENDING_IDLE_CAP: Duration = Duration::from_millis(700);
const AGENT_STARTUP_GRACE_WINDOW: Duration = Duration::from_secs(3);
const AGENT_MISS_CONFIRMATION_ATTEMPTS: u8 = 6;
const PROCESS_RECHECK_IDENTIFIED: Duration = Duration::from_secs(5);
const PROCESS_RECHECK_UNIDENTIFIED: Duration = Duration::from_millis(500);
/// How long after output or a foreground change an unidentified terminal keeps the fast
/// cadence; after that a quiet shell is only re-enumerated on the slow fallback, as herdr
/// does after its acquisition window.
const PROCESS_ACQUISITION_WINDOW: Duration = Duration::from_secs(8);
const PROCESS_RECHECK_QUIET: Duration = Duration::from_secs(30);
/// Activity after this much silence opens a new acquisition window (herdr's idle reset).
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

#[derive(Debug, Default)]
struct PendingIdleConfirmation {
    started_at: Option<Instant>,
    confirmations: u8,
}

impl PendingIdleConfirmation {
    fn active(&self) -> bool {
        self.started_at.is_some()
    }

    fn clear(&mut self) {
        self.started_at = None;
        self.confirmations = 0;
    }

    /// Working → plain Idle is held until it repeats or times out, so a redraw between
    /// turns does not flicker the state (and the GUI's Done badge).
    fn should_hold(
        &mut self,
        previous: AgentState,
        next: &AgentDetection,
        agent_changed: bool,
        process_exited: bool,
        now: Instant,
    ) -> bool {
        let is_working_to_plain_idle = previous == AgentState::Working
            && next.state == AgentState::Idle
            && !next.visible_idle
            && !next.visible_blocker
            && !agent_changed
            && !process_exited;
        if !is_working_to_plain_idle {
            self.clear();
            return false;
        }
        let Some(started_at) = self.started_at else {
            self.started_at = Some(now);
            self.confirmations = 0;
            return true;
        };
        if now.duration_since(started_at) >= AGENT_PENDING_IDLE_CAP {
            self.clear();
            return false;
        }
        self.confirmations = self.confirmations.saturating_add(1);
        if self.confirmations >= AGENT_PENDING_IDLE_CONFIRMATIONS {
            self.clear();
            return false;
        }
        true
    }
}

/// Per-terminal detection state: which agent is present, what was last published,
/// and the timers that keep the published state from flickering.
#[derive(Debug)]
pub struct AgentDetector {
    agent: Option<AgentKind>,
    consecutive_misses: u8,
    /// The probe said the agent's process is gone; publish Idle once, then clear.
    exit_pending: bool,
    published: Option<AgentState>,
    last_visible_idle: bool,
    last_visible_blocker: bool,
    last_visible_working: bool,
    pending_idle: PendingIdleConfirmation,
    startup_grace_until: Option<Instant>,
    last_process_probe: Option<Instant>,
    last_activity: Option<Instant>,
    acquisition_started: Option<Instant>,
    /// OSC evidence left by the previous agent; ignored until it changes.
    stale_osc_title: Option<String>,
    stale_osc_progress: Option<String>,
}

impl Default for AgentDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentDetector {
    pub fn new() -> Self {
        Self {
            agent: None,
            consecutive_misses: 0,
            exit_pending: false,
            published: None,
            last_visible_idle: false,
            last_visible_blocker: false,
            last_visible_working: false,
            pending_idle: PendingIdleConfirmation::default(),
            startup_grace_until: None,
            last_process_probe: None,
            last_activity: None,
            acquisition_started: None,
            stale_osc_title: None,
            stale_osc_progress: None,
        }
    }

    pub fn agent(&self) -> Option<AgentKind> {
        self.agent
    }

    /// How long the caller should wait before the next tick.
    pub fn poll_interval(&self) -> Duration {
        if self.pending_idle.active() {
            AGENT_PENDING_IDLE_RECHECK
        } else {
            AGENT_POLL_INTERVAL
        }
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
            if quiet {
                self.acquisition_started = Some(now);
            }
            self.last_activity = Some(now);
        }
        if force || self.exit_pending {
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

    /// Feeds a process probe. Returns what to publish right away: a fresh agent is
    /// announced as `Unknown` until its startup grace ends, an exited agent's Idle is
    /// published before the identity is cleared, and a clear follows.
    pub fn observe_process(
        &mut self,
        result: ProcessProbeResult,
        osc_title: &str,
        osc_progress: &str,
        now: Instant,
    ) -> AgentPublish {
        self.last_process_probe = Some(now);
        let previous = self.agent;
        match result {
            ProcessProbeResult::Agent(agent) => {
                self.consecutive_misses = 0;
                // The same kind reappearing after its Idle went out is a new process, not
                // the old one; it must not inherit the published Idle or the OSC evidence.
                let replaces_exited = std::mem::take(&mut self.exit_pending);
                if previous == Some(agent) && !replaces_exited {
                    return AgentPublish::Nothing;
                }
                self.agent = Some(agent);
                self.start_agent(previous, osc_title, osc_progress, now);
                AgentPublish::Snapshot(AgentSnapshot {
                    kind: agent,
                    state: AgentState::Unknown,
                })
            }
            ProcessProbeResult::ShellOnly | ProcessProbeResult::Unidentified => {
                let Some(agent) = previous else {
                    self.consecutive_misses = 0;
                    return AgentPublish::Nothing;
                };
                if self.exit_pending {
                    // The Idle went out on the previous tick; now the identity goes. What
                    // the old agent left in the OSC title must not classify its successor.
                    self.reset_agent();
                    self.stale_osc_title = Some(osc_title.to_owned());
                    self.stale_osc_progress = Some(osc_progress.to_owned());
                    return AgentPublish::Cleared;
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
                self.exit_pending = true;
                self.startup_grace_until = None;
                self.pending_idle.clear();
                if self.published == Some(AgentState::Idle) {
                    self.reset_agent();
                    self.stale_osc_title = Some(osc_title.to_owned());
                    self.stale_osc_progress = Some(osc_progress.to_owned());
                    return AgentPublish::Cleared;
                }
                self.published = Some(AgentState::Idle);
                AgentPublish::Snapshot(AgentSnapshot {
                    kind: agent,
                    state: AgentState::Idle,
                })
            }
        }
    }

    fn start_agent(
        &mut self,
        previous: Option<AgentKind>,
        osc_title: &str,
        osc_progress: &str,
        now: Instant,
    ) {
        self.published = Some(AgentState::Unknown);
        self.last_visible_idle = false;
        self.last_visible_blocker = false;
        self.last_visible_working = false;
        self.pending_idle.clear();
        self.startup_grace_until = Some(now + AGENT_STARTUP_GRACE_WINDOW);
        // A replacement agent must not inherit the previous one's OSC evidence; a first
        // acquisition keeps what its own process already emitted, minus whatever a
        // cleared predecessor left behind.
        if previous.is_some() {
            self.stale_osc_title = Some(osc_title.to_owned());
            self.stale_osc_progress = Some(osc_progress.to_owned());
        }
    }

    fn reset_agent(&mut self) {
        self.agent = None;
        self.consecutive_misses = 0;
        self.exit_pending = false;
        self.published = None;
        self.last_visible_idle = false;
        self.last_visible_blocker = false;
        self.last_visible_working = false;
        self.pending_idle.clear();
        self.startup_grace_until = None;
    }

    /// Whether the screen is worth reading this tick: an agent is present, its startup
    /// grace has passed, and either the screen changed or a held transition needs
    /// re-checking. `screen_changed` is false when the terminal revision is unchanged.
    pub fn wants_screen(&mut self, screen_changed: bool, now: Instant) -> bool {
        if self.agent.is_none() || self.exit_pending {
            return false;
        }
        if let Some(until) = self.startup_grace_until {
            if now < until {
                self.pending_idle.clear();
                return false;
            }
            self.startup_grace_until = None;
            self.pending_idle.clear();
            return true;
        }
        screen_changed || self.pending_idle.active() || self.published != Some(AgentState::Idle)
    }

    /// Feeds a screen and returns the state to publish, if it changed.
    pub fn observe_screen(&mut self, input: DetectionInput<'_>, now: Instant) -> AgentPublish {
        let Some(agent) = self.agent else {
            return AgentPublish::Nothing;
        };
        let osc_title = if self.stale_osc_title.as_deref() == Some(input.osc_title) {
            ""
        } else {
            self.stale_osc_title = None;
            input.osc_title
        };
        let osc_progress = if self.stale_osc_progress.as_deref() == Some(input.osc_progress) {
            ""
        } else {
            self.stale_osc_progress = None;
            input.osc_progress
        };
        let detection = manifest::detect(
            agent,
            DetectionInput {
                screen: input.screen,
                osc_title,
                osc_progress,
            },
        );
        if detection.skip_state_update {
            self.pending_idle.clear();
            return AgentPublish::Nothing;
        }
        let previous = self.published.unwrap_or(AgentState::Unknown);
        if self
            .pending_idle
            .should_hold(previous, &detection, false, false, now)
        {
            return AgentPublish::Nothing;
        }
        let visible_idle = detection.visible_idle && detection.state == AgentState::Idle;
        let visible_blocker = detection.visible_blocker && detection.state == AgentState::Blocked;
        let visible_working = detection.visible_working && detection.state == AgentState::Working;
        let changed = detection.state != previous
            || visible_idle != self.last_visible_idle
            || visible_blocker != self.last_visible_blocker
            || visible_working != self.last_visible_working;
        self.last_visible_idle = visible_idle;
        self.last_visible_blocker = visible_blocker;
        self.last_visible_working = visible_working;
        if !changed || Some(detection.state) == self.published {
            self.published = Some(detection.state);
            return AgentPublish::Nothing;
        }
        self.published = Some(detection.state);
        AgentPublish::Snapshot(AgentSnapshot {
            kind: agent,
            state: detection.state,
        })
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
            identify_agent_process(info("pwsh.exe", &args(&["pwsh.exe", "-NoLogo"]))),
            None
        );
        assert_eq!(
            identify_agent_process(info("python3", &args(&["python3", "-m", "something"]))),
            None
        );
        assert_eq!(AgentKind::parse_label("Codex.exe"), Some(AgentKind::Codex));
        assert_eq!(
            AgentKind::parse_label("muse-bin-0.1.0"),
            Some(AgentKind::Muse)
        );
    }

    #[test]
    fn the_process_named_after_the_agent_wins_over_wrappers() {
        let node = args(&["node", "/opt/claude/cli.js"]);
        let claude = args(&["claude"]);
        assert_eq!(
            identify_agent_among([info("node", &node), info("claude", &claude)]),
            Some(AgentKind::Claude)
        );
        assert_eq!(identify_agent_among([info("bash", &args(&["bash"]))]), None);
    }

    #[test]
    fn detector_announces_then_holds_working_to_idle() {
        let mut detector = AgentDetector::new();
        let start = Instant::now();
        assert_eq!(
            detector.observe_process(ProcessProbeResult::Agent(AgentKind::Codex), "", "", start),
            AgentPublish::Snapshot(AgentSnapshot {
                kind: AgentKind::Codex,
                state: AgentState::Unknown,
            })
        );
        // Startup grace: no screen reads for three seconds.
        assert!(!detector.wants_screen(true, start + Duration::from_secs(1)));
        let after_grace = start + Duration::from_secs(4);
        assert!(detector.wants_screen(true, after_grace));

        let working = DetectionInput {
            screen: "• Working (2s · esc to interrupt)\n› ",
            osc_title: "",
            osc_progress: "",
        };
        assert_eq!(
            detector.observe_screen(working, after_grace),
            AgentPublish::Snapshot(AgentSnapshot {
                kind: AgentKind::Codex,
                state: AgentState::Working,
            })
        );
        let idle = DetectionInput {
            screen: "• Done\n› ",
            osc_title: "",
            osc_progress: "",
        };
        // Plain Idle after Working is held for two more confirmations.
        let t1 = after_grace + Duration::from_millis(100);
        assert_eq!(detector.observe_screen(idle, t1), AgentPublish::Nothing);
        assert_eq!(detector.poll_interval(), AGENT_PENDING_IDLE_RECHECK);
        let t2 = t1 + Duration::from_millis(100);
        assert_eq!(detector.observe_screen(idle, t2), AgentPublish::Nothing);
        let t3 = t2 + Duration::from_millis(100);
        assert_eq!(detector.observe_screen(idle, t3), AgentPublish::Nothing);
        // The third confirmation releases the hold.
        let t4 = t3 + Duration::from_millis(100);
        assert_eq!(
            detector.observe_screen(idle, t4),
            AgentPublish::Snapshot(AgentSnapshot {
                kind: AgentKind::Codex,
                state: AgentState::Idle,
            })
        );
        assert_eq!(detector.poll_interval(), AGENT_POLL_INTERVAL);
    }

    #[test]
    fn a_visible_idle_bypasses_the_hold_and_exits_publish_idle_before_clearing() {
        let mut detector = AgentDetector::new();
        let start = Instant::now();
        detector.observe_process(ProcessProbeResult::Agent(AgentKind::Claude), "", "", start);
        let after_grace = start + Duration::from_secs(4);
        assert!(detector.wants_screen(true, after_grace));
        let working = DetectionInput {
            screen: "⏵ Thinking… (esc to interrupt · 3s)\n────\n❯ \n────",
            osc_title: "",
            osc_progress: "",
        };
        assert!(matches!(
            detector.observe_screen(working, after_grace),
            AgentPublish::Snapshot(AgentSnapshot {
                state: AgentState::Working,
                ..
            })
        ));
        let prompt = DetectionInput {
            screen: "  done\n────\n❯ \n────\n  ? for shortcuts",
            osc_title: "",
            osc_progress: "",
        };
        assert!(matches!(
            detector.observe_screen(prompt, after_grace + Duration::from_millis(300)),
            AgentPublish::Snapshot(AgentSnapshot {
                state: AgentState::Idle,
                ..
            })
        ));

        // Back to Working, then the process disappears: Idle first, then the clear.
        detector.observe_screen(working, after_grace + Duration::from_millis(600));
        let gone = after_grace + Duration::from_secs(6);
        assert_eq!(
            detector.observe_process(ProcessProbeResult::ShellOnly, "", "", gone),
            AgentPublish::Snapshot(AgentSnapshot {
                kind: AgentKind::Claude,
                state: AgentState::Idle,
            })
        );
        assert!(detector.wants_process_probe(gone, false, false));
        assert_eq!(
            detector.observe_process(ProcessProbeResult::ShellOnly, "", "", gone),
            AgentPublish::Cleared
        );
        assert_eq!(detector.agent(), None);
    }

    #[test]
    fn a_same_kind_restart_after_a_published_exit_is_a_new_agent() {
        let mut detector = AgentDetector::new();
        let now = Instant::now();
        detector.observe_process(ProcessProbeResult::Agent(AgentKind::Codex), "", "", now);
        assert_eq!(
            detector.observe_process(ProcessProbeResult::ShellOnly, "", "", now),
            AgentPublish::Snapshot(AgentSnapshot {
                kind: AgentKind::Codex,
                state: AgentState::Idle,
            })
        );

        // The Idle went out; a new codex appears before the identity is cleared.
        assert_eq!(
            detector.observe_process(
                ProcessProbeResult::Agent(AgentKind::Codex),
                "old title",
                "",
                now
            ),
            AgentPublish::Snapshot(AgentSnapshot {
                kind: AgentKind::Codex,
                state: AgentState::Unknown,
            })
        );
        assert!(
            !detector.wants_screen(true, now),
            "startup grace must restart"
        );
        assert_eq!(detector.stale_osc_title.as_deref(), Some("old title"));
    }

    #[test]
    fn a_quiet_shell_leaves_the_fast_probe_cadence_until_something_happens() {
        let mut detector = AgentDetector::new();
        let start = Instant::now();
        assert!(detector.wants_process_probe(start, false, false));
        detector.observe_process(ProcessProbeResult::ShellOnly, "", "", start);
        let half_second = Duration::from_millis(500);
        assert!(detector.wants_process_probe(start + half_second, false, false));
        detector.observe_process(ProcessProbeResult::ShellOnly, "", "", start + half_second);

        let quiet = start + PROCESS_ACQUISITION_WINDOW + half_second;
        detector.observe_process(ProcessProbeResult::ShellOnly, "", "", quiet);
        assert!(!detector.wants_process_probe(quiet + half_second, false, false));
        assert!(detector.wants_process_probe(quiet + PROCESS_RECHECK_QUIET, false, false));

        // Output reopens the window; a foreground change probes at once.
        assert!(detector.wants_process_probe(quiet + half_second, false, true));
        detector.observe_process(ProcessProbeResult::ShellOnly, "", "", quiet + half_second);
        assert!(detector.wants_process_probe(quiet + half_second * 2, false, false));
        assert!(detector.wants_process_probe(quiet + half_second * 2, true, false));

        // Continuous output does not keep the window open past its 8 s.
        let mut detector = AgentDetector::new();
        let mut at = start;
        while at < start + PROCESS_ACQUISITION_WINDOW + half_second {
            detector.wants_process_probe(at, false, true);
            detector.observe_process(ProcessProbeResult::Unidentified, "", "", at);
            at += half_second;
        }
        assert!(!detector.wants_process_probe(at, false, true));
    }

    #[test]
    fn unidentified_probes_need_six_misses_but_a_bare_shell_needs_one() {
        let mut detector = AgentDetector::new();
        let now = Instant::now();
        detector.observe_process(ProcessProbeResult::Agent(AgentKind::Codex), "", "", now);
        for _ in 0..5 {
            assert_eq!(
                detector.observe_process(ProcessProbeResult::Unidentified, "", "", now),
                AgentPublish::Nothing
            );
        }
        assert!(matches!(
            detector.observe_process(ProcessProbeResult::Unidentified, "", "", now),
            AgentPublish::Snapshot(_)
        ));
        assert_eq!(
            detector.observe_process(ProcessProbeResult::Unidentified, "", "", now),
            AgentPublish::Cleared
        );
    }

    #[test]
    fn a_replacement_agent_ignores_the_previous_title_until_it_changes() {
        let mut detector = AgentDetector::new();
        let now = Instant::now();
        detector.observe_process(ProcessProbeResult::Agent(AgentKind::Claude), "", "", now);
        detector.observe_process(
            ProcessProbeResult::Agent(AgentKind::Codex),
            "Action Required",
            "",
            now,
        );
        let later = now + Duration::from_secs(4);
        assert!(detector.wants_screen(true, later));
        let stale = detector.observe_screen(
            DetectionInput {
                screen: "› ",
                osc_title: "Action Required",
                osc_progress: "",
            },
            later,
        );
        assert!(matches!(
            stale,
            AgentPublish::Snapshot(AgentSnapshot {
                state: AgentState::Idle,
                ..
            })
        ));
        let fresh = detector.observe_screen(
            DetectionInput {
                screen: "› ",
                osc_title: "⠼ condr",
                osc_progress: "",
            },
            later + Duration::from_millis(300),
        );
        assert!(matches!(
            fresh,
            AgentPublish::Snapshot(AgentSnapshot {
                state: AgentState::Working,
                ..
            })
        ));
    }
}
