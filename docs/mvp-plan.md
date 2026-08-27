# Murmur MVP Implementation and Acceptance Plan

## Outcome

Murmur MVP is a single native window for organizing ordinary shell terminals across Workspaces, Tabs, and split Panes. An agent CLI is an optional process that the user starts inside a Terminal; Murmur never selects or launches one automatically.

The empty Session shows a Start Page with `New Terminal Workspace` and `Open Folder`. A non-empty Session shows the Workspace/Agent sidebar, the active Workspace's Tab row, and the active Tab's Pane layout.

## Architecture Boundary

| Crate | Owns | Must not own |
| --- | --- | --- |
| `murmur-core` | Stable domain IDs and split layout, PTY and VT state, terminal I/O, agent detection, Git/worktree operations, Session Snapshot | GPUI entities, Dock runtime IDs, rendering |
| `murmur-gui` | GPUI window and terminal element, input routing, Start Page, sidebar, Tab row, Dock projection | Domain truth, PTY lifecycle policy, persisted Dock state |

`murmur-core` runs Tokio. `murmur-gui` uses the GPUI executor. Typed channels carry terminal input, output notifications, resize requests, and domain events across the boundary. The MVP remains one process; this boundary preserves a later server/client split without implementing it now.

## Delivery Phases

Each phase starts only after the preceding exit condition holds.

| Phase | Deliverable | Exit condition |
| --- | --- | --- |
| 0. Build baseline | Cargo workspace, Apache-2.0 metadata, dependencies locked to reviewed revisions | `murmur-core` checks on Linux/arm64 and an empty `murmur-gui` window builds on Windows; no copied GPL Zed terminal code |
| 1. Core domain | Session, Workspace, Tab, Pane, stable Root Directory, split tree, focus/order, Pane-to-Tab-to-Workspace close cascade, snapshot schema | Headless tests prove domain invariants without GPUI or a real shell |
| 2. Terminal vertical slice | `portable-pty` shell runtime, persistent `alacritty_terminal` state, ordered I/O, resize, keyboard encoding, paste, scrollback, selection/copy, repaint notification | A real PTY integration test passes on Linux and one interactive shell Pane works on Windows |
| 3. Native orchestration UI | Start Page, sidebar, Tab row, Dock projection, split/focus/resize/swap/zoom/close, command palette and fixed shortcuts | The complete Workspace/Tab/Pane workflow works with ordinary shells; Dock cannot mutate core state independently |
| 4. Agent and Git workflows | Foreground agent recognition, bottom-buffer status rules, unseen `done`, branch display, create/open worktree, clean managed removal | State transitions and Git safety rules pass core tests and are visible in the Windows GUI |
| 5. Restore and failure handling | Debounced atomic Session Snapshot, structural restore with fresh shells, partial pruning and Start Page fallbacks | Round-trip and corruption tests pass; Windows restart restores structure but not processes or terminal history |
| 6. Release gate | Cross-platform checks, Windows evidence, documented limitations | Every required check below passes at one commit |

## Linux/arm64 Automated Gate

Run:

```bash
cargo fmt --all -- --check
cargo clippy -p murmur-core --all-targets -- -D warnings
cargo test -p murmur-core
```

The core suite must prove:

- An empty Session is valid. Workspace and Tab creation atomically creates one root Pane; a failed terminal spawn leaves no partial layout.
- Closing the only Pane closes its Tab; closing the last Tab closes its Workspace; closing the last Workspace yields an empty Session rather than exiting the application model.
- Stable IDs, names, order, active selections, focus history, split direction and ratios survive a snapshot round trip. Zoom does not.
- A Workspace Root Directory remains stable after Pane cwd changes. New Tabs follow the focused Pane cwd, Splits follow the target Pane cwd, and both fall back to the Root Directory.
- A real PTY starts an ordinary shell in the requested cwd. Reads are single-source, writes remain ordered, EOF and child exit are reported once, and shutdown reaps the child.
- One persistent VT state consumes fragmented output. Terminal replies are returned to the PTY writer; resize updates both the VT grid and PTY dimensions.
- Legacy and application-cursor key encoding, bracketed paste, main-screen scrollback, simple selection/copy, UTF-8 and wide-cell grid behavior are covered.
- Output coalescing cannot lose the final repaint notification.
- Agent detection distinguishes `unknown`, `idle`, `working`, and `blocked`; presentation derives `done` only from `idle + unseen`, and viewing the Pane clears `done`.
- Non-Git directories remain valid. Git discovery derives only metadata. Tests using temporary repositories cover branch display, new and existing branch worktree creation, opening an existing worktree, and explicit parent association.
- `Close Workspace` never invokes Git or deletes a checkout. Only a Murmur-created Managed Worktree can be removed; dirty or untracked files refuse removal; clean removal leaves the branch.
- Snapshot persistence uses atomic replacement. Missing, empty, corrupt, or wholly unrestorable snapshots yield Start Page state. A failed Pane is pruned without discarding restorable siblings. Restored Panes request fresh shells and never carry grid, scrollback, commands, live processes, Agent status, or conversations.

