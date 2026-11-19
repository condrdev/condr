//! Kimi Code, fully supported through its own reports (ADR 0071).

use super::hooks::{HookEntry, HookFormat, HookSpec, HookTarget, TomlKeys, entry};
use super::{AgentEventKind, AgentSpec, AgentSupport, InitialPrompt};

pub(super) const SPEC: AgentSpec = AgentSpec {
    id: "kimi",
    label: "Kimi Code",
    mark: "kimi.svg",
    brand_color: None,
    executable: "kimi",
    aliases: &["kimi-code"],
    packages: &[],
    resume: &["--session", "{id}"],
    // It reports over OSC 7501 once its prompt shows, before any session exists.
    reports_at_startup: true,
    reads_kitty_keys: false,
    // `-p` runs one prompt without the TUI.
    initial_prompt: InitialPrompt::None,
    // A subagent's Stop arrives under the main session's ID, so the hooks only name the
    // conversation; the state is what Kimi Code reports itself.
    support: AgentSupport::ProgramStatus(HookSpec {
        format: HookFormat::TomlArray {
            events: HOOKS,
            keys: TomlKeys {
                event: "event",
                matcher: "matcher",
                named: false,
            },
        },
        dir: |target: &HookTarget| target.env_dir("KIMI_CODE_HOME", ".kimi-code"),
        file: "config.toml",
        question_tool: None,
        adjust: None,
        prompt_ids: false,
        reply: None,
        enable: None,
        note: Some(
            "Requires Kimi Code 2.4 or newer, which reports its own state; its hooks only name the conversation, and a subagent's Stop shares the main session's ID",
        ),
    }),
};

/// A session starts with its first message or a resume; a subagent starts none.
const HOOKS: &[HookEntry] = &[entry("SessionStart", AgentEventKind::SessionStart)];
