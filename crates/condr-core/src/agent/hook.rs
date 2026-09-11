//! The `condr agent-hook` side of ADR 0014: turn one hook invocation into an OSC 777
//! event on the controlling terminal, where the Server's PTY reader picks it up.
//!
//! Everything here fails quietly. A hook that blocks or errors would stall the agent it
//! observes, so the worst outcome is a missed status update.

use std::fs::File;
use std::io::{IsTerminal as _, Read as _, Write as _};
use std::time::Duration;

use super::{AgentEvent, AgentEventKind, AgentKind, ProcessInfo, identify_agent_process};

/// How much of the agent's JSON is parsed; the rest is drained so the agent's write
/// never fails with EPIPE, but a pasted file in a tool result is not worth reading.
const MAX_PARSED_STDIN: usize = 64 * 1024;
/// Finding a terminal to write to must not take longer than this. The write itself is
/// never abandoned half way: a torn OSC would swallow the output that follows it.
const OPEN_DEADLINE: Duration = Duration::from_secs(2);
/// The `source` values Claude Code and Codex document; anything else is not a session
/// start reason Condr knows, and an unbounded string would not fit the OSC anyway.
const SESSION_SOURCES: [&str; 8] = [
    "startup", "resume", "clear", "compact", "fork", "new", "reload", "load",
];

/// Runs the hook. `agent` and `event` are the slugs the installer wrote into the agent's
/// configuration; stdin carries the agent's JSON. Returns whether the event reached a
/// terminal; callers exit 0 regardless.
pub fn run(agent: &str, event: &str) -> bool {
    let (Some(agent), Some(event)) = (AgentKind::parse_label(agent), AgentEventKind::parse(event))
    else {
        return false;
    };
    if agent == AgentKind::Antigravity {
        // Antigravity expects a JSON response even from a passive hook. The empty
        // object changes no permissions; the status still travels only over OSC.
        let _ = writeln!(std::io::stdout().lock(), "{{}}");
    }
    if std::env::var_os(crate::PaneEnvironment::ENV).is_none_or(|value| value != "1") {
        return false;
    }
    let input = HookInput::read();
    let Some(event) = input.event(agent, event) else {
        return false;
    };
    let source = (event == AgentEventKind::SessionStart)
        .then_some(input.source)
        .flatten();
    // An agent an agent runs (Claude Code calling `claude -p` from its Bash tool) has
    // the same hooks and the same terminal; its turns are not this Pane's.
    let ancestors = Ancestors::of(agent);
    if ancestors.nested {
        return false;
    }
    let pane_process = ancestors.pane_process;
    let (opened, wait) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = opened.send(open_controlling_terminal(pane_process));
    });
    let Ok(Some(mut terminal)) = wait.recv_timeout(OPEN_DEADLINE) else {
        return false;
    };
    let mut report = AgentEvent::new(agent, event, source, input.session_id);
    report.prompt_id = input.prompt_id;
    let bytes = report.encode();
    terminal
        .write_all(&bytes)
        .and_then(|()| terminal.flush())
        .is_ok()
}

#[derive(Default)]
struct HookInput {
    source: Option<String>,
    session_id: Option<String>,
    tool_name: Option<String>,
    subagent: bool,
    initial_prompt: bool,
    prompt_id: Option<String>,
    fully_idle: Option<bool>,
}

impl HookInput {
    fn event(&self, agent: AgentKind, event: AgentEventKind) -> Option<AgentEventKind> {
        if self.subagent
            || (agent == AgentKind::Antigravity
                && event == AgentEventKind::Stop
                && self.fully_idle != Some(true))
        {
            return None;
        }
        Some(match event {
            // Copilot may report sessionStart after its first prompt was submitted.
            AgentEventKind::SessionStart if agent == AgentKind::Copilot && self.initial_prompt => {
                AgentEventKind::PromptSubmit
            }
            AgentEventKind::ToolStart
                if matches!(
                    (agent, self.tool_name.as_deref()),
                    (AgentKind::Claude, Some("AskUserQuestion"))
                        | (AgentKind::Antigravity, Some("ask_question"))
                ) =>
            {
                AgentEventKind::QuestionAsked
            }
            other => other,
        })
    }

    /// Reads stdin to the end, parsing only its head.
    fn read() -> Self {
        let mut stdin = std::io::stdin();
        if stdin.is_terminal() {
            return Self::default();
        }
        let mut head = Vec::new();
        if (&mut stdin)
            .take(MAX_PARSED_STDIN as u64)
            .read_to_end(&mut head)
            .is_err()
        {
            return Self::default();
        }
        let _ = std::io::copy(&mut stdin, &mut std::io::sink());
        Self::parse(&head)
    }

