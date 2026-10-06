//! Antigravity CLI.

use super::hook::HookInput;
use super::hooks::{HookEntry, HookFormat, HookSpec, HookTarget, entry};
use super::{AgentEventKind, AgentSpec, AgentSupport};

pub(super) const SPEC: AgentSpec = AgentSpec {
    id: "antigravity",
    label: "Antigravity CLI",
    mark: "antigravity.svg",
    brand_color: None,
    executable: "agy",
    aliases: &[],
    packages: &[],
    resume: &["--conversation", "{id}"],
    // Its documented contract has no session start.
    reports_at_startup: false,
    support: AgentSupport::Full(HookSpec {
        format: HookFormat::Plugin { events: HOOKS },
        dir: |target: &HookTarget| target.home.join(".gemini/antigravity-cli"),
        file: "plugins/condr/hooks.json",
        question_tool: Some("ask_question"),
        adjust: Some(adjust),
        prompt_ids: false,
        // Antigravity expects a JSON response even from a passive hook. The empty
        // object changes no permissions.
        reply: Some("{}"),
        enable: None,
        note: Some(
            "Enable the condr plugin in Antigravity CLI; hooks expose no permission-wait event, and startup stays Unknown until the first invocation",
        ),
    }),
};

const HOOKS: &[HookEntry] = &[
    entry("PreInvocation", AgentEventKind::PromptSubmit),
    HookEntry {
        matcher: Some("*"),
        ..entry("PreToolUse", AgentEventKind::ToolStart)
    },
    HookEntry {
        matcher: Some("*"),
        ..entry("PostToolUse", AgentEventKind::ToolComplete)
    },
    entry("Stop", AgentEventKind::Stop),
];

/// A Stop ends the turn only once the agent is fully idle.
fn adjust(input: &HookInput, event: AgentEventKind) -> Option<AgentEventKind> {
    (event != AgentEventKind::Stop || input.fully_idle == Some(true)).then_some(event)
}
