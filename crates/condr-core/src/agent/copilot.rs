//! GitHub Copilot CLI.

use super::hook::HookInput;
use super::hooks::{HookEntry, HookFormat, HookSpec, HookTarget, entry};
use super::{AgentEventKind, AgentSpec, AgentSupport};

pub(super) const SPEC: AgentSpec = AgentSpec {
    id: "copilot",
    label: "GitHub Copilot",
    mark: "copilot.svg",
    brand_color: None,
    executable: "copilot",
    aliases: &["github-copilot"],
    packages: &["@github/copilot"],
    resume: &["--resume={id}"],
    // sessionStart waits for the first prompt.
    reports_at_startup: false,
    support: AgentSupport::Full(HookSpec {
        format: HookFormat::FlatList {
            events: HOOKS,
            exec: true,
        },
        dir: |target: &HookTarget| target.env_dir("COPILOT_HOME", ".copilot"),
        file: "hooks/condr.json",
        question_tool: None,
        adjust: Some(adjust),
        prompt_ids: false,
        reply: None,
        enable: None,
        note: Some(
            "Copilot can omit its completion hook after an API error; status may stay Working until the next completion or process exit",
        ),
    }),
};

const HOOKS: &[HookEntry] = &[
    entry("sessionStart", AgentEventKind::SessionStart),
    entry("userPromptSubmitted", AgentEventKind::PromptSubmit),
    entry("preToolUse", AgentEventKind::ToolStart),
    entry("postToolUse", AgentEventKind::ToolComplete),
    entry("postToolUseFailure", AgentEventKind::ToolComplete),
    HookEntry {
        matcher: Some("permission_prompt|elicitation_dialog"),
        ..entry("notification", AgentEventKind::PermissionRequest)
    },
    entry("agentStop", AgentEventKind::Stop),
];

/// Copilot may report sessionStart after its first prompt was submitted.
fn adjust(input: &HookInput, event: AgentEventKind) -> Option<AgentEventKind> {
    Some(
        if event == AgentEventKind::SessionStart && input.initial_prompt {
            AgentEventKind::PromptSubmit
        } else {
            event
        },
    )
}