    fn parse(json: &[u8]) -> Self {
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(json) else {
            return Self::default();
        };
        let field = |name: &str| value.get(name)?.as_str().map(str::to_owned);
        Self {
            source: field("source").filter(|source| SESSION_SOURCES.contains(&source.as_str())),
            tool_name: field("tool_name")
                .or_else(|| field("toolName"))
                .or_else(|| {
                    value
                        .get("toolCall")?
                        .get("name")?
                        .as_str()
                        .map(str::to_owned)
                }),
            session_id: field("session_id")
                .or_else(|| field("sessionId"))
                .or_else(|| field("conversation_id"))
                .or_else(|| field("conversationId"))
                .filter(|id| super::valid_session_id(id)),
            subagent: field("subagentType").is_some_and(|kind| !kind.is_empty()),
            initial_prompt: field("initialPrompt").is_some_and(|prompt| !prompt.is_empty()),
            prompt_id: field("promptId").filter(|id| super::event::valid_prompt_id(id)),
            fully_idle: value.get("fullyIdle").and_then(serde_json::Value::as_bool),
        }
    }
}

/// The process tree above the hook: whether a second instance of the hook's agent sits
/// above the first, in which case the hook belongs to an agent the agent itself launched
/// and its events are not the Pane's; and the Pane's own process, the child of the Server.
struct Ancestors {
    nested: bool,
    pane_process: Option<u32>,
}

impl Ancestors {
    /// Consecutive agent processes are one instance: `codex` is a Node wrapper spawning
    /// the native binary, often behind a version manager's shim. An agent runs tools
    /// through a shell, so a shell between two agent processes starts another instance:
    /// `claude` running `sh -c "claude -p …"` is two.
    fn of(agent: AgentKind) -> Self {
        use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
        let mut system = System::new();
        let mut pid = Pid::from_u32(std::process::id());
        let mut found = Self {
            nested: false,
            pane_process: None,
        };
        let mut agent_seen = false;
        let mut separated = true;
        let mut child = None;
        for _ in 0..24 {
            system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                ProcessRefreshKind::new().with_cmd(UpdateKind::Always),
            );
            let Some(process) = system.process(pid) else {
                break;
            };
            let name = process.name().to_string_lossy();
            let argv: Vec<String> = process
                .cmd()
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect();
            let info = ProcessInfo {
                name: &name,
                argv: (!argv.is_empty()).then_some(argv.as_slice()),
            };
            let front = argv.first().map(String::as_str).unwrap_or(&name);
            if super::is_shell(front) {
                separated = true;
            } else if let Some(actual) = identify_agent_process(info) {
                // Some CLIs import Claude/Cursor hook files. A hook registered for
                // another CLI must not claim this Pane before process discovery.
                if actual != agent || (separated && agent_seen) {
                    found.nested = true;
                    break;
                }
                separated = false;
                agent_seen = true;
            } else if super::normalized_lookup_name(super::path_basename(front)) == "condr"
                && argv.get(1).is_some_and(|argument| argument == "server")
            {
                found.pane_process = child;
                break;
            }
            child = Some(pid.as_u32());
            let Some(parent) = process.parent() else {
                break;
            };
            pid = parent;
        }
        found
    }
}

