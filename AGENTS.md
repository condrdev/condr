# AGENTS.md

Guidance for coding agents working in this repository.

## What Condr is

**Condr** (short for *conductor*) is a native multi-agent GUI. It orchestrates several coding-agent CLIs (Claude Code, Codex, OpenCode, …) side by side, each running in its own terminal pane, and shows their state at a glance.

The design bet: embed the vendor's own agent CLI as a subprocess instead of re-implementing a chat loop, and render everything with a native GUI instead of a webview. That keeps Condr cross-platform, light, and fast while staying easy to pick up.

## Tech stack

- **Rust** + **[GPUI Kit](https://github.com/longbridge/gpui-kit)** (re-exports GPUI, the base layer, components and assets)
- Agent backend: native agent CLIs embedded as subprocesses; Condr never runs its own conversation/tool loop

### Decided choices

- **License: Apache-2.0.** Only borrow ideas from GPL-licensed terminal implementations; never copy their code.
- **VT emulation: `alacritty_terminal`.** Pure Rust, no FFI, already proven under GPUI renderers.
- **PTY: `portable-pty`** (Unix pty / Windows ConPTY).
- **Async: `tokio` in server/core; the GPUI executor in the GUI.** The two talk only through the protocol.
- **Transport: versioned binary protocol over transport adapters.** Frames are `bincode + serde` with a length prefix and a strict version handshake. Locally, `interprocess` Unix domain sockets / Windows named pipes; remotely, the same protocol over a TCP endpoint. Every TCP connection is authenticated and encrypted with WireGuard-style `Noise_IKpsk2` (`snow`) using mutual static keys; new devices pair with a one-time invite (ADR 0011). Capability-based authorization comes later. SSH connections run the system `ssh -T` to execute `condr server bridge` remotely and forward the remote private socket; SSH handles auth and encryption (ADR 0015). The Server only exposes a local private endpoint by default and never listens on the public internet.
- **Persistence: `bincode + serde`** for sessions, **TOML** for config.
- **Terminal rendering: a custom GPUI element.** This is the largest home-grown piece; GPUI Kit has no terminal component.
- **Layout: GPUI Kit's Dock** (`gpui_kit::component::dock`), mapped onto the workspace/tab/pane model.
- **Agent state detection: only from the agent CLI's own hooks** (ADR 0014). The process table identifies the agent; `condr agent-hook` writes hook events back to the controlling terminal as OSC 777; the Server intercepts them ahead of the VT and drives `AgentState`. Nothing reported means `Unknown`; there is no screen-text classification. The GUI layers "done" on top.
- **Git: `gix` (gitoxide, pure Rust).** All read-only queries and worktree add/remove go through the gix API. Never shell out to `git`, never add git2 (libgit2 C dependency). gix has no `worktree add/remove` porcelain, so `condr-core/src/git.rs` builds it from primitives (ref transactions, `index_from_tree`, `gix::worktree::state::checkout`) following git's own `worktrees/<id>` layout.
- **Dependencies: declare only `gpui-kit = "0.6"` from crates.io.** Use `gpui_kit::*` for GPUI types, `gpui_kit::component` for components, `gpui_kit::assets` for assets; create the app with `gpui_kit::application()` and initialize with `gpui_kit::init(cx)`. The matching `gpui-pre` crates are managed by Kit and pinned by `Cargo.lock`; do not declare GPUI/platform/component/asset crates directly. GUI tests use `gpui-kit/test-support`.

### Architecture and repository layout

Condr has been a separate server/client system from the first version. "Local" is not another backend: the GUI simply discovers or starts the same `condr server` on this machine and connects to it. One machine runs exactly one Server. It always listens on a local private socket and, when `[server] listen` is configured, on one extra TCP address; both serve the same Session (ADR 0013). All orchestration lives in the Server, so the CLI and the Server are the same binary `condr`. The GUI is a separate binary `condr-gui` and is just one Server client:

```
crates/condr-core    # domain, protocol, PTY, VT, agent detection, Git — no GUI deps, testable headless
crates/condr-server  # builds `condr`: the Server process (`condr server …`) and CLI subcommands; owns Session, terminal runtime, persistence and connections
crates/condr-gui     # builds `condr-gui`: pure client, connects to one or more servers, renders with GPUI
```

Closing the GUI only disconnects. The server, PTYs, agents and Session keep running; reopening the GUI reconnects to the existing local server first. Stopping the server is an explicit action.

### Website (`website/`)

[condr.dev](https://condr.dev) lives in `website/`: Astro + Starlight, pnpm, deployed by Cloudflare Pages from `main` (root directory `website`). It is documentation, not part of the Rust build: `ci.yml` ignores it, and `website/README.md` has its own commands. `website/public/install.sh` and `install.ps1` are the online installers behind `curl -fsSL https://condr.dev/install.sh | sh`; keep them in step with `script/install-condr.*` and the README install table.

### Development environment (two machines)

- **Linux server (arm64, headless):** all condr-core development and tests (`cargo test/clippy` need no display). The GUI cannot run here.
- **Windows laptop:** native GUI builds and manual verification (GPUI does not cross-compile), plus the ConPTY path.

### Pre-release compatibility policy

- The project is unreleased. Breaking changes are allowed; backward compatibility is not required.
- Prefer the simplest final design with clear boundaries. Do not keep compatibility layers, migration paths or deprecated APIs just for old implementations.
- When breaking something, update in-repo callers, tests and source docs in the same change. Implement migrations only when the task explicitly asks for them.

## Common commands

```bash
cargo build            # build
cargo run              # run
cargo test             # all tests
cargo test <name>      # one test
cargo clippy           # lint
cargo fmt              # format
```

## Architecture principles

- The GUI only orchestrates and renders; the conversation/tool loop belongs to the embedded agent CLI subprocess.
- The server is the sole owner of Session, PTY, VT, agent and Git/worktree runtime; local and remote GUI features use the same protocol. On reconnect a client first fetches the authoritative Server/Session structure snapshot and each Pane's live terminal view, then subscribes to incremental events. Closing the GUI never stops the server or its children.
- Stay light: no webview, no unnecessary dependencies.
- GUI sizes: a length that text flows into (row heights, label widths, indents, menu widths) is `rems`, so it follows the interface font; pure geometry (hairlines, drag handles, icon slots, widths the user dragged, insets that mirror platform or Kit pixel constants, Dock pixel arithmetic) is `px`. Both are logical pixels; DPI scaling is GPUI's.

### Terminal rendering performance requirements

The terminal is Condr's core surface. It must feel as smooth as a native terminal; stutter is not a cosmetic issue to defer. While a representative agent CLI (especially Codex) streams output and redraws spinners/animations, mouse drag-selection, keyboard input, scrolling and Pane operations must stay responsive, targeting 60 Hz display cadence with no backlog of stale frames.

- A PTY read chunk is a terminal-state wakeup, not a GUI frame that must be shown one-for-one. The Server must coalesce consecutive wakeups, publish the latest state, and never drop the final/exit frame. An unbounded queue of old `TerminalView`s must not add input latency.
- The GUI consumes terminal events in batches and drops or overwrites stale intermediate visual states; control, lifecycle and layout events stay reliable and ordered.
- Terminal visual updates use a per-client stream independent of the reliable Session event cursor: each client writer has exactly one droppable batched visual slot, and reliable messages take priority. The client's baseline advances only after a visual frame is successfully slotted; when the slot is full, only record the Panes that need refresh, and after the writer drains, regenerate the latest frame from the Server's authoritative VT state. Bootstrap must clear queued visual frames and reset the baseline; when the GUI detects a revision gap it requests exactly one new Bootstrap.
- A small terminal change must not invalidate the whole-screen shaping cache. Invalidate by actual cell/row/run content and style; avoid per-frame whole-screen string allocation, whole-screen shaping and needless per-cell paint. When full views become the bottleneck, add per-client baseline/damage deltas while keeping reconnect bootstrap correct.
- Pure GUI interactions such as an in-progress drag-selection stay local to the Client. Once the drag ends, the selection is handed to the Server's VT tracking (it scrolls with output and clears on resize/alt screen); view frames carry the selection clipped to the viewport, and the Client draws it and uses it for copy. Rendering caches for unchanged content must be reused during continuous output too, not just on a static screen.
- The ordinary dev command `cargo run -p condr-gui` must also have a usable frame rate. Do not remove the dev-profile optimizations in the root `Cargo.toml` for GPUI, text shaping, VT and Condr hot paths unless an equivalent replacement is in place and verified on Windows.
- Performance changes must at least cover: burst wakeups coalescing to the latest revision, the final frame never dropping, unchanged cells not being re-shaped across revisions, and real GPUI drag-selection. A single drag-selection test on a static terminal is not enough; Windows acceptance also requires manually observing interaction and frame rate under representative agent animation / high-frequency output.
- Before optimizing, locate the real hot spot among parse, snapshot/serialization, event queue, prepaint/shaping and paint. When studying other terminal renderers for ideas, respect the license boundary above: Condr is Apache-2.0, so GPL code may only inform the approach, never be copied.

## Agent skills

### Issue tracker

GitHub Issues via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Five default labels: needs-triage / needs-info / ready-for-agent / ready-for-human / wontfix. See `docs/agents/triage-labels.md`.

### Domain docs

Single context: root `CONTEXT.md` + `docs/adr/`. See `docs/agents/domain.md`.
