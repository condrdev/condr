---
title: Why Condr
description: Learn the thinking behind Condr, and why we chose an extremely light native architecture to rebuild the Agent development workflow across devices and servers.
---

When you do heavy AI-assisted programming across several machines and environments (a local laptop, a remote GPU dev box, a cloud build machine), the existing toolchains tend to leave you stuck between poor choices:

- **Heavy Remote IDEs**: launching remote VS Code or JetBrains Gateway easily eats hundreds of megabytes of memory and plenty of CPU, the interface freezes at the slightest network jitter, and the setup cost is high.
- **Raw terminal workflows (SSH + tmux)**: light and stable enough, but with no intuitive code review views (Diff / Preview), no concurrent collaboration from several devices, and no unified way to manage the states of Agents spread across machines.
- **AI locked inside a Webview**: most tools force the Agent into an IDE chat box, losing the flexibility of the command-line ecosystem without shedding the resource burden of a graphical IDE.

Condr exists to end that split: **it is an extremely light, millisecond-startup, modern workbench built for Agent collaboration across devices.**

---

## Core strengths and design

### Lightweight and fast

Condr drops Electron and heavy full-featured remote development environments in favor of a pure Rust stack:

- **Very low system resource usage**: both the `condr-server` server and the `condr-gui` client compile to compact native machine code with a tiny resident memory footprint, so the server carries almost no extra load.
- **Millisecond cold start and 60 Hz rendering**: built on a pure Rust virtual terminal core, it keeps a steady frame rate while an Agent streams output, redraws the whole screen or scrolls through a very long scrollback buffer, with no sluggish interaction.
- **Connect instantly**: no bulky language servers or daemon clusters to bring up on the remote side; pairing finishes and the terminal is ready within seconds.

### Modern remote connectivity

Condr treats "connect seamlessly to any machine, from anywhere" as a first-class capability:

- **Zero-configuration NAT traversal (direct P2P)**: based on the Iroh protocol with Relay-assisted hole punching, so a direct peer-to-peer connection can be established even when both ends sit behind complex internal networks or NAT, without a public IP or port forwarding.
- **Bank-grade secure channel**: built-in mutual encrypted authentication based on the Noise protocol, combined with one-time pairing codes and a per-device Ed25519 key, so there are no certificates or key distribution to maintain.
- **Every channel supported**: full support for native SSH socket forwarding and for local Unix Domain Sockets / named pipes, fitting into existing operations setups.
- **Independent views across devices**: when multiple devices connect to the same remote Server, each browses its own Workspaces and Tabs and stays fully usable; terminals automatically resize to match the window you last clicked into.

### Server-owned runtime

Say goodbye to the fragile experience of "close the window, lose the connection; drop the network, lose the session":

- **Processes fully decoupled from windows**: every terminal session, Agent process and long-running task is hosted by the resident `condr-server`. Closing the desktop app only closes the projected view; background tasks never stop.
- **Smooth topology recovery**: the Server persists the layout snapshot in real time. Even if the daemon restarts, it rebuilds the Workspace split structure automatically and reconnects the existing contexts.

### Terminal-first, keeping 100% of the native Agent ecosystem

Condr does not build a second-hand chat interface, take over the conversation loop or send API requests on your behalf:

- **Native subprocess passthrough**: CLIs such as Claude Code, Codex and OpenCode run exactly as they are in a real system PTY.
- **Full ecosystem support**: each vendor's native context compaction, local configuration, permission prompts and rich interactions are kept intact.
- **Review panels out of the box**: review every code change an Agent produces through the light Diff Tab and Preview Tab, without switching to a full IDE.

---

## How common approaches compare

| Dimension | Heavy remote IDE (such as VS Code Remote) | Classic terminal multiplexer (SSH + tmux) | Condr |
| :--- | :--- | :--- | :--- |
| **Performance and resource usage** | Hundreds of MB of resident memory on the remote side, prone to stutter | Very low (plain text terminal) | **Very low (native Rust architecture, tiny memory footprint)** |
| **NAT traversal and networking** | Relies on public exposure, reverse proxies or tunneling tools | Requires your own SSH jump host or VPN | **Built-in P2P hole punching, zero-configuration connectivity across complex internal networks** |
| **Session persistence** | Depends on remote server state; occasional glitches after a drop | Depends on the tmux daemon; terminal-only recovery | **Resident server daemon, restores windows and split layout** |
| **Multi-device collaboration** | Multiple users share a single cursor and view | Shared terminal screen where actions can conflict | **Each client has an independent view, with terminal dimensions adapting to the active window** |
| **Code review efficiency** | Full-featured but heavy to launch | Typing `git diff` by hand, no graphical comparison | **Built-in light syntax-highlighted Diff and Preview views** |
| **Agent CLI nativeness** | Mostly steered toward sidebar extensions, cutting the CLI apart | 100% native, but no visual orchestration | **100% native, plus intuitive state display and review** |

---

## Next steps

- [Getting started](/docs/start/getting-started/): install and set up your first remote connection in a few minutes.
- [Remote connections](/docs/using/remote/): learn how to set up SSH, Noise TCP and P2P NAT traversal in detail.
- [Core concepts](/docs/start/concepts/): understand the entity hierarchy of Device, Server, Workspace and Pane in depth.
