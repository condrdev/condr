//! Claude Code.

use super::hooks::{HookEntry, HookFormat, HookShell, HookSpec, HookTarget, entry};
use super::{AgentEventKind, AgentSpec, AgentSupport};

pub(super) const SPEC: AgentSpec = AgentSpec {
    id: "claude",
    label: "Claude",
    mark: "claude.svg",
    // Anthropic's terracotta, as the published mark carries it.
    brand_color: Some(0xD97757),
    executable: "claude",
    aliases: &["claude-code"],
    packages: &["@anthropic-ai/claude-code"],
    resume: &["--resume", "{id}"],
    reports_at_startup: true,
    reads_kitty_keys: true,
    support: AgentSupport::Full(HookSpec {
        format: HookFormat::NestedMap {
            events: HOOKS,
            // Git Bash runs the hooks on Windows.
            shell: HookShell::Plain,
        },
        dir: |target: &HookTarget| target.env_dir("CLAUDE_CONFIG_DIR", ".claude"),
        file: "settings.json",
        question_tool: Some("AskUserQuestion"),
        adjust: None,
        prompt_ids: false,
        reply: None,
        enable: None,
        note: None,
    }),
};

/// `Notification` is narrowed to the kinds that wait on the user; a user interrupt fires
/// nothing, which ADR 0014 accepts.
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
    entry("Stop", AgentEventKind::Stop),
    entry("StopFailure", AgentEventKind::StopFailure),
];
