---
title: Keyboard shortcuts
description: Look up every Condr shortcut on macOS, Windows and Linux.
---

:::caution[Under construction]
This page is still being written. Content will follow.
:::

Check this page when you need a shortcut. Condr also shows the same list under **Settings › Shortcuts**.

## Know which keys Condr handles

Condr only intercepts the shortcuts in the tables. Every other key goes to the program in the Pane, including every Ctrl combination on macOS except Ctrl+Tab. An unbound Cmd or Win combination is discarded and never reaches the terminal. No shortcut works while a dialog is open.

On Windows and Linux, Ctrl+B is left to tmux and readline, so the sidebar uses Ctrl+Shift+B. On macOS, Option plus a letter types that character; Option does not act as a Meta key.

## Work with the window and sidebars

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Open Settings | Cmd+, | Ctrl+, |
| Toggle the left sidebar | Cmd+B | Ctrl+Shift+B |
| Toggle the right sidebar (Changes & Files) | Cmd+Alt+B | Ctrl+Alt+B |
| Close the Settings window | Esc | Esc |

## Switch Workspaces and Tabs

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Previous Workspace | Cmd+Shift+↑ | Ctrl+Shift+↑ |
| Next Workspace | Cmd+Shift+↓ | Ctrl+Shift+↓ |
| New Tab | Cmd+T | Ctrl+Shift+T |
| Next Tab | Cmd+} or Ctrl+Tab | Ctrl+Tab |
| Previous Tab | Cmd+{ or Ctrl+Shift+Tab | Ctrl+Shift+Tab |
| Jump to Tab 1 to 9 | Cmd+1 to Cmd+9 | Alt+1 to Alt+9 |

Workspaces cycle in the order of each Device in the sidebar and wrap around at the end. Use the context menu to rename or close a Workspace or to close a Tab; these actions have no shortcut.

## Work with Panes

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Split right | Cmd+D | Alt+Shift+= |
| Split down | Cmd+Shift+D | Alt+Shift+- |
| Close the Pane | Cmd+W | Ctrl+Shift+W |
| Zoom or restore | Cmd+Shift+Enter | Alt+Shift+Enter |
| Move focus | Cmd+Alt+Arrow | Alt+Arrow |
| Resize | Cmd+Ctrl+Arrow | Alt+Shift+Arrow |

Each press resizes by 5%. Swapping Panes has no shortcut; use the Pane menu or drag instead.

## Work with the terminal

| Action | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Copy | Cmd+C | Ctrl+Shift+C, or Ctrl+C when there is a selection | Ctrl+Shift+C |
| Paste | Cmd+V | Ctrl+Shift+V or Ctrl+V | Ctrl+Shift+V |
| Paste an image to a remote Device | Option+V | Alt+V | Alt+V |
| Open a link | Cmd+click | Ctrl+click | Ctrl+click |
| Select in mouse mode | Shift+drag | Shift+drag | Shift+drag |
| Scroll the history | Shift+PageUp / PageDown, or the mouse wheel | Same as macOS | Same as macOS |

Ctrl+Insert copies and Shift+Insert pastes on all three platforms. On Linux, Ctrl+C and Ctrl+V always go to the terminal. Tab and Shift+Tab also go to the terminal and never move window focus.

Copying ends the selection. There is no clear-screen shortcut; use `clear` or Ctrl+L in the shell.
