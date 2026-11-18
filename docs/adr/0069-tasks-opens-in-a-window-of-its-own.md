# Tasks Opens in a Window of Its Own

Status: proposed 2026-11-18. It is accepted once the acceptance on Windows with two screens that Delivery names has passed. When accepted, it amends ADR 0068, which kept Tasks mode to the main window and left detached windows for later, and ADR 0043, whose units it joins with one that is a page rather than a part of the Session tree.

Tasks mode (ADR 0068) covers the main window's content area: while it shows, the Workspace does not, so watching the board and working in a terminal take turns. A person with two screens wants the board on one and the Workspace on the other, as ADR 0043 lets them do with a Device, a Workspace or a Tab.

## The Tasks window

Tasks opens in a window of its own: the **Tasks window**. There is at most one, and it shows everything; its toolbar's scope narrows it to a Device or a Workspace as in the main window. A Device, Workspace or Tab window still has no Tasks mode. Tasks is drawn in one place at a time, as ADR 0043's units are: the main window's Tasks mode or the Tasks window.

The Tasks window has no left sidebar. Its title bar reads `Condr – Tasks`, and below it are the toolbar, the cards and the interaction area, exactly as in Tasks mode; the main window, with its sidebar, is beside it on the other screen, and what the sidebar does in Tasks mode, its Agent and "Needs you" rows selecting a card, the cards and the Needs you column already do. A Device's connection state, which the sidebar beside Tasks mode carries, moves into the scope's menu, where each Device carries ADR 0020's mark; a disconnected Device's cards are hidden as ADR 0067 hides its rows, and the main window's sidebar says why. `toggle-sidebar` and `toggle-tasks` do nothing in the Tasks window, and every layout and Tab action stays inert as in Tasks mode, `Cmd+W` included: it closes neither a Pane nor the window (ADR 0043).

## Opening and bringing back

In Tasks mode the main window's title bar ends with **Open in New Window**, an icon and a label, where a detached window has Bring Back. There is no context menu entry on the Tasks row, no drag out of the window, no shortcut and no command: the title bar is where a person in Tasks mode is looking, and the button is the one way.

The window opens as ADR 0043's do, on the main window's display, at its size, cascaded from it. The main window then shows the Workspace and Tab it showed before Tasks mode, rather than ADR 0043's page saying where Tasks went: moving Tasks out is for working in the main window, and the new window opens in front of it, so where Tasks went is in plain sight.

The Tasks window's title bar ends with **Bring Back**: Tasks returns to the main window, which enters Tasks mode and is activated. Closing the Tasks window only puts Tasks away; the main window keeps showing what it shows. This departs from ADR 0043, where closing a detached window and Bring Back are the same: a Tab whose window closes must be shown somewhere, but Tasks is a mode, its row is always there, and a person closing the window most likely wants the board gone, not back over their terminal.

Moving between windows takes everything along and asks nothing: the selected card, the open interaction area, and a Task's form with unsaved text. Closing the Tasks window is leaving Tasks mode, which asks nothing either: the selection and the form stay in memory, there the next time Tasks shows, in the main window or a new Tasks window. A restart keeps no selection, as ADR 0068 has it.

While the Tasks window is open, the main window's Tasks row and `toggle-tasks` show ADR 0043's page in its content area: Tasks is shown in another window, with **Go to Window** and **Bring Back**.

## The interaction area

A terminal is never drawn twice (ADR 0043, ADR 0068). With the Tasks window beside the main window, the main window can be showing the very Tab whose Pane the interaction area shows. Then the interaction area yields: it shows ADR 0043's page for the Pane, with Go to Window. Only a window that draws the Tab right now counts, the Tab it shows, not every Tab it holds; otherwise nearly every Pane would belong to the main window and the area would almost always yield. When that window shows another Tab, the area shows the terminal again, without taking the keyboard.

The sizing rules are ADR 0036's and 0068's unchanged: one Client, whose window last activated holds the sizes, and the area's terminal sizes the PTY only while it holds the keyboard; an Agent moved between the area and its Tab redraws once, as entering and leaving Tasks mode makes it do today.

