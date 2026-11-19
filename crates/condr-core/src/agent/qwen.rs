//! Qwen Code.

use super::hook::HookInput;
use super::hooks::{HookEntry, HookFormat, HookShell, HookSpec, HookTarget, entry};
use super::{AgentEventKind, AgentSpec, AgentSupport, InitialPrompt};

pub(super) const SPEC: AgentSpec = AgentSpec {
    id: "qwen",
    label: "Qwen Code",
    mark: "qwen.svg",
    brand_color: None,
    executable: "qwen",
    // The `app` its OSC 7501 reports carry.
    aliases: &["qwen-code"],
    packages: &["@qwen-code/qwen-code"],
    resume: &["--resume", "{id}"],
    reports_at_startup: true,
    reads_kitty_keys: false,
    initial_prompt: InitialPrompt::Flag("-i"),
    support: AgentSupport::Full(HookSpec {
        format: HookFormat::NestedMap {
            events: HOOKS,
            shell: HookShell::Plain,
        },
        dir: |target: &HookTarget| target.env_dir("QWEN_HOME", ".qwen"),
        file: "settings.json",
        question_tool: None,
        adjust: Some(adjust),
        prompt_ids: false,
        reply: None,
        enable: None,
        note: None,
    }),
};

/// `Stop` is the main agent's alone; a subagent ends with `SubagentStop`.
const HOOKS: &[HookEntry] = &[
    entry("SessionStart", AgentEventKind::SessionStart),
    entry("UserPromptSubmit", AgentEventKind::PromptSubmit),
    entry("PermissionRequest", AgentEventKind::PermissionRequest),
    HookEntry {
        matcher: Some("permission_prompt"),
        ..entry("Notification", AgentEventKind::PermissionRequest)
    },
    entry("PreToolUse", AgentEventKind::ToolStart),
    entry("PostToolUse", AgentEventKind::ToolComplete),
    entry("PostToolUseFailure", AgentEventKind::ToolComplete),
    entry("Stop", AgentEventKind::Stop),
    entry("StopFailure", AgentEventKind::StopFailure),
];

/// An event that fires inside a subagent carries its `agent_id`; it is not the Pane's.
fn adjust(input: &HookInput, event: AgentEventKind) -> Option<AgentEventKind> {
    (!input.in_subagent).then_some(event)
}
