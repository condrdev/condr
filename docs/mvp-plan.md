# Condr MVP Implementation and Acceptance Plan

## Outcome

Condr MVP is a native GUI client for organizing ordinary shell terminals across one or more connected Servers, Workspaces, Tabs, and split Panes. An agent CLI is an optional process that the user starts inside a Terminal; Condr never selects or launches one automatically.

The GUI discovers an existing local Server or starts one with `condr server run`, the same binary used remotely, then connects through the common protocol. Closing the GUI only disconnects the client: Servers, Sessions, PTYs, and agents continue running. A selected Server with no Workspaces shows a Start Page with `New Workspace…`, which prompts for an absolute Root Directory in that Server's filesystem namespace; a non-empty selected Server shows its Server/Workspace/Agent hierarchy, Tab row, and active Pane layout.

## Architecture Boundary

| Crate | Owns | Must not own |
| --- | --- | --- |
| `condr-core` | Stable domain IDs and split layout, versioned protocol types, PTY and VT components, terminal I/O, agent detection, Git/worktree operations, Session Snapshot schema | GPUI entities, Dock runtime IDs, network listeners, GUI state |
| `condr-server` | Stable Server identity, Session registry, live Terminal runtimes, protocol endpoint, client synchronization, Session Snapshot persistence | GPUI rendering, window focus, local-only behavior |
| `condr` | Connections to one or more Servers, local Server discovery/start, GPUI window and terminal element, input routing, Start Page, sidebar, Tab row, Dock projection | Authoritative domain state, PTY ownership, implicit Server shutdown |

`condr-core` and `condr-server` run Tokio; `condr-gui` uses the GPUI executor. The wire protocol is transport-independent: length-prefixed `bincode + serde` frames, a `Hello`/`Welcome` handshake, strict protocol-version rejection, and bounded frame sizes. Local connections use a private `interprocess` endpoint (Unix domain socket or Windows named pipe); remote MVP connections use an explicitly configured trusted TCP endpoint, normally a Server loopback listener forwarded through an external SSH TCP tunnel. Both paths carry the same protocol and server behavior; the local Server does not expose a public listener, and application authentication/authorization is deferred.

