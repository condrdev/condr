//! Cursor CLI.

use super::hooks::{HookEntry, HookFormat, HookSpec, HookTarget, entry};
use super::{AgentEventKind, AgentSpec, AgentSupport};

pub(super) const SPEC: AgentSpec = AgentSpec {
    id: "cursor",
    label: "Cursor CLI",
    mark: "cursor.svg",
    brand_color: None,
    executable: "cursor-agent",
    aliases: &[],
    packages: &[],
    resume: &["--resume", "{id}"],
    // A resumed session reports no start.
    reports_at_startup: false,
    reads_kitty_keys: false,
    support: AgentSupport::Full(HookSpec {
        format: HookFormat::FlatList {
            events: HOOKS,
            exec: false,
        },
        dir: |target: &HookTarget| {
            target.var("CURSOR_CONFIG_DIR").unwrap_or_else(|| {
                target
                    .var("XDG_CONFIG_HOME")
                    .map_or_else(|| target.home.join(".cursor"), |dir| dir.join("cursor"))
            })
        },
        file: "hooks.json",
        question_tool: None,
        adjust: None,
        prompt_ids: false,
        reply: None,
        enable: None,
        note: Some(
            "Cursor hooks report activity and completion, but expose no event for a permission prompt",
        ),
    }),
};

const HOOKS: &[HookEntry] = &[
    entry("sessionStart", AgentEventKind::SessionStart),
    entry("beforeSubmitPrompt", AgentEventKind::PromptSubmit),
    entry("preToolUse", AgentEventKind::ToolStart),
    entry("postToolUse", AgentEventKind::ToolComplete),
    entry("postToolUseFailure", AgentEventKind::ToolComplete),
    entry("stop", AgentEventKind::Stop),
];
