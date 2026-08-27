# Murmur MVP Implementation and Acceptance Plan

## Outcome

Murmur MVP is a native GUI client for organizing ordinary shell terminals across one or more connected Servers, Sessions, Workspaces, Tabs, and split Panes. An agent CLI is an optional process that the user starts inside a Terminal; Murmur never selects or launches one automatically.

The GUI discovers an existing local `murmur-server` or starts the same standalone server used remotely, then connects through the common protocol. Closing the GUI only disconnects the client: Servers, Sessions, PTYs, and agents continue running. An empty active Session shows a Start Page with `New Terminal Workspace` and `Open Folder`; a non-empty Session shows its Server/Session/Workspace/Agent hierarchy, Tab row, and active Pane layout.

## Architecture Boundary

| Crate | Owns | Must not own |
| --- | --- | --- |
| `murmur-core` | Stable domain IDs and split layout, versioned protocol types, PTY and VT components, terminal I/O, agent detection, Git/worktree operations, Session Snapshot schema | GPUI entities, Dock runtime IDs, network listeners, GUI state |
| `murmur-server` | Stable Server identity, Session registry, live Terminal runtimes, protocol endpoint, client synchronization, Session Snapshot persistence | GPUI rendering, window focus, local-only behavior |
| `murmur-gui` | Connections to one or more Servers, local Server discovery/start, GPUI window and terminal element, input routing, Start Page, sidebar, Tab row, Dock projection | Authoritative domain state, PTY ownership, implicit Server shutdown |

`murmur-core` and `murmur-server` run Tokio; `murmur-gui` uses the GPUI executor. Every connection begins with a protocol-version handshake, and commands/events identify their Server and Session rather than assuming a singleton local runtime. Local and remote endpoints use the same TCP protocol; the local Server listens on loopback by default. Cross-machine MVP use goes through an SSH tunnel or an explicitly configured trusted endpoint until application authentication and authorization are implemented.

The Server continues consuming PTY output and updating VT state with no clients connected. Reconnecting to a running Server first receives the authoritative Session/layout state (Server, Session, Workspace, Tab, Pane order, active selections, focus, and cwd), then each Pane's live terminal view (grid, retained scrollback, foreground shell/Agent identity and status) before incremental events; no shell or Agent is recreated. This connection synchronization is distinct from durable Session Snapshot restore after a Server restart.

## Delivery Phases

Each phase starts only after the preceding exit condition holds.

| Phase | Deliverable | Exit condition |
| --- | --- | --- |
| 0. Build baseline | Cargo workspace, Apache-2.0 metadata, dependencies locked to reviewed revisions | `murmur-core` checks on Linux/arm64 and an empty `murmur-gui` window builds on Windows; no copied GPL Zed terminal code |
| 1. Core domain | Session, Workspace, Tab, Pane, stable Root Directory, split tree, focus/order, Pane-to-Tab-to-Workspace close cascade, durable snapshot schema | Headless tests prove domain invariants without GPUI or a real shell |
| 2. Server/client foundation | Standalone `murmur-server`, stable Server/Session addressing, versioned command/event protocol, local discovery/start, remote endpoint configuration, disconnect/reconnect | Headless integration proves the same client protocol reaches local and remote-style endpoints, rejects incompatible versions, preserves Sessions with zero clients, synchronizes authoritative structure on reconnect, and never exposes a non-loopback listener by default |
| 3. Terminal vertical slice | Server-owned `portable-pty` shell runtime, persistent `alacritty_terminal` state, ordered I/O, resize, input encoding, paste, scrollback, selection/copy, terminal synchronization | A real PTY integration test passes on Linux; one Windows shell/Agent Pane survives GUI disconnect/reconnect with its process, layout, and terminal state intact |
| 4. Native orchestration UI | Multi-Server navigation, Start Page, sidebar, Tab row, Dock projection, split/focus/resize/swap/zoom/close, command palette and fixed shortcuts | One GUI controls the complete Workspace/Tab/Pane workflow across local and remote Servers; Dock cannot mutate Server state independently |
| 5. Agent and Git workflows | Foreground agent recognition, bottom-buffer status rules, unseen `done`, branch display, create/open worktree, clean managed removal | State transitions and Git safety rules pass core/server tests and are visible for local and remote Sessions |
| 6. Persistence and failure handling | Debounced atomic Session Snapshot on the Server, Server-restart recovery with fresh shells, partial pruning, reconnect and Start Page fallbacks | Round-trip and corruption tests pass; live reconnect preserves running work, while Server restart restores structure but not processes or terminal history |
| 7. Release gate | Cross-platform checks, Windows-to-Linux remote evidence, documented limitations | Every required check below passes at one commit |

