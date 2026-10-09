---
title: Core concepts
description: Learn Condr's entity hierarchy (Device, Server, Workspace, Tab, Pane, Agent) and its core design philosophy.
---

Condr is a unified workbench that is **terminal-centric and built for multi-Agent collaboration**. Unlike approaches that put the model in a Webview chat box, Condr runs each vendor's native Agent CLI in a real terminal environment, keeping the native experience while adding cross-device, multi-branch orchestration and state monitoring.

---

## Core design principles

- **Terminal first, native execution**: no rebuilt chat UI and no takeover of the conversation loop. Agents such as Claude Code and Codex run as native subprocesses in a PTY, keeping their context compaction, local configuration and tool ecosystem intact.
- **Runtime decoupled from view**: the server (`condr-server`) hosts the terminal processes, Agent states and layout snapshot, and is the single source of truth; a client window is only a projection of it. Closing a window does not affect background tasks, and several clients can connect at the same time without interfering with each other.
- **Deterministic state awareness**: no fragile regex scraping of screen text. Agents report precise states and the reason they are waiting only through their official lifecycle Hooks, using an in-band control sequence (OSC 777).

---

## Entity hierarchy

Condr's resources and interface are organized in a strict tree:

```text
Device (physical or virtual machine, holds a unique Ed25519 key)
└── Server (the single resident background service on the machine, owns the runtime)
    └── Session (the Workspace topology and split layout snapshot the server maintains)
        ├── Workspace 1 (main Workspace: bound to a root directory such as ~/code/my-app)
        │   ├── Tab 1 (terminal Tab: holds split Panes running Agents or plain shells)
        │   ├── Tab 2 Diff (code review: view one file's diff against HEAD or the base branch)
        │   └── Tab 3 Preview (source preview: browse a file with syntax highlighting)
        │
        └── Workspace 2 (managed worktree: an isolated branch environment based on Git Worktree)
            └── Tab 1 (terminal Tab)
                └── Pane 1 (an Agent running another task)
```

| Entity | Responsibility and lifecycle |
| :--- | :--- |
| **Device / Server** | The abstraction of a machine running Condr. Each device holds a unique cryptographic identity (Device Key); closing the client does not affect the Server. |
| **Session** | The layout topology the server maintains. Layout changes are written to the snapshot in real time, and after a Server restart the structure and Agent session contexts are restored automatically. |
| **Workspace** | A unit of project work, anchored to a unique absolute **root directory**. An isolated **Managed Worktree** Workspace can be derived from it using Git Worktree. |
| **Tab** | A page inside a Workspace. Either a **terminal Tab** (holds split Panes, switchable with the `1~9` shortcuts) or a **viewer Tab** (read-only Diff / Preview review pages). |
| **Pane / Terminal** | A leaf of the layout and the virtual terminal engine (based on the Rust `alacritty_terminal`). Supports horizontal and vertical splits, zoom, and safe cascading exit of the process tree. |
| **Agent** | A recognized command-line program running inside a Pane. Its state is synced to the sidebar in real time, including the reason it is blocked. |

---

## Key mechanisms

### Session snapshot and restart recovery
Keep the **Condr Session** (the window and split topology the server maintains) apart from the **Agent Conversation** (the conversation context the Agent itself stores on disk):
- **What the snapshot records**: the Server continuously saves the Workspace tree, the split ratios of the Panes and the native session IDs of the running Agents.
- **Seamless continuation**: after a Server restart, Condr rebuilds the splits and restores the working directories; for Agents that support Hooks, it runs the resume command automatically to reconnect their context. *(Note: the terminal's past screen output and process memory state are not saved.)*

### Managed Worktree (isolated branches)
When several Agents change the same project at once, code and Git conflicts are almost inevitable:
- Condr can create a **Managed Worktree** for a repository, checking out a new branch in a separate `<repo>.worktrees/<branch>` directory and opening it as its own Workspace.
- Removing that Workspace requires a clean state, cleans up only the separate worktree directory, and keeps the Git branch and its commits.

### Independent Client views and multi-device collaboration
- **Independent views**: each connected Client keeps its own record of which Workspace and Tab it is viewing. When several devices connect to the same Server, switching views on one does not affect the others.
- **Simultaneous access**: every connected device can type, scroll, select and rearrange the layout without locking others out. A terminal keeps a single shared size and matches whichever window you last clicked into; other windows show the same terminal from its top-left corner until you click into them.

### Agent state indicators

| State | Meaning |
| :--- | :--- |
| **Working** | Running a task (thinking, generating text or calling tools). |
| **Blocked** | Waiting for your input or approval (listed in Needs you). |
| **Done** | Finished in the background. Resets to Idle once focus returns to its Pane. |
| **Idle** | Idle: the turn is over and it waits for the next instruction. |
| **Unknown** | A plain shell session, or an Agent without Condr hooks. |
| **Bell** | The terminal rang its bell (Bell takes priority over the regular lifecycle states). |

---

## Quick links

- [Workspaces, Tabs and Panes](/docs/using/workspaces/): splitting Panes, shortcuts and using Managed Worktrees.
- [Agent integrations](/docs/using/agents/): the list of supported Agents and how state reporting is configured.
- [Changes and file preview](/docs/using/changes/): reviewing files with the Diff Tab and Preview Tab.
- [Remote connections](/docs/using/remote/): setting up SSH, TCP LAN pairing or direct P2P connections through NAT.
