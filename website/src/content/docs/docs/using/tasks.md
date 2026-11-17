---
title: Tasks
description: See every Agent on every Device in one view, answer them without leaving it, and start Agents on work you wrote down.
---

When several projects run at once, switching between them one Workspace at a time costs attention. **Tasks** mode puts every Agent on every connected Device in one window, sorted by what it needs from you, and opens any one of them beside the rest so you can answer it and move on.

## Tasks mode

Open it with the **Tasks** row at the top of the sidebar, or its icon at the top of the collapsed sidebar; it takes the place of the Workspace, beside the sidebar, as the Ports page does. The ✕ at the right of its toolbar takes you back to the Tab you were looking at, and choosing a Workspace in the sidebar goes there. In the sidebar, choosing an Agent selects its card. Only the main window has Tasks mode.

Agents sit in columns by state: **To do** (Tasks not started yet), **Needs you** (a bell, Blocked or Error), **Done** (finished and not looked at yet), **Working** and **Idle**. You never drag cards between them; they move as the Agents report. A column's heading folds it to a narrow strip, and Needs you unfolds by itself when an Agent arrives in it.

- **Board or Table.** **Layout**, at the right of the toolbar above the cards, switches between them. The Board shows full cards, with the Workspace, branch, Device, changes and what the Agent waits for, or, with **Compact cards** on, one row per Agent; the Table has one row per Agent.
- **Scope.** The breadcrumb at the left of the toolbar, `MacBook › api-server`, narrows the view to one Device or one Workspace. A repository's scope includes the worktrees made from it.
- **Disconnected Devices.** Their Agents' cards are hidden; the Device's mark in the sidebar says why.

### Answering an Agent

Click a card to open its terminal beside the cards, below the Board or to the right of the Table, and type: the keyboard goes straight to the Agent. `Cmd+]` and `Cmd+[` (`Alt+]` and `Alt+[`) step to the next and previous Agent, so you can answer one after another without the mouse. Click the card again, or ✕, to close the terminal.

Click the cards' background to take the keyboard back: the arrow keys then preview each Agent without typing into it or resizing it, and `Enter` hands the keyboard over. **Open in Workspace** leaves Tasks mode for the Agent's own Tab, where you can split or rearrange Panes. Layout and Tab shortcuts do nothing in Tasks mode.

## Tasks

A **Task** is work you write down before any Agent starts on it. Tasks are kept on each Device's Server, so every window and the `condr task` command see the same ones.

- **Write one** with **+ New task** at the top of To do, or **New Task** in the Command Palette. Give it a title, a plain-text description, the Device it belongs to and, optionally, the repository. Nothing is kept until you **Save**.
- **Start it** from the right half of its form. Choose where: the repository's own Workspace, a **New worktree** with its own branch, or, for a Task worked on before, where it left off. Choose the Agent and press **Start**. Condr opens the Workspace, starts the Agent there and asks it to read the Task, its title and description, from a file the Server writes.
- **While it runs**, every Agent in its Workspace carries the Task's title on its card. If its Workspace has no Agent left, the Task shows in Idle as "No agent running", ready to start again.
- **Archive it** when the work is done or dropped. For a started Task, Archive asks whether to close its Workspace, ending its Agents, and whether to remove its worktree; the branch always stays. Archived Tasks wait at the bottom of To do, where you can restore or delete them.

Each Workspace holds one Task at a time. Hooks are not required to start an Agent; without them its conversation does not resume after a Server restart.

The same Tasks from the command line: [`condr task`](/docs/reference/cli/#task).