## Linux/arm64 Automated Gate

Run:

```bash
cargo fmt --all -- --check
cargo clippy -p murmur-core -p murmur-server --all-targets -- -D warnings
cargo test -p murmur-core -p murmur-server
```

The core/server suite must prove:

- An empty Session is valid. Workspace and Tab creation atomically creates one root Pane; a failed terminal spawn leaves no partial layout.
- Closing the only Pane closes its Tab; closing the last Tab closes its Workspace; closing the last Workspace yields an empty Session without stopping the Server.
- Stable IDs, names, order, active selections, focus history, split direction and ratios survive a durable snapshot round trip. Zoom does not.
- A Workspace Root Directory remains stable after Pane cwd changes. New Tabs follow the focused Pane cwd, Splits follow the target Pane cwd, and both fall back to the Root Directory.
- Protocol negotiation rejects incompatible versions. Commands and events identify Server and Session explicitly, and local/remote-style connections exercise the same behavior.
- The default Server endpoint is loopback-only. Disconnecting the final client keeps the Server, Sessions, PTYs, and Agents alive; reconnect returns the complete authoritative layout and live Pane views, including an Agent already running inside a Pane.
- A real PTY starts an ordinary shell in the requested cwd. Reads are single-source, writes remain ordered, EOF and child exit are reported once, and explicit Server shutdown reaps the child.
- One persistent VT state consumes fragmented output while no client is connected. Terminal replies return to the PTY writer; resize updates both the VT grid and PTY dimensions.
- Reconnection sends the complete Session/layout and terminal view before incremental updates; a running Agent is visible in its original Pane without a new shell. Legacy and application-cursor key encoding, bracketed paste, main-screen scrollback, simple selection/copy, UTF-8, and wide-cell behavior are covered.
- Output coalescing cannot lose the final terminal update notification.
- Agent detection distinguishes `unknown`, `idle`, `working`, and `blocked`; presentation derives `done` only from `idle + unseen`, and viewing the Pane clears `done`.
- Non-Git directories remain valid. Git discovery derives only metadata. Tests using temporary repositories cover branch display, new and existing branch worktree creation, opening an existing worktree, and explicit parent association.
- `Close Workspace` never invokes Git or deletes a checkout. Only a Murmur-created Managed Worktree can be removed; dirty or untracked files refuse removal; clean removal leaves the branch.
- Durable Snapshot persistence uses atomic replacement. Missing, empty, corrupt, or wholly unrestorable snapshots yield Start Page state. A failed Pane is pruned without discarding restorable siblings. Server-restart recovery starts fresh shells and never restores grid, scrollback, commands, live processes, Agent status, or conversations.

## Windows Manual Gate

Record the Windows version, commit, Rust toolchain, shell, GPU, Server endpoint, and agent CLI used for the representative smoke test. Build and run with the MSVC toolchain, then verify:

```powershell
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p murmur-gui
```

