//! Pi.

use super::hooks::{HookFormat, HookSpec, HookTarget};
use super::{AgentSpec, AgentSupport};

/// The extension Pi and Oh My Pi both load, named for the agent it is installed into.
pub(super) const EXTENSION: &str = include_str!("pi-extension.js");

pub(super) const SPEC: AgentSpec = AgentSpec {
    id: "pi",
    label: "Pi",
    mark: "pi.svg",
    brand_color: None,
    executable: "pi",
    aliases: &[],
    packages: &[
        "@earendil-works/pi-coding-agent",
        "@mariozechner/pi-coding-agent",
    ],
    resume: &["--session", "{id}"],
    reports_at_startup: true,
    reads_kitty_keys: false,
    support: AgentSupport::Full(HookSpec {
        format: HookFormat::Script {
            template: EXTENSION,
            registry: None,
        },
        dir: |target: &HookTarget| target.env_dir("PI_CODING_AGENT_DIR", ".pi/agent"),
        file: "extensions/condr-pi.ts",
        question_tool: None,
        adjust: None,
        prompt_ids: false,
        reply: None,
        enable: None,
        note: Some("Requires Pi 0.85.1 or newer for settled and UI prompt events"),
    }),
};
