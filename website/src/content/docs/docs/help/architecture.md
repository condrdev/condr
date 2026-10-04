---
title: Architecture
description: The processes and crates that make up Condr, how terminal frames and Agent state flow, and where to start reading when you contribute.
---

Condr splits the runtime and the interface into two processes. The Server hosts terminals, Agents and the Session; the window is just one client connected to the Server. This page is for developers who plan to read or change Condr's code.

---

## Process model

* **One Server per machine**: the Server always listens on a private local socket (a named pipe on Windows). With `[server] listen` configured, the Server opens one extra TCP listening port; with Peer-to-peer turned on, it also accepts P2P connections. Every entry point reaches the same Session.
* **The Server and the command line are one binary**: `condr server …` manages the Server, and every other subcommand connects to the Server as a client.
* **The window runs as a separate client**: on startup `condr-gui` first probes the local Server. If nothing answers, it starts `condr server run --detached` as a background process. Closing the window only disconnects; the Server, terminals and Agents keep running.
* **Single window instance**: on startup the window locks `condr.lock` in the runtime directory. If the lock is already held, the new instance exits at once.

---

## Crates

| Crate | Builds | Responsibilities |
| :--- | :--- | :--- |
| `condr-core` | Library | Domain model, protocol, PTY and terminal emulation, Agent detection, Git. No GUI dependencies, so it can be tested without a display |
| `condr-server` | `condr` | The Server process and the command line. Manages the Session, the terminal runtime, persisted state and every connection type |
| `condr-gui` | `condr-gui` | The window. Renders with GPUI Kit, and reuses `condr-server`'s client connection code as a library |

---

## Terminal frames

1. The Server creates the PTY through `portable-pty`. On Windows it uses ConPTY, and each terminal's processes go into their own Job Object, so closing a Pane ends them together.
2. Before reaching the terminal emulator, PTY output passes an OSC scan: sequences such as OSC 7 update the current working directory, and Agent events in OSC 777 are extracted and stripped from the raw output.
3. The remaining bytes are parsed by `alacritty_terminal`.
4. The Server coalesces wakeups at 60 Hz and reads the changed region once: a full-screen change produces a complete frame, a partial change produces only the changed cells.
5. The Server builds each client's frame against the frame that client already has, and sends it over the protocol.
6. The window draws the terminal with a custom GPUI element and caches the shaping results of unchanged content.

---

## Reliable events and visual frames

