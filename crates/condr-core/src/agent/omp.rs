//! Oh My Pi, which loads Pi's extension from its own agent directory.

use super::hooks::{HookFormat, HookSpec, HookTarget};
use super::{AgentSpec, AgentSupport};
use std::path::PathBuf;

pub(super) const SPEC: AgentSpec = AgentSpec {
    id: "omp",
    label: "Oh My Pi",
    mark: "omp.svg",
    brand_color: None,
    executable: "omp",
    aliases: &["oh-my-pi"],
    packages: &["@oh-my-pi/pi-coding-agent"],
    resume: &["--session", "{id}"],
    reports_at_startup: true,
    reads_kitty_keys: false,
    support: AgentSupport::Full(HookSpec {
        format: HookFormat::Script {
            template: super::pi::EXTENSION,
            registry: None,
        },
        dir,
        file: "extensions/condr-omp.ts",
        question_tool: None,
        adjust: None,
        prompt_ids: false,
        reply: None,
        enable: None,
        note: Some(
            "Requires Oh My Pi 18.1.17 or newer; install into each profile's agent directory when using named profiles",
        ),
    }),
};

fn dir(target: &HookTarget) -> PathBuf {
    target.var("PI_CODING_AGENT_DIR").unwrap_or_else(|| {
        // OMP joins PI_CONFIG_DIR to home, even when it starts with a separator.
        let mut root = target.home.clone().into_os_string();
        root.push(std::path::MAIN_SEPARATOR_STR);
        root.push(target.var("PI_CONFIG_DIR").unwrap_or_else(|| ".omp".into()));
        PathBuf::from(root).join("agent")
    })
}
