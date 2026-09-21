# Condr

Condr organizes terminal-first work across projects while recognizing agent CLI processes as optional occupants of terminals.

## Language

**Server**:
A long-lived Condr runtime that owns one or more Sessions and their Terminals. Each machine runs one Server: it always answers on a private local socket for the GUI and CLI on that machine, and on a configured TCP address as well for other Devices, as ADR 0013 describes. Closing a Client does not stop it or its work. Over TCP a Server accepts paired Devices (ADR 0011); SSH Clients access it with the remote login user’s permissions (ADR 0015). Server is an engineering term: the interface never shows it, and presents a Server together with the machine it runs on as one Device.

**Device**:
A machine running Condr, identified by its one persistent key stored beside its `config.toml` (ADR 0025). That key is the machine's identity whether its Server is being connected to or its GUI or CLI connects out, so a Device has one fingerprint and appears once in any authorized list. Device is the interface's name for a whole machine, its Server included, so people learn one concept for both what they connect to and what connects: the sidebar lists Devices, and Connect Remote Device adds one. A Device reaching a Server over TCP is paired once through an Invite and listed in that Server's `authorized-clients` until revoked; local and SSH Clients need no TCP pairing.

**Invite**:
A one-time secret that `condr server invite` creates for ten minutes. Pasted into Connect Remote Device together with the Server's Device key, as `tcp://<id>.<invite>@host:port`, it lets one unknown Device complete the handshake and become authorized; it is never stored by the Client.
_Avoid_: Local backend, GUI runtime

**Client**:
The GUI or CLI connecting to a Server to inspect or change its Sessions. The native GUI can present several Servers; the CLI reaches one per call, its own Pane's Server by default or a saved Device named with `--device` (ADR 0022); neither Client owns Terminal processes.
_Avoid_: Server, runtime owner

**Session**:
A Server-owned working arrangement containing zero or more Workspaces, their Tabs and Pane layouts, including Pane focus. Each Client owns its View (ADR 0021). It is not an agent conversation.
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

**View**:
Which Workspace and which Tab one Client shows. It is that Client's own state, never part of the Session or its Snapshot (ADR 0021): two Clients on one Server look at different things, and selecting a Workspace or Tab in the GUI sends nothing. `ActivateWorkspace` and `ActivateTab` are requests to every Client to show a target, arriving as the `Activated` event; the CLI's `focus` commands and `--focus` flags send them. The GUI remembers its View, its window and its sidebars across runs in its own state file (ADR 0023).
_Avoid_: Active Workspace, active Tab, selection

**Tab**:
A Workspace's view, optionally named by the user: either a terminal layout that owns an arrangement of Panes and its current focus, or a viewer with no Panes at all (the Diff Tab, ADR 0017, and the Preview Tab, ADR 0018; at most one of each per Workspace). The GUI numbers open Tabs from 1 in their current Workspace order, recalculating after reordering or closing. An unnamed Tab displays only its centered number; a named Tab displays the number followed by its user-provided name. The number is derived presentation, not a stable identity or a persisted name.

`Alt+1` through `Alt+9` (`Cmd+1` through `Cmd+9` on macOS) activate the corresponding displayed Tab number in the current Workspace. A missing number does nothing, and 9 means the ninth Tab, not the last. These GUI shortcuts are consumed before terminal input and do not switch Tabs while a modal dialog is open.

**Pane**:
A terminal location and layout leaf within a terminal Tab. It may show a shell or an Agent running inside that shell.
_Avoid_: Agent

**Diff Tab**:
The one viewer Tab a Workspace may have, showing one file's working-tree changes against `HEAD` as the Server computes them (ADR 0017). Clicking a file in the Changes sidebar creates it or retargets it; it is Session state like every Tab and has no Panes, so Pane commands do not apply to it. Its header's "Show File" opens the same file in the Preview Tab at the line under the diff cursor. Its name starts as "Diff" and is display only.

**Preview Tab**:
The one viewer Tab a Workspace may have for a file's content as it is on disk, read by the Server under the Root Directory (ADR 0018). Clicking a file in the Files view creates it or retargets it; it sits beside the Diff Tab and behaves like it. Its name starts as "Preview" and is display only.

**Changes**:
One of the two views of the right sidebar: the presented Workspace's working-tree changes against `HEAD`, index and worktree folded into one status per path, read-only (ADR 0017). The default view for a Workspace inside a repository.

**Files**:
The other view of the right sidebar: the presented Workspace's directory tree under its Root Directory, one level per Server answer, unfolded by the user, read-only (ADR 0018). `.git` is hidden, dotfiles are shown, ignored entries are dimmed but listed, rows carry JetBrains file-type icons, a changed file's name takes its status colour, and a directory holding a change carries a dot. The default view for a Workspace outside a repository. The Server's watcher refreshes it through `WorkspaceFilesChanged`. A file row's context menu copies its path, opens it in the "Open in" editor, or inserts its path into the Workspace's terminal.

**Terminal**:
The interactive command-line environment presented by a Pane. A Terminal remains useful whether or not it currently contains an Agent.

**Agent**:
A recognized agent CLI process running inside a Terminal. It does not own or create the Pane that presents it. Its state (`Unknown`, `Idle`, `Working`, `Blocked`) comes only from hooks Condr installed into that CLI, delivered in-band as OSC 777 (ADR 0014); an Agent that has not reported is `Unknown`, never guessed. A `Blocked` Agent may carry `blocked_on`, what its hook said it waits for (a tool and command, or a question), cleared with the state (ADR 0024).
_Avoid_: Pane

**Agent Conversation**:
A conversation owned and stored by a native Agent CLI, identified by that CLI’s session ID. A Pane retains a reference to its running Agent Conversation so a replacement Server can reopen it.
_Avoid_: Session, process snapshot

**Managed Worktree**:
A linked Git worktree created by Condr and explicitly associated with its parent repository Workspace. Only a Managed Worktree is eligible for the separate Remove Worktree action; opening an existing directory never grants deletion authority. Removal requires a clean checkout and leaves its Git branch intact. It lives in `<repo>.worktrees/<branch>` beside the repository, or where `[server] worktree_root` says: absolute or `~` paths group by repository, relative paths resolve against the repository (ADR 0002).
_Avoid_: Git Workspace, any detected worktree

**Session Snapshot**:
A durable description of Session structure and references to Agent Conversations, including an empty Session, used to rebuild the arrangement after a Server restart. It contains neither terminal history nor live processes nor conversation contents; those contents remain owned by the native CLI.
_Avoid_: Backup, process snapshot
