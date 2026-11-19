//! Mistral Vibe.

use super::hooks::{HookEntry, HookFormat, HookSpec, HookTarget, TomlKeys, entry};
use super::{AgentEventKind, AgentSpec, AgentSupport, InitialPrompt};

pub(super) const SPEC: AgentSpec = AgentSpec {
    id: "vibe",
    label: "Mistral Vibe",
    mark: "vibe.svg",
    brand_color: None,
    executable: "vibe",
    // `vibe` is a launcher that execs its Rust TUI.
    aliases: &["vibe-rs", "mistral-vibe"],
    packages: &[],
    resume: &["--resume", "{id}"],
    reports_at_startup: true,
    reads_kitty_keys: false,
    initial_prompt: InitialPrompt::Positional,
    support: AgentSupport::Full(HookSpec {
        format: HookFormat::TomlArray {
            events: HOOKS,
            keys: TomlKeys {
                event: "type",
                matcher: "match",
                named: true,
            },
        },
        dir: |target: &HookTarget| target.env_dir("VIBE_HOME", ".vibe"),
        file: "hooks.toml",
        question_tool: None,
        adjust: None,
        prompt_ids: false,
        reply: None,
        enable: None,
        note: Some(
            "Vibe fires no hook on an interrupt; where it cannot report over OSC 7501, as on Windows, an interrupted turn stays Working until the next one",
        ),
    }),
};

/// `match = "main"` keeps a subagent's turns out; `session_start` never fires for one.
const HOOKS: &[HookEntry] = &[
    entry("session_start", AgentEventKind::SessionStart),
    HookEntry {
        matcher: Some("main"),
        ..entry("pre_agent", AgentEventKind::PromptSubmit)
    },
    HookEntry {
        matcher: Some("main"),
        ..entry("post_agent", AgentEventKind::Stop)
    },
    HookEntry {
        matcher: Some("approval_requested"),
        ..entry("notification", AgentEventKind::PermissionRequest)
    },
    HookEntry {
        matcher: Some("question_asked"),
        ..entry("notification", AgentEventKind::QuestionAsked)
    },
    HookEntry {
        matcher: Some("turn_failed"),
        ..entry("notification", AgentEventKind::StopFailure)
    },
    entry("post_tool", AgentEventKind::ToolComplete),
];
