//! Grok Build.

use super::hooks::{HookEntry, HookFormat, HookShell, HookSpec, HookTarget, entry};
use super::{AgentEventKind, AgentSpec, AgentSupport};

pub(super) const SPEC: AgentSpec = AgentSpec {
    id: "grok",
    label: "Grok Build",
    mark: "grok.svg",
    brand_color: None,
    executable: "grok",
    aliases: &["grok-build"],
    packages: &[],
    resume: &["--resume", "{id}"],
    reports_at_startup: true,
    support: AgentSupport::Full(HookSpec {
        format: HookFormat::NestedMap {
            events: HOOKS,
            // PowerShell by default on Windows, but users may select Git Bash or cmd.
            shell: HookShell::PowerShellUnless("GROK_SHELL"),
        },
        dir: |target: &HookTarget| target.env_dir("GROK_HOME", ".grok"),
        file: "hooks/condr.json",
        question_tool: None,
        adjust: None,
        prompt_ids: true,
        reply: None,
        enable: None,
        note: Some(
            "Grok Stop hooks can request continuation; completion may appear early when other Stop hooks block. Native idle notifications repair missed completion reports",
        ),
    }),
};

const HOOKS: &[HookEntry] = &[
    entry("SessionStart", AgentEventKind::SessionStart),
    entry("UserPromptSubmit", AgentEventKind::PromptSubmit),
    entry("PreToolUse", AgentEventKind::ToolStart),
    entry("PostToolUse", AgentEventKind::ToolComplete),
    entry("PostToolUseFailure", AgentEventKind::ToolComplete),
    HookEntry {
        matcher: Some("permission_prompt"),
        ..entry("Notification", AgentEventKind::PermissionRequest)
    },
    HookEntry {
        matcher: Some("idle_prompt"),
        ..entry("Notification", AgentEventKind::Stop)
    },
    entry("Stop", AgentEventKind::Stop),
    entry("StopFailure", AgentEventKind::StopFailure),
    entry("StopCancelled", AgentEventKind::Interrupt),
];
