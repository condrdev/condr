//! Kimi Code, recognized only (ADR 0035).

use super::{AgentSpec, AgentSupport};

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
    support: AgentSupport::RecognitionOnly(
        "Kimi 1.50 hooks cannot distinguish a subagent's Stop from the main agent's Stop; status integration is unavailable until native hooks identify their agent",
    ),
};