## Windows Manual Gate

Record the Windows version, commit, Rust toolchain, shell, GPU, and the agent CLI used for the representative smoke test. Build and run with the MSVC toolchain, then verify:

```powershell
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p murmur-gui
```

- First launch shows Start Page and starts no shell. `New Terminal Workspace` opens a shell in the user home; `Open Folder` opens one in the selected folder. Neither launches an agent CLI.
- PowerShell or the configured shell accepts typing, Enter, Backspace, navigation keys, Ctrl+C, ANSI color, bracketed paste, UTF-8/wide text, long scrolling output, selection and clipboard copy.
- Rapid window and divider resizing updates the shell's reported rows/columns without freezing, blanking, or leaving stale regions.
- One representative agent CLI can be started manually and used interactively without product-specific launch code. Its recognized states appear in the sidebar; clicking it activates its Workspace, Tab, and Pane.
- New Tab, split right/down, Pane focus, divider and keyboard resize, swap, zoom, Tab/Workspace reorder and close all preserve the intended focus. New Tabs are auto-named. A failed shell spawn leaves the previous layout usable.
- Closing a Pane or Tab does not inspect its foreground process. An operation that closes a Workspace asks for confirmation; closing a parent repository Workspace does not close associated worktree Workspaces.
- `Ctrl+Shift+P`, `Ctrl+Shift+T`, `Ctrl+Shift+W`, `Ctrl+Tab`, `Ctrl+Shift+Tab`, `Alt+Shift++`, `Alt+Shift+-`, `Alt+Arrow`, and `Alt+Shift+Arrow` invoke their documented commands without being sent to the shell.
- The current Git branch is shown. Create Worktree and Open Existing Worktree open the expected checkout. Closing either Workspace leaves files intact. Dirty removal is refused; clean managed removal deletes only the checkout and keeps the branch.
- Restarting a non-empty Session restores Workspace/Tab/Pane structure, names, layout, order and cwd using fresh shells. Closing all Workspaces keeps the window open on Start Page, including after restart.
- Closing the application terminates and reaps an identifiable long-running child process; the MVP offers no detach or background survival.

Any crash, deadlock, data loss, shell input/output failure, incorrect delete authority, unreaped process, or structural restore error blocks the MVP. Cosmetic defects may be deferred only when recorded with reproduction steps and they do not obscure terminal content or controls.

## Handoff Evidence

The release candidate must include:

- A locked `Cargo.lock` and the reviewed GPUI/gpui-component revisions.
- Passing Linux command output from the required commit.
- A completed Windows checklist with environment details and screenshots or a short recording of the terminal, layout, worktree, and restart flows.
- Updated `CONTEXT.md` and ADRs when implementation discovers a domain change. Behavioral changes require updating this acceptance plan before merge.
- A known-limitations list matching the exclusions below.

## Accepted MVP Limits

- One native window and one process; application exit terminates all PTYs.
- Structural Session restore only; no terminal history, process, command, Agent conversation, detach, attach, remote access, or background server.
- No automatic agent launch, agent-specific terminal path, hook installer, or agent session resume. Status remains heuristic and unsupported CLIs may stay `unknown`.
- No multi-window Pane undock, Pane Stack, free Pane drag/drop, or cross-Workspace live Pane move.
- Fixed shortcuts; no herdr prefix/navigation/resize modes and no keybinding remap.
- Start Page has no Recent Workspaces list.
- No terminal application mouse reporting, CSI-u/Kitty keyboard, focus/bell/title handling, OSC 8/52, image protocols, search, or copy mode unless a target CLI demonstrates a blocking need.
- Branch display only; no dirty or ahead/behind presentation, bare-repository workflow, forced worktree removal, branch deletion, or configurable worktree root.
- Windows cwd following uses the best ConPTY/shell signal available and falls back to the stable Workspace Root Directory; Unix foreground-process cwd fidelity is not assumed on Windows.
