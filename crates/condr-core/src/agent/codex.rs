//! Codex.

use super::hooks::{HookEntry, HookFormat, HookShell, HookSpec, HookTarget, entry};
use super::{AgentEventKind, AgentKind, AgentSpec, AgentSupport};

pub(super) const SPEC: AgentSpec = AgentSpec {
    id: "codex",
    label: "Codex",
    mark: "codex.svg",
    brand_color: None,
    executable: "codex",
    aliases: &[],
    packages: &["@openai/codex"],
    resume: &["resume", "{id}"],
    // SessionStart waits for the first prompt.
    reports_at_startup: false,
    support: AgentSupport::Full(HookSpec {
        format: HookFormat::NestedMap {
            events: HOOKS,
            shell: HookShell::PowerShell,
        },
        dir: |target: &HookTarget| target.env_dir("CODEX_HOME", ".codex"),
        file: "hooks.json",
        question_tool: None,
        adjust: None,
        prompt_ids: false,
        reply: None,
        enable: Some(enable_hooks),
        note: Some(
            "Codex runs hooks only with the hooks feature enabled and, unless managed, after they are trusted from /hooks inside Codex",
        ),
    }),
};

const HOOKS: &[HookEntry] = &[
    entry("SessionStart", AgentEventKind::SessionStart),
    entry("UserPromptSubmit", AgentEventKind::PromptSubmit),
    entry("PermissionRequest", AgentEventKind::PermissionRequest),
    entry("PreToolUse", AgentEventKind::ToolStart),
    entry("PostToolUse", AgentEventKind::ToolComplete),
    entry("Stop", AgentEventKind::Stop),
    // Codex clamps Interrupt hooks to 3 s and warns in the TUI about anything longer.
    HookEntry {
        timeout_secs: 3,
        ..entry("Interrupt", AgentEventKind::Interrupt)
    },
];

/// Codex ignores hooks.json until its hooks feature is on; the user can also set
/// `[features] hooks = true` in config.toml by hand, which the error text says.
fn enable_hooks() -> Result<(), String> {
    // Discovery also finds `codex.cmd`, which a bare `Command::new("codex")` cannot on
    // Windows.
    let codex = crate::agent_discovery::discover()
        .into_iter()
        .find(|found| found.kind == AgentKind::Codex)
        .map_or_else(|| SPEC.executable.into(), |found| found.executable);
    let hint = "set [features] hooks = true in Codex's config.toml";
    match std::process::Command::new(codex)
        .args(["features", "enable", "hooks"])
        .stdin(std::process::Stdio::null())
        .output()
    {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!(
            "codex features enable hooks failed ({}): {}; {hint}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(error) => Err(format!(
            "could not run codex to enable hooks: {error}; {hint}"
        )),
    }
}
