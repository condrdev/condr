//! MiniMax Code, recognized only (ADR 0035, ADR 0071).

use super::{AgentSpec, AgentSupport, InitialPrompt};

pub(super) const SPEC: AgentSpec = AgentSpec {
    id: "minimax",
    label: "MiniMax Code",
    mark: "minimax.svg",
    // MiniMax's pink, as the published mark carries it.
    brand_color: Some(0xE73562),
    executable: "mcode",
    aliases: &["minimax-code"],
    packages: &["@minimax-ai/code"],
    resume: &["--session", "{id}"],
    // It reports over OSC 7501 (`app=mcode`) once its prompt shows.
    reports_at_startup: true,
    reads_kitty_keys: false,
    initial_prompt: InitialPrompt::Positional,
    support: AgentSupport::RecognitionOnly(
        "MiniMax Code loads hooks only from plugins its own plugin system has installed, which Condr cannot add to: its state shows only as MiniMax Code reports it to the terminal, and its conversations do not resume",
    ),
};