- First launch discovers or starts one detached local Server, shows Start Page, and starts no shell. `New Terminal Workspace` opens a shell in the user home; `Open Folder` opens one in the selected folder. Neither launches an agent CLI.
- PowerShell or the configured shell accepts typing, Enter, Backspace, navigation keys, Ctrl+C, ANSI color, bracketed paste, UTF-8/wide text, long scrolling output, selection, and clipboard copy.
- Rapid window and divider resizing updates the shell's reported rows/columns without freezing, blanking, or leaving stale regions.
- Start an identifiable long-running command and a representative Agent, close the GUI, and verify the Server, shell, and Agent remain alive. Reopen the GUI and verify it reconnects to the same Server/Session, restores the complete layout, and shows the Agent in its original Pane with current terminal content; only explicit Server stop terminates and reaps the child.
- Connect the same GUI to the local Windows Server and a Linux Server through an SSH tunnel. Both appear in the Server hierarchy and expose the same Workspace, Tab, Pane, terminal, agent, and Git commands.
- One representative agent CLI can be started manually and used interactively without product-specific launch code. Its recognized states appear in the sidebar; clicking it activates its Server, Session, Workspace, Tab, and Pane.
- New Tab, split right/down, Pane focus, divider and keyboard resize, swap, zoom, Tab/Workspace reorder, and close all preserve the intended focus. New Tabs are auto-named. A failed shell spawn leaves the previous layout usable.
- Closing a Pane or Tab does not inspect its foreground process. An operation that closes a Workspace asks for confirmation; closing a parent repository Workspace does not close associated worktree Workspaces.
- `Ctrl+Shift+P`, `Ctrl+Shift+T`, `Ctrl+Shift+W`, `Ctrl+Tab`, `Ctrl+Shift+Tab`, `Alt+Shift++`, `Alt+Shift+-`, `Alt+Arrow`, and `Alt+Shift+Arrow` invoke their documented commands without being sent to the shell.
- The current Git branch is shown. Create Worktree and Open Existing Worktree open the expected checkout. Closing either Workspace leaves files intact. Dirty removal is refused; clean managed removal deletes only the checkout and keeps the branch.
- Restarting a Server restores Workspace/Tab/Pane structure, names, layout, order, and cwd using fresh shells. Closing all Workspaces keeps the Server Session empty and the connected GUI on Start Page.

Any crash, deadlock, data loss, shell input/output failure, accidental Server shutdown on client disconnect, unauthenticated default network exposure, incorrect delete authority, unreaped process, or structural restore error blocks the MVP. Cosmetic defects may be deferred only when recorded with reproduction steps and they do not obscure terminal content or controls.

## Handoff Evidence

The release candidate must include:

- A locked `Cargo.lock` and the reviewed GPUI/gpui-component revisions.
- Passing Linux command output from the required commit.
- A completed Windows checklist with environment details and screenshots or a short recording of local reconnect, the remote Linux Server, terminal/layout/worktree flows, and Server restart.
- Updated `CONTEXT.md` and ADRs when implementation discovers a domain change. Behavioral changes require updating this acceptance plan before merge.
- A known-limitations list matching the exclusions below.

## Accepted MVP Limits

- One native GUI window can connect to multiple independently running Servers. Simultaneous collaborative control of one Session by multiple GUI clients is not guaranteed.
- Local and remote Servers have identical product behavior. The MVP remote path uses an SSH tunnel or explicitly trusted network; application authentication, authorization, encryption, account management, and public Internet exposure are deferred. Servers bind loopback by default.
- GUI reconnect preserves the authoritative Session/layout plus live processes and terminal state held by a running Server, including a running Agent and its visible Pane. Server restart performs structural Session restore with fresh shells and does not preserve terminal history, process, command, Agent conversation, or live Agent state.
- No automatic agent launch, agent-specific terminal path, hook installer, or agent session resume. Status remains heuristic and unsupported CLIs may stay `unknown`.
- No multi-window Pane undock, Pane Stack, free Pane drag/drop, or cross-Workspace live Pane move.
- Fixed shortcuts; no herdr prefix/navigation/resize modes and no keybinding remap.
- Start Page has no Recent Workspaces list.
- No terminal application mouse reporting, CSI-u/Kitty keyboard, focus/bell/title handling, OSC 8/52, image protocols, search, or copy mode unless a target CLI demonstrates a blocking need.
- Branch display only; no dirty or ahead/behind presentation, bare-repository workflow, forced worktree removal, branch deletion, or configurable worktree root.
- Windows cwd following uses the best ConPTY/shell signal available and falls back to the stable Workspace Root Directory; Unix foreground-process cwd fidelity is not assumed on Windows.
