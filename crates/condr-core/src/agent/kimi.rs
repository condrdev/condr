//! Kimi Code, recognized only (ADR 0035).

use super::{AgentSpec, AgentSupport, InitialPrompt};

pub(super) const SPEC: AgentSpec = AgentSpec {
    id: "kimi",
    label: "Kimi Code",
    mark: "kimi.svg",
    brand_color: None,
    executable: "kimi",
    aliases: &["kimi-code"],
    packages: &[],
    resume: &["--session", "{id}"],
    reports_at_startup: false,
    reads_kitty_keys: false,
    initial_prompt: InitialPrompt::None,
    support: AgentSupport::RecognitionOnly(
        "Kimi Code's hooks report a subagent's Stop under the main session's ID, so Condr installs none: its state shows only as Kimi Code reports it to the terminal, and its conversations do not resume",
    ),
};
