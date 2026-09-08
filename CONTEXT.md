# Condr

Condr organizes terminal-first work across projects while recognizing agent CLI processes as optional occupants of terminals.

## Language

**Server**:
A long-lived Condr runtime that owns one or more Sessions and their Terminals. Each machine runs one Server: it always answers on a private local socket for the GUI and CLI on that machine, and on a configured TCP address as well for other Devices, as ADR 0013 describes. Closing a Client does not stop it or its work. Over TCP a Server accepts paired Devices (ADR 0011); SSH Clients access it with the remote login user’s permissions (ADR 0015).

**Device**:
A Client installation on another machine, identified by its own persistent static key stored beside its `config.toml`. A Device is paired once through an Invite and listed in the Server's `authorized-clients` until revoked; local and SSH Clients need no TCP pairing.

**Invite**:
A one-time secret that `condr server invite` creates for ten minutes. Pasted into Add Server as `tcp://<server key>.<invite>@host:port`, it lets one unknown Device complete the handshake and become authorized; it is never stored by the Client.
_Avoid_: Local backend, GUI runtime

**Client**:
The GUI or CLI connecting to a Server to inspect or change its Sessions. The native GUI can present several Servers; neither Client owns Terminal processes.
_Avoid_: Server, runtime owner

**Session**:
A Server-owned working arrangement containing zero or more Workspaces and the current selection. It is not an agent conversation.
_Avoid_: Agent session, conversation

**Start Page**:
The window surface shown when a Session contains no Workspaces. It offers an entry point to choose a Root Directory and create a Workspace, but is not itself a Workspace.
_Avoid_: Empty Workspace, empty Pane

**Settings**:
A GUI-owned window for this Client's appearance and Terminal preferences, each connected Server's shell preference, and bundled license notices. Client preferences apply to all its Panes; a Server preference belongs to that Server and is shared by its Clients.
_Avoid_: Preferences, options, config

**Appearance**:
The Settings page owning how the GUI chrome looks, and the `System` / `Light` / `Dark` preference on it, labeled Theme in the UI. `System` follows the operating system's appearance for as long as it stays selected. It also owns the Terminal font family, size and Color Scheme, which apply to every Pane on every connected Server.
_Avoid_: dark mode, mode

**Color Scheme**:
A named Terminal palette (background, foreground, cursor, selection and the 16 ANSI colors) chosen on the Appearance page. The built-in collection is vendored from iTerm2-Color-Schemes in Alacritty's TOML layout; `Default` is Condr's own dark palette. It is independent of the `System` / `Light` / `Dark` mode.
_Avoid_: Terminal theme, palette (as a user-facing name)

**Workspace**:
A project- or task-level container with a stable identity and Root Directory. It owns an ordered set of Tabs. Closing it terminates its terminals but never removes files or a Git worktree checkout.
_Avoid_: Project, space

**Root Directory**:
An absolute directory on the owning Server, selected when a Workspace is created or opened. It is the fallback cwd for new terminals and remains stable when a shell changes directory. A local Client can choose it with the native directory picker; a Client connected through TCP or SSH enters the path in the Server's filesystem namespace.
_Avoid_: Current directory, identity cwd

**Tab**:
A terminal layout within a Workspace, optionally named by the user. It owns an arrangement of Panes and its current focus. The GUI numbers open Tabs from 1 in their current Workspace order, recalculating after reordering or closing. An unnamed Tab displays only its centered number; a named Tab displays the number followed by its user-provided name. The number is derived presentation, not a stable identity or a persisted name.

`Alt+1` through `Alt+9` (`Cmd+1` through `Cmd+9` on macOS) activate the corresponding displayed Tab number in the current Workspace. A missing number does nothing, and 9 means the ninth Tab, not the last. These GUI shortcuts are consumed before terminal input and do not switch Tabs while a modal dialog is open.

**Pane**:
A terminal location and layout leaf within a Tab. It may show a shell or an Agent running inside that shell.
_Avoid_: Agent

**Terminal**:
The interactive command-line environment presented by a Pane. A Terminal remains useful whether or not it currently contains an Agent.

**Agent**:
A recognized agent CLI process running inside a Terminal. It does not own or create the Pane that presents it. Its state (`Unknown`, `Idle`, `Working`, `Blocked`) comes only from hooks Condr installed into that CLI, delivered in-band as OSC 777 (ADR 0014); an Agent that has not reported is `Unknown`, never guessed.
_Avoid_: Pane

**Agent Conversation**:
A conversation owned and stored by a native Agent CLI, identified by that CLI’s session ID. A Pane retains a reference to its running Agent Conversation so a replacement Server can reopen it.
_Avoid_: Session, process snapshot

**Managed Worktree**:
A linked Git worktree created by Condr and explicitly associated with its parent repository Workspace. Only a Managed Worktree is eligible for the separate Remove Worktree action; opening an existing directory never grants deletion authority. Removal requires a clean checkout and leaves its Git branch intact.
_Avoid_: Git Workspace, any detected worktree

**Session Snapshot**:
A durable description of Session structure and references to Agent Conversations, including an empty Session, used to rebuild the arrangement after a Server restart. It contains neither terminal history nor live processes nor conversation contents; those contents remain owned by the native CLI.
_Avoid_: Backup, process snapshot