## Going places from the Tasks window

The Tasks window shows Tasks and nothing else; every other place is the main window's.

- **Go to** in the Command Palette, run in the Tasks window, selects the card of an Agent it names. Any other place it names, a Workspace, a Tab, a Device or the Ports page, shows in the main window, which is activated.
- **Open in Workspace** on the selected card shows its Pane in its Workspace and Tab in the main window, which is activated; the interaction area then yields as above.
- **An OS notification's click**, while the Tasks window is open, selects its Agent's card there, widening the scope to All when the Agent is outside it. If another window draws that Agent's Tab right now, that window is activated, since the area would only point to it; otherwise the Tasks window is, restored when minimized.

The main window keeps working as in normal mode: its sidebar's Agent and "Needs you" rows, `next-agent` and `previous-agent` go to the Pane there, as ADR 0067 has them, and never pull the keyboard to the Tasks window.

## State

The GUI state file (ADR 0023) keeps one set of Tasks preferences, layout, density, folded columns and groups, scope and splits, which follows Tasks between windows. The list of detached windows gains the Tasks window, with its bounds and display, and it opens again after a restart once the main window has, as ADR 0043's windows do.

## Considered

- **A Tasks window without the interaction area**, only the cards, a click selecting the Pane in the main window. Simpler, since no terminal could be drawn twice; declined because answering an Agent, `Cmd+]`, answering, the reason Tasks mode has the area, would need a trip to the other window every time.
- **A Tasks window with the left sidebar**, as in Tasks mode. Declined: the main window's sidebar is beside it, and two sidebars on two screens list the same thing.
- **The area taking the Pane from the main window**, which would show a placeholder where it was. Declined: ADR 0043 moves units, not Panes, and a placeholder inside a Tab's Dock is a mechanism of its own.
- **Tasks mode in Device or Workspace windows**, or more than one Tasks window. Declined until asked for: the same Agent's card in two windows needs an answer to which one shows its terminal.
- **Opening it from the Tasks row's context menu or by dragging the row out**, as ADR 0043's units open. Declined for one button where Tasks mode's title is.
- **Closing the window bringing Tasks back**, as ADR 0043 has it. Declined above.

## Not in this decision

Quitting with a Task's form holding unsaved text loses it without asking, in the main window today as in the Tasks window; that is a gap of its own. A Tasks window per Device, a Tasks window without the main window, and drags between the Tasks window and the others.

## Consequences

- `WindowScope` gains `Tasks`, a scope that holds no part of the Session tree. What Tasks mode keeps per window, its preferences, selection, card area, focus and Dock, belongs to whichever window shows Tasks, and moves with it.
- Whether a window draws a Tab now counts the Tab it shows, for the interaction area; ADR 0043's units keep their rule.

Tests, in `visual/tasks.rs` with two windows:

- Open in New Window: the main window back on its Workspace and Tab, the Tasks window showing the same layout, selection and unsaved form; the main window's Tasks row then showing the page, whose Go to Window and Bring Back work.
- Bring Back entering Tasks mode in the main window; closing the window leaving the main window as it was, and Tasks mode then showing the same selection and form.
- The area yielding while the main window shows the selected Pane's Tab and coming back without the keyboard when it shows another.
- Go to, Open in Workspace and a notification's click from the Tasks window, each activating the window this decision names; the main window's rows and `next-agent` staying in it.
- The scope menu's connection marks; layout and Tab actions, `Cmd+W`, `toggle-sidebar` and `toggle-tasks` inert in the Tasks window.
- The state file saving and restoring the Tasks window and the one set of preferences.

## Delivery

One change: this decision, the scope and the moving state, the title bar buttons and the page, the area's rule, the routing, the state file, the messages in English and Simplified Chinese, the tests above, and the website's page on Tasks mode. It is accepted once the acceptance on Windows with two screens has passed: the Tasks window's area and the main window's terminal both under a Codex stream, selecting by drag, typing, stepping cards with `Alt+]` and switching windows at display cadence with no backlog; then checked by hand on macOS.
