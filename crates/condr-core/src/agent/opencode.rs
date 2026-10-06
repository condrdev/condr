//! OpenCode.

use super::hooks::{HookFormat, HookSpec, HookTarget};
use super::{AgentSpec, AgentSupport};

pub(super) const SPEC: AgentSpec = AgentSpec {
    id: "opencode",
    label: "OpenCode",
    mark: "opencode.svg",
    brand_color: None,
    executable: "opencode",
    aliases: &[],
    packages: &["opencode-ai"],
    resume: &["--session", "{id}"],
    reports_at_startup: true,
    support: AgentSupport::Full(HookSpec {
        // OpenCode's TUI owns the selected conversation; backend events may belong to
        // other roots or subagents. The plugin reads the current route and its native
        // status.
        format: HookFormat::Script {
            template: include_str!("opencode.js"),
            registry: Some("tui.json"),
        },
        // `$XDG_CONFIG_HOME/opencode` or `~/.config/opencode`, on every platform.
        dir: |target: &HookTarget| {
            target
                .var("XDG_CONFIG_HOME")
                .unwrap_or_else(|| target.home.join(".config"))
                .join("opencode")
        },
        file: "condr-tui.js",
        question_tool: None,
        adjust: None,
        prompt_ids: false,
        reply: None,
        enable: None,
        note: None,
    }),
};
