---
title: Keyboard shortcuts
description: Look up every Condr shortcut for macOS, Windows, and Linux.
---

:::caution[Under construction]
This page is still being written. Content will follow.
:::

Use this page to find a shortcut. Condr shows the same list in **Settings › Shortcuts**.

## Know which keys reach Condr

Condr intercepts only the shortcuts in the tables. Every other key goes to the program in the Pane, including every Ctrl chord on macOS except Ctrl+Tab. An unbound Cmd or Win chord is dropped and never reaches the terminal. No shortcut works while a dialog is open.

On Windows and Linux, Ctrl+B goes to tmux and readline, so the sidebar uses Ctrl+Shift+B. On macOS, Option plus a letter types that character. Option does not act as Meta.

## Control the window and sidebars

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
| Go to Tab 1 to 9 | Cmd+1 to Cmd+9 | Alt+1 to Alt+9 |

Workspace switching follows every Device in sidebar order, then wraps to the first. Use the right-click menu to rename or close a Workspace or close a Tab. These actions have no shortcut.

## Control Panes

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Split right | Cmd+D | Alt+Shift+= |
| Split down | Cmd+Shift+D | Alt+Shift+- |
| Close Pane | Cmd+W | Ctrl+Shift+W |
| Zoom or unzoom | Cmd+Shift+Enter | Alt+Shift+Enter |
| Move focus | Cmd+Alt+Arrows | Alt+Arrows |
| Resize | Cmd+Ctrl+Arrows | Alt+Shift+Arrows |

Each resize press moves 5%. Use the Pane menu or drag to swap Panes because it has no shortcut.

## Control the terminal

| Action | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Copy | Cmd+C | Ctrl+Shift+C, or Ctrl+C with a selection | Ctrl+Shift+C |
| Paste | Cmd+V | Ctrl+Shift+V or Ctrl+V | Ctrl+Shift+V |
| Paste an image to a remote Device | Option+V | Alt+V | Alt+V |
| Open a link | Cmd+click | Ctrl+click | Ctrl+click |
| Select while a program has mouse mode on | Shift+drag | Shift+drag | Shift+drag |
| Scroll history | Shift+PageUp / PageDown, or the wheel | Same | Same |

Ctrl+Insert copies and Shift+Insert pastes on all three platforms. On Linux, Ctrl+C and Ctrl+V always go to the terminal. Tab and Shift+Tab also go to the terminal, so they do not move focus in the window.

Copying ends the selection. There is no clear-screen shortcut. Use `clear` or Ctrl+L in the shell.
