//! Goose, recognized only (ADR 0035, ADR 0071).

use super::{AgentSpec, AgentSupport, InitialPrompt};

pub(super) const SPEC: AgentSpec = AgentSpec {
    id: "goose",
    label: "Goose",
    mark: "goose.svg",
    brand_color: None,
    executable: "goose",
    aliases: &[],
    packages: &[],
    resume: &["session", "--resume", "--session-id", "{id}"],
    // From 1.56 it reports Idle over OSC 7501 once its prompt shows.
    reports_at_startup: true,
    reads_kitty_keys: false,
    // A first prompt takes `goose run`, not the interactive session.
    initial_prompt: InitialPrompt::None,
    support: AgentSupport::RecognitionOnly(
        "Goose's hooks start a subagent's session as they start the main one, with nothing to tell them apart, so Condr installs none: its state shows only as Goose reports it to the terminal, and its conversations do not resume",
    ),
};
