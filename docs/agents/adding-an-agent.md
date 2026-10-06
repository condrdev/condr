# Adding an agent CLI

Condr supports only the agent CLIs it ships an integration for (ADR 0035). A CLI that is not supported arrives through an Issue (the "Agent CLI request" template) or a pull request that adds it. Everything Condr knows about one agent lives in one `AgentSpec`.

## Pick the level

The level follows from what the CLI's own hooks can report, never from how far its integration has been tried.

- **Fully supported**: the hooks report a turn starting, and the main agent's turn ending told apart from a subagent's, so `Working` and `Idle` can be trusted. An agent with no event for a permission prompt or a question still qualifies; its `note` says it never shows `Blocked` for that.
- **Recognized only**: the process table names it, but its hooks cannot report a state worth trusting. Its `support` is `AgentSupport::RecognitionOnly(reason)`, nothing is installed, and it stays `Unknown`. Kimi is the example.

Never guess a state from the screen (ADR 0014). If the hooks cannot tell the main agent's Stop from a subagent's, the agent is recognized only.

## What to change

1. **The spec.** Add `crates/condr-core/src/agent/<id>.rs` holding `pub(super) const SPEC: AgentSpec`. Copy the agent closest to the new one's hook format: `claude.rs` for a nested JSON hook map, `antigravity.rs` for a plugin with a manifest, `cursor.rs` or `copilot.rs` for a flat list, `opencode.rs` or `pi.rs` for a script Condr owns. Every field is required. Decide `reports_at_startup` from the CLI's documented events: an agent that sends nothing before its first prompt must say `false`, or `agent start` waits out its timeout. Leave `reads_kitty_keys` `false` unless the CLI was seen to act on a kitty key report it never negotiated: on a Windows Server Condr sends such an agent Cmd+C in kitty form, and anything the agent runs in the foreground receives it too.
2. **The identity.** Add the module to `agent/mod.rs`, a value to `AgentKind`, `AgentKind::ALL` and `spec()`, and an `AGENT_KIND_<ID>` after the last value in `proto/condr/v1/session.proto`. The compiler names each `match` that misses it. An older peer reads the new value as `OTHER` (ADR 0028).
3. **What only this agent needs.** Put it in its module: a field when a value says it (`question_tool`, `prompt_ids`, `reply`), a function when it takes logic (`adjust`, `enable`). A new hook format goes into `agent/hooks.rs` only when no existing one fits.
4. **The mark.** Add `assets/agents/<id>.svg`, recolored to `currentColor`, with its line in `assets/agents/NOTICE`, then add it to `CONDR_ICON_PATHS` and the `load` match in `crates/condr-gui/src/assets.rs`. Run `script/generate-licenses` so the Settings › Licenses page, `crates/condr-gui/assets/licenses.md`, carries the new NOTICE line.
5. **The snapshot.** Run `CONDR_UPDATE_SNAPSHOTS=1 cargo test -p condr-core --test agent_hooks every_agent_installs_exactly_its_snapshot` and review every file it writes under `crates/condr-core/tests/hook_snapshots/<id>/`. A fully supported agent must write at least one file; a recognized-only agent writes none.
6. **The documentation.** Add the agent to its level on the website's Agents page (`website/src/content/docs/zh-cn/docs/using/agents.md` first, then the English page), to the `--kind` list in both CLI references, and to the `agents` list on the home page, `website/src/pages/index.astro`. If its `reports_at_startup` is `false`, add it to the Agents the troubleshooting page names as sending no state before their first prompt, in both languages. Record the CLI version, the hook schema it was built against, and which platforms have run a real session through the whole chain in `docs/research/additional-agent-hooks.md`.

## Check it

- `cargo test -p condr-core` covers the spec table, process recognition and the snapshots.
- `cargo test -p condr-gui` covers the mark: its asset test fails when `CONDR_ICON_PATHS` or the `load` match misses it.
- Run one real session in a Condr Pane: install the hooks from Settings, start the CLI, submit a prompt, and watch the sidebar go from `Working` to `Idle`. On Windows, the shell that runs the hook command decides whether a quoted path needs PowerShell's call operator, so check the hooks run there too when the CLI uses PowerShell.
- A gap in real-session coverage is a known limitation to record, not a reason to change the agent's level.