The Server continues consuming PTY output and updating VT state with no clients connected. Reconnecting to a running Server first receives a one-shot authoritative bootstrap (stable Server identity plus runtime epoch, Session/layout state, active selections, focus, cwd, each Pane's live terminal view, foreground shell/Agent identity and status), then subscribes to ordered reliable events and a coalesced terminal visual stream; no shell or Agent is recreated. The identity/epoch pair lets the GUI distinguish a live reconnect from a replacement Server after restart. The server owns the event order and is the only authority for layout mutations. MVP has one active controller per Server/Session; a newly attached controller may supersede the previous one, while collaborative multi-client control is not guaranteed. This connection synchronization is distinct from durable Session Snapshot restore after a Server restart.

## Delivery Phases

Each phase starts only after the preceding exit condition holds.

| Phase | Deliverable | Exit condition |
| --- | --- | --- |
| 0. Build baseline | Cargo workspace, Apache-2.0 metadata, dependencies locked to reviewed revisions | `condr-core` checks on Linux/arm64 and an empty `condr` window builds on Windows; no copied GPL Zed terminal code |
| 1. Core domain | Session, Workspace, Tab, Pane, stable Root Directory, split tree, focus/order, Pane-to-Tab-to-Workspace close cascade, durable snapshot schema | Headless tests prove domain invariants without GPUI or a real shell |
| 2. Server/client foundation | Standalone `condr-server`, stable Server/Session addressing, transport-independent versioned command/event protocol, private local IPC, local discovery/start, trusted TCP/SSH-tunnel configuration, disconnect/reconnect | Headless integration proves the same framed client protocol reaches local IPC and remote-style SSH/TCP endpoints, rejects incompatible versions and oversized frames, preserves Sessions with zero clients, bootstraps authoritative structure plus Server identity/epoch before ordered reliable events and coalesced terminal visual frames on reconnect, distinguishes a live reconnect from a replacement Server, enforces one active controller, and never exposes a non-loopback listener by default |
| 3. Terminal vertical slice | Server-owned `portable-pty` shell runtime, persistent `alacritty_terminal` state, ordered I/O, resize, input encoding, paste, scrollback, GUI-local selection with Server-side copy extraction, terminal synchronization | A real PTY integration test passes on Linux; one Windows shell/Agent Pane survives GUI disconnect/reconnect with its process, layout, and terminal state intact |
| 4. Native orchestration UI | Multi-Server navigation, Start Page, sidebar, Tab row, Dock projection, split/focus/resize/swap/zoom/close, and fixed shortcuts | One GUI controls the complete Workspace/Tab/Pane workflow across local and remote Servers; Dock cannot mutate Server state independently |
| 5. Agent and Git workflows | Foreground agent recognition, bottom-buffer status rules, unseen `done`, branch display, create/open worktree, clean managed removal | State transitions and Git safety rules pass core/server tests and are visible for local and remote Sessions |
| 6. Persistence and failure handling | Debounced atomic Session Snapshot on the Server, Server-restart recovery with fresh shells, partial pruning, reconnect and Start Page fallbacks | Round-trip and corruption tests pass; live reconnect preserves running work, while Server restart restores structure but not processes or terminal history |
| 7. Release gate | Cross-platform checks, Windows-to-Linux remote evidence, documented limitations | Every required check below passes at one commit |

### Phase 4 UI Contract

- `Session` remains a Server-owned domain and protocol boundary; it is not a user-visible navigation item. The sidebar hierarchy is Server → Workspace, with recognized Agents added under their Workspace in Phase 5.
- The sidebar is persistent and resizable. Selecting a Server shows its last active Workspace or Start Page; selecting an Agent activates its Workspace, Tab, and Pane.
- The active Workspace owns the Tab row. Panes have no permanent title bar; a visible focus treatment identifies the active Pane, while Pane commands remain available through shortcuts and context menus.
- The GUI lazily retains one replaceable Dock projection per connected Server/Tab so returning to a previously shown layout does not rebuild its GPUI entity tree. Only the active projection is mounted and participates in layout, prepaint, paint, and Terminal resize; inactive projections retain state only. A Bootstrap prunes removed Tabs, and a changed Server/runtime/Session authority clears that Server's entire projection cache.
- Server, Workspace, and Agent navigation remains on the currently presented Dock until the matching Layout request is applied and an authoritative Bootstrap covers its event sequence. A rejected request, disconnect, or removed Server releases that presentation fence and restores the current Server's authoritative projection in the same GUI update. Cached Dock state is never Session or persistence truth.
- A disconnected Server keeps its last Workspace tree and terminal views visible but read-only, shows connection status and a reconnect action, and disables mutations until control is restored.
- Root Directory and existing-worktree paths belong to the owning Server. A local endpoint uses the native directory picker; a TCP endpoint, including one reached through an SSH tunnel, uses text input for an absolute Server path. The Server validates the path before changing authoritative layout.

## Linux/arm64 Automated Gate

Run:

```bash
cargo fmt --all -- --check
cargo clippy -p condr-core -p condr-server --all-targets -- -D warnings
cargo test -p condr-core -p condr-server
cargo test -p condr-gui --features test-support
```

The GUI test-support suite runs headless with GPUI's `TestPlatform`, but starts a real
Server (`condr server run`) over a local IPC endpoint. It drives the same Root, buttons, keyboard
shortcuts, Session/Layout protocol, PTY, and terminal rendering path as the desktop client;
it does not replace the small Windows ConPTY/window-manager smoke test.

The core/server suite must prove:

- An empty Session is valid. Workspace and Tab creation atomically creates one root Pane; an invalid Root Directory or failed terminal spawn leaves no partial layout.
- Closing the only Pane closes its Tab; closing the last Tab closes its Workspace; closing the last Workspace yields an empty Session without stopping the Server.
- Stable IDs, names, order, active selections, focus history, split direction and ratios survive a durable snapshot round trip. Zoom does not.
- A Workspace Root Directory remains stable after Pane cwd changes. New Tabs follow the focused Pane cwd, Splits follow the target Pane cwd, and both fall back to the Root Directory.
- Protocol negotiation sends `Hello` first, rejects incompatible versions and oversized frames with a clear error, and uses bounded length-prefixed `bincode + serde` messages. Commands and events identify Server and Session explicitly. Layout requests carry a client request ID; success identifies the reliable event sequence that must be covered by Bootstrap, while rejection is typed and correlated to the same request. A rejected Snapshot request identifies the authoritative Server/Session so the GUI retargets its single in-flight resync instead of remaining permanently unsynchronized. A one-frame Bootstrap header carries Server identity, runtime epoch, and structural Snapshot; Terminal, Agent, Git, and zoom records are bounded to 32 MiB each and 64 MiB in aggregate, reliably chunked when needed, validated and assembled completely, then applied atomically. Bootstrap VT materialization and encoding happen outside the Session lock, while a per-client reliable fence orders concurrent events after the complete batch. A rejected event cursor uses a typed response; the GUI disables mutations, requests one authoritative Bootstrap, and resubscribes instead of remaining connected without events. Ordinary layout or visual Bootstrap recovery preserves the current subscriber and its installed terminal baseline; changed Server/runtime/Session authority reacquires control. Local IPC and remote-style connections exercise the same behavior.
- The default Server endpoint is private/local-only with owner-only permissions and stale-endpoint cleanup that never removes a live listener. Disconnecting the final client keeps the Server, Sessions, PTYs, and Agents alive; reconnect returns the complete authoritative bootstrap and live Pane views, then ordered reliable events and coalesced terminal visual frames, including an Agent already running inside a Pane. Reconnect attempts are generation-scoped and detach superseded connections so a late result cannot retain control. Only one active controller owns input and resize for a Server/Session.
- A real PTY starts an ordinary shell in the requested cwd. Reads are single-source, writes remain ordered and bounded, EOF and child exit are reported once, and explicit Server shutdown keeps reading through bounded process-tree escalation before draining the final tail. It reaps the complete Terminal process tree without waiting forever on a retained slave or blocked PTY writer, and validates process birth identity before cwd/Agent inspection or signaling.
- One persistent VT state consumes fragmented output while no client is connected. Terminal replies use FIFO capacity reserved from bounded user input; resize uses an independent latest-wins control so input backpressure cannot delay the final size, and each applied size updates the PTY dimensions and VT grid together. A failed resize worker rejects later requests instead of leaving false pending state. PTY backpressure cannot hold the Server state lock or block shutdown. An authoritative Bootstrap or new connection generation releases Client-side pending resize suppression so a replaced runtime can be resized again.
- Reconnection sends the complete Session/layout and terminal views before reliable incremental events and per-client terminal deltas; a running Agent is visible in its original Pane without a new shell. A live Pane frame larger than one protocol frame is chunked under a 32 MiB record limit, and extracted Terminal cell text is capped at 256 UTF-8 bytes. Legacy and application-cursor key encoding, bracketed paste, main-screen scrollback, client-local simple selection with range-based Server copy extraction, UTF-8, and wide-cell behavior are covered.
- Output coalescing cannot lose the final terminal update notification.
- Agent detection distinguishes `unknown`, `idle`, `working`, and `blocked`; presentation derives `done` only from `idle + unseen`, and viewing the Pane clears `done`.
- Non-Git directories remain valid. Git discovery derives only metadata. Tests using temporary repositories cover branch display, new and existing branch worktree creation, opening an existing worktree, and explicit parent association.
- `Close Workspace` never invokes Git or deletes a checkout. Only a Condr-created Managed Worktree can be removed; dirty or untracked files refuse removal; clean removal leaves the branch. Restoring a saved worktree association revalidates the exact parent and child checkout roots against their current Git topology before retaining it; a stale association is cleared and cannot grant deletion authority. If removal fails after live terminals are stopped, fresh terminal instances are restored in the unchanged Workspace; a Pane whose replacement shell cannot start remains coherently exited with its final view. Prepared-checkout rollback failures are reported.
- Durable Snapshot persistence uses atomic replacement. The bounded flat codec rejects payloads over 8 MiB, while the Server enforces a frame-safe durable budget of 2 MiB minus 64 KiB because the structural Snapshot occupies one Bootstrap header. Missing, empty, corrupt, over-budget, or wholly unrestorable snapshots yield Start Page state. Server-restart recovery tries a Pane's saved cwd, falls back to its stable Workspace Root Directory when that saved cwd is unusable, persists the repair, and prunes only when neither can start a fresh shell. Restorable siblings survive. Recovery never restores grid, scrollback, commands, live processes, Agent status, or conversations. The unreleased format is replaced in place without a legacy migration or compatibility path.

## Windows Manual Gate

Record the Windows version, commit, Rust toolchain, shell, GPU, Server endpoint, and agent CLI used for the representative smoke test. Build and run with the MSVC toolchain, then verify:

```powershell
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
cargo run -p condr-gui
```

- First launch discovers or starts one detached local Server, shows Start Page, and starts no shell. `New Workspace…` uses the native directory picker for Local and a Server-path text field for TCP/SSH-tunnel connections, then opens a shell in the validated absolute Root Directory. It does not launch an agent CLI.
- PowerShell or the configured shell accepts typing, Enter, Backspace, navigation keys, Ctrl+C, ANSI color, bracketed paste, UTF-8/wide text, long scrolling output, selection, and clipboard copy.
- Rapid window and divider resizing updates the shell's reported rows/columns without freezing, blanking, or leaving stale regions.
- Start an identifiable long-running command and a representative Agent, close the GUI, and verify the Server, shell, and Agent remain alive. Reopen the GUI and verify it reconnects to the same Server/Session, restores the complete layout, and shows the Agent in its original Pane with current terminal content; only explicit Server stop terminates and reaps the child.
- Connect the same GUI to the local Windows Server and a Linux loopback Server through an external SSH TCP tunnel. Both appear in the Server hierarchy and expose the same Workspace, Tab, Pane, terminal, agent, and Git commands.
- One representative agent CLI can be started manually and used interactively without product-specific launch code. Its recognized states appear in the sidebar; clicking it activates its Server, Session, Workspace, Tab, and Pane.
- New Tab, split right/down, Pane focus, divider and keyboard resize, swap, zoom, Tab/Workspace reorder, and close all preserve the intended focus. New Tabs are auto-named. A failed shell spawn leaves the previous layout usable.
- Closing a Pane or Tab does not inspect its foreground process. An operation that closes a Workspace asks for confirmation; closing a parent repository Workspace does not close associated worktree Workspaces.
- `Ctrl+Shift+T`, `Ctrl+Shift+W`, `Ctrl+Tab`, `Ctrl+Shift+Tab`, `Alt+Shift++`, `Alt+Shift+-`, `Alt+Arrow`, and `Alt+Shift+Arrow` invoke their documented commands without being sent to the shell.
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
- Local and remote Servers have identical product behavior and wire semantics. The MVP remote path uses an explicitly trusted TCP endpoint, normally through an external SSH TCP tunnel; application authentication, authorization, encryption, account management, and public Internet exposure are deferred. Transport protection depends on the external tunnel. Servers use private local endpoints by default.
- The GUI accepts remote Servers as socket addresses. SSH tunneling is configured outside Condr, and additional Server entries are not persisted across GUI process restarts in the MVP.
- GUI reconnect preserves the authoritative Session/layout plus live processes and terminal state held by a running Server, including a running Agent and its visible Pane. Server restart performs structural Session restore with fresh shells and does not preserve terminal history, process, command, Agent conversation, or live Agent state.
- No automatic agent launch, agent-specific terminal path, hook installer, or agent session resume. Status remains heuristic and unsupported CLIs may stay `unknown`.
- No multi-window Pane undock, Pane Stack, free Pane drag/drop, or cross-Workspace live Pane move.
- Fixed shortcuts; no herdr prefix/navigation/resize modes and no keybinding remap.
- Start Page has no Recent Workspaces list.
- No terminal application mouse reporting, CSI-u/Kitty keyboard, focus/bell/title handling, OSC 8/52, image protocols, search, or copy mode unless a target CLI demonstrates a blocking need.
- Branch display only; no dirty or ahead/behind presentation, bare-repository workflow, forced worktree removal, branch deletion, or configurable worktree root.
- Cross-platform `bincode` encoding of `PathBuf` requires UTF-8. A non-UTF-8 Unix cwd observation is rejected and the last durable Pane cwd is retained.
- Linux Bash reports its cwd on normal shell exit so an immediate `cd DIR; exit` is durable. Other Unix shells, a shell replaced with `exec`, and forced `SIGKILL` retain the last cwd observed from the foreground process or an OSC report; no polling interval is treated as an exit-tail guarantee.
- Windows cwd following uses the best ConPTY/shell signal available and retains the last-known Pane cwd, initialized from that Terminal's startup directory. Session layout inheritance falls back to the stable Workspace Root Directory only when no Pane cwd is available; Unix foreground-process cwd fidelity is not assumed on Windows.
