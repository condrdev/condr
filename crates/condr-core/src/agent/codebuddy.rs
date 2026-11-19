//! Tencent CodeBuddy Code.

use super::hooks::{HookEntry, HookFormat, HookShell, HookSpec, HookTarget, entry};
use super::{AgentEventKind, AgentSpec, AgentSupport, InitialPrompt};

pub(super) const SPEC: AgentSpec = AgentSpec {
    id: "codebuddy",
    label: "CodeBuddy Code",
    mark: "codebuddy.svg",
    brand_color: None,
    executable: "codebuddy",
    aliases: &["cbc", "codebuddy-code"],
    packages: &["@tencent-ai/codebuddy-code"],
    resume: &["--resume", "{id}"],
    reports_at_startup: true,
    reads_kitty_keys: false,
    initial_prompt: InitialPrompt::Positional,
    support: AgentSupport::Full(HookSpec {
        format: HookFormat::NestedMap {
            events: HOOKS,
            shell: HookShell::Plain,
        },
        dir: |target: &HookTarget| target.env_dir("CODEBUDDY_CONFIG_DIR", ".codebuddy"),
        file: "settings.json",
        question_tool: None,
        adjust: None,
        prompt_ids: false,
        reply: None,
        enable: None,
        note: None,
    }),
};

/// Claude Code's events: `Stop` is the main agent's alone, a subagent ends with
/// `SubagentStop`, and `Notification` is narrowed to the kinds that wait on the user.
const HOOKS: &[HookEntry] = &[
    entry("SessionStart", AgentEventKind::SessionStart),
    entry("UserPromptSubmit", AgentEventKind::PromptSubmit),
    entry("PermissionRequest", AgentEventKind::PermissionRequest),
    HookEntry {
        matcher: Some("permission_prompt|elicitation_dialog"),
        ..entry("Notification", AgentEventKind::PermissionRequest)
    },
    entry("PreToolUse", AgentEventKind::ToolStart),
    entry("PostToolUse", AgentEventKind::ToolComplete),
    entry("PostToolUseFailure", AgentEventKind::ToolComplete),
    entry("Stop", AgentEventKind::Stop),
    entry("StopFailure", AgentEventKind::StopFailure),
];
