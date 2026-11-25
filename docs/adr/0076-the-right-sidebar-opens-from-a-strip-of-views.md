# The Right Sidebar Opens From a Strip of Views

Status: accepted 2026-11-25. Supersedes ADR 0018's header tab strip, ADR 0046's place for Refresh, the title bar's toggle beside "Open in", and the name `toggle-changes`.

The right sidebar chose among Files, Changes and History with a tab strip in its own header and opened from a button in the title bar. Three tabs already left the header too narrow for the summed `+N −N` (ADR 0046), so a fourth view would not fit, and the sidebar's header sat under the Tab strip although both belong to the Workspace. JetBrains IDEs answer the same problem with a column of tool-window buttons along the window's edge, which takes any number of tools and shows them while every tool window is closed.

## Decision

**A strip of buttons along the window's right edge opens the views**, one icon button each, top to bottom Files, Changes and History (Lucide `folder-tree`, `git-commit-horizontal`, `git-branch`), named by their tooltips. A view's button opens the sidebar on that view; the button of the view shown is selected, and clicking it closes the sidebar. History's button is there only where the Server serves History (ADR 0046). The strip is wherever the right sidebar can be, the Workspace page of a window that draws its Workspace, and always drawn there, open or closed; Tab windows, the Tasks window, Tasks mode, Ports and the welcome page have none. Which view a Workspace shows and whether its sidebar is open stay this Client's view per Workspace (ADR 0023). The title bar's toggle is gone; its shortcut, now `toggle-right-sidebar`, opens the sidebar on the view it last showed, which outside a repository starts as Files (ADR 0018).

**The open sidebar's header sits in the title bar above it**, as the left sidebar's segment does, so the Tab strip and the sidebar are siblings under the Workspace rather than one under the other: the view's name, Changes with its count as "Changes (N)" (ADR 0017), and Refresh at the far end (ADR 0046). The Tab strip's segment takes the window's width less the left sidebar, the right sidebar and the strip, and the header the rest, so the header's edge meets the sidebar's whatever the window controls take. On macOS, whose controls are on the other side, the strip runs the window's full height beside the title bar; on Windows and Linux the controls hold the corner, the strip starts below the title bar and the header reaches them.

**The sidebar's lists always show their scrollbar**, whatever the theme's mode: a list longer than the sidebar says so before anyone scrolls it. Kit's `overflow_y_scrollbar` follows the theme, so the lists scroll through `SidebarList`, a Kit `Scrollbar` set to `Always` over a tracked scroll area.

One view shows at a time. The strip holds only the presented Workspace's views; a tool that belongs to no Workspace, such as Ports or the Tasks board, is a separate decision.

`toggle-changes` in `[client.keybindings]` is renamed `toggle-right-sidebar`. The GUI rewrites an entry still written the old way when it reads the config, keeping a `toggle-right-sidebar` already there, marked `compat(0.2.6)`.

## Considered

- **Labels written down the strip**, as JetBrains' classic UI has them. Declined: GPUI does not rotate text, and a letter per line reads badly in English.
- **Changes' count as a badge on its icon.** Declined: a number in a 24 px button is cramped, and the header already shows it.
- **Two groups and split mode**, two tool windows open at once, and dragging buttons to reorder or hide them. Declined until there are more views than fit.
- **The strip below the title bar on macOS too.** Declined: an empty cell would sit above it.
- **Keeping the title bar's toggle beside the strip.** Declined: two controls for one thing.

## Tests

The strip absent without a Workspace and at the window's right edge with one; a view's button opening the sidebar on it, with the header in the title bar edge on edge with the column, and closing it again; the strip switching views; the shortcut opening Files outside a repository; `toggle-changes` rewritten to `toggle-right-sidebar` without overwriting one already there.