#[cfg(unix)]
fn open_controlling_terminal(_pane_process: Option<u32>) -> Option<File> {
    if let Ok(tty) = File::options().write(true).open("/dev/tty") {
        return Some(tty);
    }
    // Without a controlling terminal of our own, the nearest ancestor that has one is
    // the agent, or the shell it runs in.
    let mut pid = std::os::unix::process::parent_id();
    for _ in 0..8 {
        if pid <= 1 {
            break;
        }
        let output = std::process::Command::new("ps")
            .args(["-o", "tty=", "-o", "ppid=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        let line = String::from_utf8_lossy(&output.stdout);
        let mut fields = line.split_whitespace();
        let tty = fields.next().unwrap_or("");
        let parent = fields
            .next()
            .and_then(|ppid| ppid.parse().ok())
            .unwrap_or(1);
        if !tty.is_empty() && tty != "?" && tty != "??" {
            return File::options().write(true).open(format!("/dev/{tty}")).ok();
        }
        pid = parent;
    }
    None
}

#[cfg(windows)]
#[allow(unsafe_code)] // Win32 console attach; the only way to reach an ancestor's ConPTY.
fn open_controlling_terminal(pane_process: Option<u32>) -> Option<File> {
    use windows_sys::Win32::System::Console::{AttachConsole, FreeConsole};

    // The Pane's process sits on the ConPTY. The inherited console need not be: Claude
    // Code spawns its shell with CREATE_NO_WINDOW, so the hook starts on a private,
    // invisible conhost whose output goes nowhere.
    if let Some(pid) = pane_process {
        // SAFETY: plain Win32 console calls with no pointers involved.
        let attached = unsafe {
            FreeConsole();
            AttachConsole(pid) != 0
        };
        if attached && let Some(out) = open_console_out() {
            return Some(out);
        }
    }
    if let Some(out) = open_console_out() {
        return Some(out);
    }
    // Otherwise borrow an ancestor's: the launcher, or the shell it runs in.
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        sysinfo::ProcessRefreshKind::new(),
    );
    let mut pid = sysinfo::Pid::from_u32(std::process::id());
    for _ in 0..16 {
        let parent = system.process(pid).and_then(sysinfo::Process::parent)?;
        // SAFETY: plain Win32 console calls with no pointers involved.
        let attached = unsafe {
            FreeConsole();
            AttachConsole(parent.as_u32()) != 0
        };
        if attached && let Some(out) = open_console_out() {
            return Some(out);
        }
        pid = parent;
    }
    None
}

/// `CONOUT$` with virtual terminal processing on, so conhost passes the OSC through to
/// the ConPTY instead of printing it. The agent usually enables the mode on its own
/// handle; this one is the hook's, and must not depend on that.
#[cfg(windows)]
#[allow(unsafe_code)] // GetConsoleMode/SetConsoleMode on the handle we just opened.
fn open_console_out() -> Option<File> {
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::System::Console::{
        ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, SetConsoleMode,
    };

    let out = File::options()
        .read(true)
        .write(true)
        .open("CONOUT$")
        .ok()?;
    let handle = out.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
    let mut mode = 0;
    // SAFETY: `handle` is a live console handle owned by `out`; `mode` outlives the call.
    unsafe {
        if GetConsoleMode(handle, &mut mode) != 0
            && SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) == 0
        {
            return None;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_hook_shapes_keep_identity_and_only_report_real_boundaries() {
        use AgentEventKind::*;
        let copilot =
            HookInput::parse(br#"{"sessionId":"root","initialPrompt":"hello","toolName":"shell"}"#);
        assert_eq!(copilot.session_id.as_deref(), Some("root"));
        assert_eq!(
            copilot.event(AgentKind::Copilot, SessionStart),
            Some(PromptSubmit)
        );
        let cursor = HookInput::parse(br#"{"conversation_id":"cursor-root"}"#);
        assert_eq!(cursor.session_id.as_deref(), Some("cursor-root"));
        let grok = HookInput::parse(
            br#"{"sessionId":"root","promptId":"turn-1","subagentType":"explore"}"#,
        );
        assert_eq!(grok.prompt_id.as_deref(), Some("turn-1"));
        assert_eq!(grok.event(AgentKind::Grok, Stop), None);
        let agy = HookInput::parse(br#"{"conversationId":"agy-root","toolCall":{"name":"ask_question"},"fullyIdle":false}"#);
        assert_eq!(agy.session_id.as_deref(), Some("agy-root"));
        assert_eq!(
            agy.event(AgentKind::Antigravity, ToolStart),
            Some(QuestionAsked)
        );
        assert_eq!(agy.event(AgentKind::Antigravity, Stop), None);
        assert_eq!(
            HookInput::default().event(AgentKind::Antigravity, Stop),
            None
        );
        assert_eq!(
            HookInput::parse(br#"{"fullyIdle":true}"#).event(AgentKind::Antigravity, Stop),
            Some(Stop)
        );
    }

    #[test]
    fn hook_input_keeps_the_native_session_id_and_bounded_status_fields() {
        let input = HookInput::parse(
            br#"{"session_id":"abc","source":"compact","cwd":"/x","tool_name":"Bash"}"#,
        );
        assert_eq!(input.session_id.as_deref(), Some("abc"));
        assert_eq!(input.source.as_deref(), Some("compact"));
        assert_eq!(input.tool_name.as_deref(), Some("Bash"));
        let long = format!(r#"{{"source":"{}"}}"#, "x".repeat(5000));
        assert_eq!(HookInput::parse(long.as_bytes()).source, None);
        assert_eq!(HookInput::parse(br#"{"session_id":"abc"}"#).source, None);
        assert_eq!(HookInput::parse(b"not json").tool_name, None);
        for id in [
            "",
            "--last",
            "a\nb",
            "../session",
            "a;b",
            "a b",
            &"x".repeat(257),
        ] {
            let json = serde_json::json!({"session_id": id}).to_string();
            assert_eq!(HookInput::parse(json.as_bytes()).session_id, None, "{id:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "invoked by the process-boundary regression below"]
    fn ancestor_fixture() {
        let kind = AgentKind::parse_label(&std::env::var("CONDR_HOOK_TEST_KIND").unwrap()).unwrap();
        let ancestors = Ancestors::of(kind);
        assert_eq!(ancestors.nested, kind != AgentKind::Grok);
        if kind == AgentKind::Grok {
            assert!(
                ancestors.pane_process.is_some(),
                "stop at the owning Server"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_imported_hook_cannot_claim_another_cli_and_the_server_bounds_the_walk() {
        use std::os::unix::process::CommandExt as _;
        let root =
            std::env::temp_dir().join(format!("condr-hook-ancestors-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("server"), r#"
bash -c 'exec -a grok "$CONDR_HOOK_TEST_EXE" --exact agent::hook::tests::ancestor_fixture --ignored --nocapture'
status=$?
exit "$status"
"#).unwrap();
        for kind in ["grok", "claude"] {
            let output = std::process::Command::new("/bin/bash")
                .arg0("condr")
                .arg("server")
                .current_dir(&root)
                .env("CONDR_HOOK_TEST_EXE", std::env::current_exe().unwrap())
                .env("CONDR_HOOK_TEST_KIND", kind)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
