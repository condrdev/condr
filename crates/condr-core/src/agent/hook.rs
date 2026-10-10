//! The `condr agent-hook` side of ADR 0014: turn one hook invocation into an OSC 777
//! event on the controlling terminal, where the Server's PTY reader picks it up.
//!
//! Everything here fails quietly. A hook that blocks or errors would stall the agent it
//! observes, so the worst outcome is a missed status update.

use std::io::{IsTerminal as _, Read as _};

use super::{AgentEventKind, AgentKind};

#[cfg(feature = "runtime")]
mod report;

#[cfg(feature = "runtime")]
pub use report::run;

/// How much of the agent's JSON is parsed; the rest is drained so the agent's write
/// never fails with EPIPE, but a pasted file in a tool result is not worth reading.
const MAX_PARSED_STDIN: usize = 64 * 1024;
/// The `source` values Claude Code and Codex document; anything else is not a session
/// start reason Condr knows, and an unbounded string would not fit the OSC anyway.
const SESSION_SOURCES: [&str; 8] = [
    "startup", "resume", "clear", "compact", "fork", "new", "reload", "load",
];

/// What one hook invocation's JSON says, as far as Condr reads it.
#[derive(Default)]
pub(super) struct HookInput {
    source: Option<String>,
    session_id: Option<String>,
    tool_name: Option<String>,
    subagent: bool,
    pub(super) initial_prompt: bool,
    prompt_id: Option<String>,
    pub(super) fully_idle: Option<bool>,
    /// A ready-made detail from Condr's own OpenCode/Pi plugins.
    detail: Option<String>,
    /// The question a question tool asks.
    question: Option<String>,
    /// The command a shell tool wants to run.
    command: Option<String>,
    /// A notification's text, when that is all the native event carries.
    message: Option<String>,
}

impl HookInput {
    /// What the user is being asked for (ADR 0024): the question for a question, else
    /// the tool and its command for a permission, else the notification text. Only a
    /// blocking event carries one.
    fn detail(&self, event: AgentEventKind) -> Option<String> {
        let raw = match event {
            AgentEventKind::QuestionAsked => self
                .detail
                .clone()
                .or_else(|| self.question.clone())
                .or_else(|| self.message.clone()),
            AgentEventKind::PermissionRequest => {
                self.detail
                    .clone()
                    .or_else(|| match (&self.tool_name, &self.command) {
                        (Some(tool), Some(command)) => Some(format!("{tool}: {command}")),
                        (Some(tool), None) => Some(tool.clone()),
                        (None, Some(command)) => Some(command.clone()),
                        (None, None) => self.message.clone(),
                    })
            }
            _ => None,
        }?;
        super::event::bound_detail(&raw)
    }

    /// The event `agent`'s hooks report this invocation as, or none: a subagent's events
    /// are not the Pane's.
    fn event(&self, agent: AgentKind, event: AgentEventKind) -> Option<AgentEventKind> {
        if self.subagent {
            return None;
        }
        let Some(hooks) = agent.spec().hooks() else {
            return Some(event);
        };
        let event = match hooks.adjust {
            Some(adjust) => adjust(self, event)?,
            None => event,
        };
        Some(
            if event == AgentEventKind::ToolStart
                && hooks.question_tool.is_some()
                && self.tool_name.as_deref() == hooks.question_tool
            {
                AgentEventKind::QuestionAsked
            } else {
                event
            },
        )
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
            detail: field("detail"),
            question: [&value["tool_input"], &value["toolCall"]["args"], &value]
                .into_iter()
                .find_map(|input| {
                    // Claude's AskUserQuestion takes `questions: [{question}]`; other
                    // question tools take one `question`.
                    input["questions"][0]["question"]
                        .as_str()
                        .or_else(|| input["question"].as_str())
                        .map(str::to_owned)
                }),
            command: [&value["tool_input"], &value["toolCall"]["args"]]
                .into_iter()
                .find_map(|input| match &input["command"] {
                    serde_json::Value::String(command) => Some(command.clone()),
                    // Codex hands a shell command over as argv.
                    serde_json::Value::Array(argv) => Some(
                        argv.iter()
                            .filter_map(serde_json::Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" "),
                    ),
                    _ => None,
                })
                .filter(|command| !command.is_empty())
                // The first line says what runs; a heredoc body is not the point.
                .map(|command| command.lines().next().unwrap_or_default().to_owned()),
            message: field("message").or_else(|| field("title")),
        }
    }
}

#[cfg(all(test, feature = "runtime"))]
mod tests {
    #[cfg(unix)]
    use super::report::Ancestors;
    use super::*;
    use crate::agent::AgentEvent;

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
        assert_eq!(
            input.detail(AgentEventKind::PermissionRequest).as_deref(),
            Some("Bash")
        );
        assert_eq!(input.detail(AgentEventKind::ToolStart), None);
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

    #[test]
    fn blocking_hooks_say_what_they_wait_for() {
        use AgentEventKind::*;
        let detail = |json: &str, event| HookInput::parse(json.as_bytes()).detail(event);
        // Claude Code PermissionRequest: the tool and the command's first line.
        assert_eq!(
            detail(
                r#"{"tool_name":"Bash","tool_input":{"command":"cargo test\n# and more"}}"#,
                PermissionRequest
            )
            .as_deref(),
            Some("Bash: cargo test")
        );
        // Codex hands argv over.
        assert_eq!(
            detail(
                r#"{"tool_name":"shell","tool_input":{"command":["git","status"]}}"#,
                PermissionRequest
            )
            .as_deref(),
            Some("shell: git status")
        );
        // A notification only has its message.
        assert_eq!(
            detail(
                r#"{"notification_type":"permission_prompt","message":"Claude needs your permission to use Bash"}"#,
                PermissionRequest
            )
            .as_deref(),
            Some("Claude needs your permission to use Bash")
        );
        // AskUserQuestion and Antigravity's ask_question.
        assert_eq!(
            detail(
                r#"{"tool_name":"AskUserQuestion","tool_input":{"questions":[{"question":"Which  DB?\n"}]}}"#,
                QuestionAsked
            )
            .as_deref(),
            Some("Which DB?")
        );
        assert_eq!(
            detail(
                r#"{"toolCall":{"name":"ask_question","args":{"question":"Deploy?"}}}"#,
                QuestionAsked
            )
            .as_deref(),
            Some("Deploy?")
        );
        // Condr's own plugins pass a finished detail; it still gets bounded.
        let long = format!(r#"{{"detail":"{}"}}"#, "x".repeat(400));
        let bounded = detail(&long, QuestionAsked).unwrap();
        assert_eq!(
            bounded.chars().count(),
            super::super::event::MAX_DETAIL_CHARS
        );
        assert!(bounded.ends_with('…'));
        // Nothing usable stays absent; non-blocking events never carry one.
        assert_eq!(detail(r#"{"session_id":"x"}"#, PermissionRequest), None);
        assert_eq!(detail(r#"{"tool_name":"Bash"}"#, ToolStart), None);
        // And the OSC carries it through.
        let mut event = AgentEvent::new(AgentKind::Claude, PermissionRequest, None, None);
        event.detail = Some("Bash: rm -rf target".into());
        let payload = event.encode();
        let decoded = AgentEvent::decode(&payload[2..payload.len() - 1]).unwrap();
        assert_eq!(decoded.detail.as_deref(), Some("Bash: rm -rf target"));
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
