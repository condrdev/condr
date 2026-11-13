# Double Shift Opens a Command Palette of Actions and Places

Status: accepted 2026-11-12. Amends ADR 0063: the shortcut table holds every window action, and an action may have no default key.

A window action was reached by remembering its key or finding its button, eleven had no key at all, and Settings > Shortcuts is a list to read. With several Devices, Workspaces, Tabs and Agents, finding one meant walking the sidebar. herdr-app (Apache-2.0) answers both with two popups, a command palette and a Go to picker. Condr puts both in one overlay, as IntelliJ's Search Everywhere does, opened by pressing Shift twice.

## One overlay, three pages

The Command Palette is a search field over three pages, All, Actions and Go to; Tab and Shift+Tab move between them. All shows a Go to section above an Actions section, at most five rows each; the other two show every match of their kind. Esc, a click outside, or opening it again closes it.

It opens three ways:

- Shift pressed and released twice within a second, either Shift, with no other key between, opens it on All. GPUI makes a modifier pressed and released alone a keystroke of its own, so this is the binding `shift shift`, and the second must come within GPUI's one-second wait for the rest of a sequence. A lone `shift` that no second one follows is handed on, and the terminal sends nothing for it, so nothing is taken from a program. It does nothing while a modal dialog other than the Command Palette is open, like every window shortcut. It cannot be turned off: a key in `[client.keybindings]` needs Cmd, Ctrl or Alt (ADR 0063), and a switch waits for someone whose input method's Shift toggle trips it.
- `command-palette` (Cmd+Shift+P; Ctrl+Shift+P elsewhere) opens it on Actions, and `go-to` (Cmd+P; Ctrl+Shift+K) on Go to. They are two rows of the shortcut table, "Command Palette: Actions" and "Command Palette: Go to", so they can be rebound; their keys are bound in the palette's own key context as well, so they close it. Ctrl+P stays the terminal's.
- The magnifier at the bottom of the sidebar, beside ⇄, opens it on All.

Settings > Shortcuts lists Shift twice at the end of the Application group, as a row that only shows its keys, like the terminal's copy and paste.

## Actions

The rows are `keymap.rs`'s `GROUPS` (ADR 0063), which now holds every window action. The eleven without a key join with empty defaults: New, Rename and Close workspace, Rename and Close tab, the four Swaps, and Connect remote device and Reconnect device in a new Devices group. Settings > Shortcuts lists them too, so any of them can be given a key. A row shows the table's label and its first current key, read from the installed keymap, so a rebound key shows. Left out: Go to tab 1–9, which Go to replaces; the palette's own two rows; and macOS's system chords, which the app menu carries.

Each row of the table gains `needs`, what the action acts on: nothing, a focused Pane, a presented Tab, a presented Tab on a connection that can change the Session, such a connection, any Workspace, or a Workspace whose right sidebar the window draws. It is judged for the window the palette opened in. A disabled action is listed greyed and cannot be chosen; the handlers keep their own checks. Choosing an action closes the palette, gives focus back to that window and dispatches the action, so it runs through the handler its key runs, on the Tab and Pane that window presents.

The five actions chosen last lead the Actions section while the query is empty, and win ties after. They are kept in memory until the GUI quits; with none chosen yet, an empty All shows only Go to.

## Go to

Four kinds of row, from every connected Device, whichever window the palette opened in:

| Row | Shows | Matched and highlighted | Matched only |
| --- | --- | --- | --- |
| Agent | its mark with the state on it, Pane title, Workspace, the state's name | Pane title | Agent kind, Workspace and Tab names |
| Tab | its number and name, or its focused Pane's title as the tab bar has it; Workspace | Tab name | Workspace name |
| Workspace | name, Root Directory | name | Root Directory |
| Device | plug, name, connection state | name | |

With more than one Device connected, each row ends with "· Device", as Needs you does. A Pane with no Agent has no row; its Tab does. A disconnected Device lists itself and nothing it held, as the sidebar does (ADR 0020). `detail` stays in the sidebar and the notification.

With an empty query, Agents come first by urgency, the sidebar's `agent_status_summary` order: bell, Blocked, Error, Done, Working, Idle, with `Unknown` counted as Idle. Tabs, Workspaces and Devices follow in sidebar order. All's Go to section shows the first five Agents.

Choosing an Agent does what clicking it in the sidebar does, `select_pane`. A Tab, or a Workspace's shown Tab, goes the same way to its focused Pane, so typing goes on there; a viewer Tab, which has none, is shown as an `Activated` event shows it. A Device is presented by the window that draws it. Whichever window that is comes forward (ADR 0043).

## Matching

`nucleo-matcher`, helix's fzf-style matcher (MPL-2.0, which `about.toml` accepts), scores each row without regard to case and gives the matched characters to highlight. Its grapheme segmentation is off, so they are character positions; whitespace separates words that must all match, and fzf's `^`, `$`, `'` and `!` mean nothing here. Rows rank by score. Ties go to the more urgent Agent, then Agent before Tab before Workspace before Device, and among actions to the recently chosen. Urgency never outranks a better match: typing a name finds that name, not an unrelated Blocked Agent.

A row whose highlighted text does not match tries its matched-only text, which repeats the label, for an action its group name with its label. Such a match ranks below every highlighted one and highlights nothing, so "pane zoom" finds Toggle zoom. The table has no keywords column.

## How it is built

Kit's `Command` in a Kit Dialog, with `filterable(false)` and `on_query` for the matching above, and each row drawn through `.child()` with the sidebar's Agent icon and state badge. Kit's own confirm dispatches from the dialog's focus, which sits outside the Condr view and never reaches its handlers; hence close, refocus, dispatch.

Not done: terminal output (in-terminal search), file contents, opening files by name, commands from plugins, recency that survives a restart, a switch for Double Shift, and the Pane menu's Title Case labels, which stay as they are.
