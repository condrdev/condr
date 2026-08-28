# Murmur

Murmur organizes terminal-first work across projects while recognizing agent CLI processes as optional occupants of terminals.

## Language

**Server**:
A long-lived Murmur runtime that owns one or more Sessions and their Terminals. A local Server is the same Server started on the GUI machine; closing a Client does not stop it or its work.
_Avoid_: Local backend, GUI runtime

**Client**:
The native GUI that connects to one or more Servers and presents their Sessions. It does not own Terminal processes.
_Avoid_: Server, runtime owner

**Session**:
A Server-owned working arrangement containing zero or more Workspaces and the current selection. It is not an agent conversation.
_Avoid_: Agent session, conversation

**Start Page**:
The window surface shown when a Session contains no Workspaces. It offers an entry point to choose a Root Directory and create a Workspace, but is not itself a Workspace.
_Avoid_: Empty Workspace, empty Pane

**Workspace**:
A project- or task-level container with a stable identity and Root Directory. It owns an ordered set of Tabs. Closing it terminates its terminals but never removes files or a Git worktree checkout.
_Avoid_: Project, space

**Root Directory**:
The directory selected when a Workspace is created or opened. It is the fallback cwd for new terminals and remains stable when a shell changes directory.
_Avoid_: Current directory, identity cwd

**Tab**:
A named terminal layout within a Workspace. It owns an arrangement of Panes and its current focus.

**Pane**:
A terminal location and layout leaf within a Tab. It may show a shell or an Agent running inside that shell.
_Avoid_: Agent

**Terminal**:
The interactive command-line environment presented by a Pane. A Terminal remains useful whether or not it currently contains an Agent.

**Agent**:
A recognized agent CLI process running inside a Terminal. It does not own or create the Pane that presents it.
_Avoid_: Pane

**Managed Worktree**:
A linked Git worktree created by Murmur and explicitly associated with its parent repository Workspace. Only a Managed Worktree is eligible for the separate Remove Worktree action; opening an existing directory never grants deletion authority. Removal requires a clean checkout and leaves its Git branch intact.
_Avoid_: Git Workspace, any detected worktree

**Session Snapshot**:
A durable description of Session structure, including an empty Session, used to rebuild the arrangement after a Server restart. It does not represent terminal history, live processes, or agent conversations; reconnecting a Client to a running Server instead receives the live Session layout and Pane views, including Agents that are still running.
_Avoid_: Backup, process snapshot
