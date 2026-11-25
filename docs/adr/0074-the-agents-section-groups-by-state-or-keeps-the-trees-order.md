# The Agents Section Groups by State or Keeps the Tree's Order

Status: accepted 2026-11-23. Amends ADR 0067, whose section grouped by Workspace and ordered by urgency only, and the urgency order the Command Palette (ADR 0064) and Tasks mode (ADR 0068) share with it.

ADR 0067's section answers one question: which Workspace wants me most. Two others had no answer. A person running many Agents wants to see every finished one together and every working one together, wherever they run. And the urgency order moves a row whenever its Agent's state changes, so a list the person reads top to bottom reshuffles under them while Agents work.

## Decision

While expanded, the section's heading carries a button at its right, after the label, that opens a menu in two parts, a check on each part's current choice:

- **Group**: Workspace, as ADR 0067 has it and the default, or State.
- **Order**: Urgency, as ADR 0067 has it and the default, or Sidebar order.

Folded, the heading shows its counts by state as before and no button.

**State** groups the Agents under their states' names, in the muted text a Workspace's name has: Blocked, Error, Bell, Done, Working, Idle, an `Unknown` Agent with the Idle ones, as the folded heading counts it. A state with no Agent has no heading. Within a state the rows keep Session order. A row names its Workspace on its right, muted, with ` · Device` once more than one Device is configured, in place of its state, which the heading and its badge already say. Order is greyed while grouping by State: within one state every order is Session order.

**Sidebar order** lists the groups and their rows in the tree's order, Device, Workspace, Tab, Pane, and a state change moves nothing, with one exception below.

**`Blocked` and `Error` come first whatever the grouping and order.** A Workspace group holding one comes first, and they come first within it, a `Blocked` before an `Error`; grouped by State, their headings are the first two. They stay in their group rather than rising above every group, which would repeat "Needs you" and lose their Workspace.

The urgency order changes with it, everywhere it is used: Blocked, Error, Bell, Done, Working, Idle. A bell came first before. The Command Palette's Go to, each Tasks column and the folded counts follow it.

ADR 0067's sticky selection holds in every grouping and order: an answered `Blocked` Agent, now `Working`, keeps its place at the top until another Agent is selected, so stepping through the blocked ones with the keyboard skips none. `next-agent` and `previous-agent` step through the order shown.

Both choices are each window's own and kept with the section's height and fold in the GUI state file; a detached window starts from the main window's (ADR 0023).

## Considered

- **Grouping by Device.** Dropped: a Workspace group already names its Device once more than one is configured, grouping by Device in Sidebar order is grouping by Workspace under other headings, and with one Device it is one group.
- **Ordering by Device, by Workspace or by title.** Dropped. Session order already nests Workspaces under Devices, so ordering by either is grouping by it, and both come down to Sidebar order. A title moves with the spinner an agent CLI puts in front of it and with each new summary it writes.
- **One menu of three views**, Workspace by urgency, Workspace in Sidebar order and State, with nothing greyed. Declined: two parts say what each choice changes.
- **No grouping.** Declined: State grouping already gives one list in urgency order, with headings.

## Tests

The section's order: Workspace and State grouping under both orders, `Blocked` and `Error` first under Sidebar order with their group, `Unknown` among the Idle, the anchored Agent keeping its State heading, and the new urgency order putting a bell after `Error`. The GUI state round-trip carries both choices.

> ADR 0075 drops the sticky selection this decision kept; `next-agent` and `previous-agent` still step through the order shown.