* **Reliable events**: layout changes, lifecycle, Agent state and Git changes are sent in order with a replay cursor, so none is lost.
* **Visual frames**: terminal frames travel on a separate channel. Each client gets only one droppable frame slot, and reliable messages always go first. When the slot is full, the Server records only which Panes need a refresh; once the slot frees up, it builds a new frame straight from the latest terminal state and never replays stale intermediate frames.
* **Reconnecting**: after connecting, the window first requests a Bootstrap (the full structure snapshot plus each Pane's current frame), and subscribes to live events once that is in sync. If it detects a gap in the event sequence, the window requests one new Bootstrap.

---

## Agent state

1. `condr agent hooks install` writes the hooks into the Agent's own configuration.
2. When the Agent fires a hook, it runs `condr agent-hook <agent> <event>`. That command reads the JSON the Agent passes and writes it back to the current terminal as an OSC 777 sequence.
3. The Server intercepts the sequence in the PTY output and updates that Pane's Agent state to Unknown, Idle, Working or Blocked. A Blocked state can carry a `blocked_on` field that says what it waits for.
4. If the Agent has never reported, its state stays Unknown. Condr does not infer Agent state from screen text.

Done exists only in the window: when an Agent turns Idle while focus is elsewhere, the window shows it as Done.

---

## Connection types

| Type | Implementation |
| :--- | :--- |
| Local | Unix domain sockets or Windows named pipes from `interprocess` |
| TCP | `Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s` implemented with `snow`; both sides authenticate with static keys, and a new Device pairs with a one-time invite |
| SSH | Runs the system `ssh -T` to execute `condr server bridge` on the remote Device, forwarding the remote private socket to this machine. Authentication and transport encryption are left entirely to OpenSSH |
| Peer-to-peer | QUIC connections from `iroh`, dialled by Device key with NAT hole punching; when a direct connection fails, traffic is relayed through `relay.condr.dev` |

`condr --device <name>` connects directly to a Device saved in `[[client.servers]]`, without going through the local Server.

---

## Protocol

* **Encoding**: frames are Protocol Buffers, delimited by a varint length prefix, at most 2 MiB each. The definitions are in `proto/condr/v1/` and compiled by `condr-core`'s `build.rs`. Generated types are used only inside `protocol::pb` and are converted to domain types at the module boundary.
* **Handshake**: the client sends a `ClientHandshake`, and the Server answers with a `Welcome` (which carries a refusal reason when the handshake fails). The handshake messages' Protobuf field numbers never change, and both sides report their build version during the handshake.
* **Compatibility**: on connecting, each side states the oldest peer protocol version it accepts. Fields and oneof members may only be added, never removed; an unknown enum value is treated as the default documented in the `.proto` file; an unknown oneof member causes the enclosing message to be dropped, without disconnecting. When the two builds differ, the window only shows a warning mark.

---

## Persistence

| File | Format | Contents |
| :--- | :--- | :--- |
| `condr-server-<id>.snapshot` | Protocol Buffers | The Session structure: Workspaces, Tabs, the Pane layout and its split ratios, working directories, Agent kind and session ID for resuming, and worktree associations. No terminal scrollback |
| `condr-gui.state` | Protocol Buffers | Window position, sidebar widths, and what each Device and Workspace is showing. Read only by the window; the Server never touches it |
| `config.toml` | TOML | The settings file shared by the window, the Server and the command line. Every read and write goes through `condr_core::{read_config_value, update_config_values}`, behind a cross-process file lock that keeps the existing formatting |

---

## Main dependencies

| Dependency | Role |
| :--- | :--- |
| `gpui-kit` | GPUI, UI components, the Dock layout and assets |
| `alacritty_terminal` | Terminal emulation and VT sequence parsing |
| `portable-pty` | Cross-platform PTYs (Unix pty and Windows ConPTY) |
| `gix` | Every Git query, plus creating and removing worktrees. Never calls the `git` command and does not link libgit2 |
| `snow` | The Noise handshake for TCP connections |
| `iroh` | P2P QUIC transport and hole punching |
| `syntect` and `two-face` | Syntax highlighting in the Preview and Diff views |
| `tracing` | Structured logging |
| `tokio` | The async runtime in the Server and core. The window uses GPUI's own executor |

---

## Where to start reading

| Area | Entry points |
| :--- | :--- |
| Terminal rendering | `crates/condr-gui/src/terminal_element.rs`, `crates/condr-server/src/server/terminal_stream.rs`, `crates/condr-server/src/client_writer.rs` |
| Terminal input | `crates/condr-gui/src/app/terminal_input.rs`, `crates/condr-core/src/terminal/input.rs` |
| Protocol | `proto/condr/v1/`, `crates/condr-core/src/protocol.rs`, `crates/condr-server/src/server/client.rs` |
| Agent detection | `crates/condr-core/src/agent/`, `crates/condr-core/src/terminal/osc.rs`, `crates/condr-server/src/server/agents.rs` |
| Git and worktrees | `crates/condr-core/src/git.rs`, `crates/condr-server/src/server/workspace_git.rs` |
| Settings | `crates/condr-gui/src/app/settings.rs`, `crates/condr-gui/src/app/config.rs` |

[`docs/code-organization.md`](https://github.com/condrdev/condr/blob/main/docs/code-organization.md) lists every module's entry point.

---

## Build and test

```sh
cargo build                                  # build everything
cargo run -p condr-gui                       # run the window; needs a display
cargo test --workspace --features condr-gui/test-support
cargo clippy --workspace --all-targets --features condr-gui/test-support -- -D warnings
cargo fmt --all
```

* **Build the GUI on its target OS**: GPUI does not cross-compile, so the window must be built natively on the target system. `condr-core` and `condr-server` build and test without a display.
* **Optimized dev builds**: the root `Cargo.toml` turns on optimization for GPUI, text shaping, terminal emulation and Condr's hot paths, so `cargo run -p condr-gui` keeps a usable frame rate in a dev build.

See [CONTRIBUTING.md](https://github.com/condrdev/condr/blob/main/CONTRIBUTING.md) for how to contribute.

---

## Design decisions

Major architecture decisions are recorded in the [ADR](https://github.com/condrdev/condr/tree/main/docs/adr) directory. Read the relevant record before changing a module.
