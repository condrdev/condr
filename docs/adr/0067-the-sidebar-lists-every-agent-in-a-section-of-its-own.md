# The Sidebar Lists Every Agent in a Section of Its Own

Status: accepted 2026-11-16.

An Agent's row sits under its Workspace in the left sidebar, and a Workspace is unfolded only while it is the one shown, so going from one Agent to another meant unfolding Workspaces to find it. "Needs you" (ADR 0024, ADR 0044) lists only `Blocked` and `Error`, and the Command Palette's Go to (ADR 0064) is a search, not something that stays on screen. Switching among the Agents that are working, and the ones that just finished, had nothing of its own.

## Prior art

- herdr and herdr-app (Apache-2.0) recognize an Agent in a Pane, as Condr does. The sidebar stacks a Spaces tree above an Agents list, with a divider between them; the list sorts by priority (blocked, then done, then working, then idle) or groups by space.
- T3 Code makes the sidebar itself a list of threads, ordered by when each came back to its person; grouping by project is an option.
- Superset (now Paradev) keeps a project tree and puts its Agents view outside it: a ⌘; switcher and a separate page.
- paseo lists Workspaces only, grouped by project or by status.

In all four, choosing an Agent goes to its Workspace rather than opening it in place. Finished counts as needing its person only until that person has seen it, which Condr already does (ADR 0052).

## Decision

The left sidebar gains an **Agents section** at its bottom, the way VS Code stacks Outline and Timeline under the Explorer. The Workspace tree above it, "Needs you" and the Agent rows under each Workspace stay as they are. The section is a GUI view over state the Client already holds; the Server, the protocol and the Companion, whose Agents tab already lists every Agent, do not change.

What it lists:

- Every recognized Agent on every connected Device, `Unknown` included; a Pane running only a shell is not listed.
- A disconnected Device's Agents are left out, as "Needs you" leaves them out.
- A detached window (ADR 0043) lists what its tree lists: a Device window that Device's Agents, a Workspace window that Workspace's. A Tab window has no sidebar. The collapsed sidebar shows only Workspace avatars, as before.

Grouping and order:

- Each Workspace's Agents sit under its name, in muted text, with ` · Device` once more than one Device is configured. The name does nothing when clicked and does not fold; only the section's own heading folds.
- Urgency is the ranking the Command Palette already uses: a bell, then `Blocked`, `Error`, unseen `Done`, `Working`, and `Idle` with `Unknown`.
- The group holding the most urgent Agent comes first, so a Workspace with a `Blocked` Agent rises to the top; within a group, the most urgent Agent comes first.
- Ties keep Session order: Device, Workspace, Tab, Pane.
- There is no timestamp of the last state change to sort by, and the section adds none.

The row last selected keeps the rank it had when it was selected, and its group with it, until another Agent is selected. Without this, selecting a `Done` Agent marks it seen and it drops among the `Idle` ones under the pointer, and stepping through finished Agents with the keyboard skips one.

A row is one line: the Agent's mark with its state badge, then the Pane's terminal title, or the agent kind's name until a title arrives. On the right, the state's name in its colour (Bell, Blocked, Error, Done or Working); an Idle or Unknown Agent's row shows none, so a quiet row stays quiet. While the Agent carries a `detail`, a second line shows it across the row's width, as the tree's Agent rows do (ADR 0024). The branch is left to the tree: every Agent of a group shares it.

Two earlier layouts were tried and dropped. Each row first carried its Workspace and Device beside the title and its branch below; that crowded the title out. Then the state went beside the title and the Workspace below; every row was two lines, and the Workspace repeated on each row of a Workspace.

- A row is lit while its Pane is the keyboard's target, together with the same Agent's row in the tree.
- A click runs `select_pane`, as "Needs you", Go to and a notification's click do. A row cannot be dragged and has no menu.

The section:

- **Expanded**, it takes a height of its own, a third of the window at first, with a handle on its top edge to drag. Its rows scroll inside that height.
- **Collapsed**, only its heading remains at the bottom of the sidebar, with the Agents counted by state beside the label, as a folded Device heading counts them. A click on the heading or its chevron collapses and expands it.
- With no Agent, the expanded section reads "No agents", so the sidebar does not jump when the first one starts.
- The main window's height and collapse are kept in the GUI state file (ADR 0023); a detached window starts from the main window's.

`next-agent` and `previous-agent` (Cmd+] and Cmd+[ on macOS; Alt+] and Alt+[ elsewhere) step through the section's order in the window last activated, wrapping at both ends. When the keyboard's target is not one of its Agents, next starts at the first row and previous at the last. They are rows of the shortcut table (ADR 0063, ADR 0064), so they can be rebound and the Command Palette lists them. On Windows and Linux, Alt+[ is a key a terminal program would otherwise receive as Meta+[; rebinding the action gives it back.

## Considered

- **A toggle at the top of the sidebar between Workspaces and Agents**, like the right sidebar's Changes, Files and History. Rejected: one of the two would always be hidden, and the person would have to switch before switching.
- **Agents taken out of the Workspace tree, as herdr does.** Rejected: the tree is how Condr is navigated today, and it is kept as it is.
- **Widening "Needs you" to every Agent at the top.** Rejected: every Agent would be listed twice at the top of one scrolling list.
- **A ⌘;-style switcher with nothing on screen.** Rejected: the point is a list that stays in sight.
- **A grid of every Agent's terminal.** Rejected: drawing many live Panes at once contradicts the one droppable visual slot each Client has (ADR 0004).

A blocked Agent now shows three times while the section is expanded: in "Needs you", in the tree and at the top of the section. That is accepted: "Needs you" is what remains in sight once the section is collapsed.

> ADR 0068 adds a Tasks row above "Needs you", makes the sidebar a cached view no terminal frame rebuilds, and reuses this section's order and sticky selection in Tasks mode. In normal mode a row's click and a notification's click still run `select_pane`.

> ADR 0074 lets the section group by state or keep the tree's order from a button on its heading, puts `Blocked` and `Error` first whatever its order, and moves a bell after them in the urgency order.

> ADR 0075 drops the sticky selection: the selected row moves with its Agent's state at once, and every row slides to its new place.
