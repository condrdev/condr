//! Kilo Code, a fork of OpenCode that takes the same TUI plugin.

use super::hooks::{HookFormat, HookSpec, HookTarget};
use super::{AgentSpec, AgentSupport, InitialPrompt};

pub(super) const SPEC: AgentSpec = AgentSpec {
    id: "kilo",
    label: "Kilo Code",
    mark: "kilo.svg",
    brand_color: None,
    executable: "kilo",
    aliases: &["kilocode"],
    packages: &["@kilocode/cli"],
    resume: &["--session", "{id}"],
    reports_at_startup: true,
    reads_kitty_keys: false,
    initial_prompt: InitialPrompt::Flag("--prompt"),
    support: AgentSupport::Full(HookSpec {
        format: HookFormat::Script {
            template: include_str!("opencode.js"),
            registry: Some("tui.json"),
        },
        // `$XDG_CONFIG_HOME/kilo` or `~/.config/kilo`, on every platform.
        dir: |target: &HookTarget| {
            target
                .var("XDG_CONFIG_HOME")
                .unwrap_or_else(|| target.home.join(".config"))
                .join("kilo")
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
