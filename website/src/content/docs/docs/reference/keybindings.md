---
title: Keyboard shortcuts
description: Every Condr key mapping on macOS, Windows and Linux, plus how copy and paste, scrolling and mouse interaction behave in the terminal.
---

Condr's shortcuts work on two levels:
* **Window-level shortcuts**: drive the window, Workspaces, Tabs and Panes. The client intercepts and consumes them, and never passes them through to the terminal's foreground process.
* **Terminal-level shortcuts**: cover the text and image clipboard, scrollback and mouse interaction, and work only while the current terminal Pane has focus.

Every key combination not declared on this page passes through to the child process running in the current Pane.

> **macOS note**: on macOS the `Option` key produces special characters (`Option + S` types `ß`) and is not mapped to the terminal's `Meta` key. So word-movement shortcuts in the shell, such as `Option + B` and `Option + F`, do not work.

---

## Window and sidebars

| Action | macOS | Windows / Linux |
| :--- | :--- | :--- |
| Open Settings | `Cmd + ,` | `Ctrl + ,` |
| Close Settings | `Esc` | `Esc` |
| Toggle the left sidebar | `Cmd + B` | `Ctrl + Shift + B` |
| Toggle the right sidebar (Changes & Files) | `Cmd + Option + B` | `Ctrl + Alt + B` |

---

## Workspaces

| Action | macOS | Windows / Linux |
| :--- | :--- | :--- |
| Previous Workspace | `Cmd + Shift + ↑` | `Ctrl + Shift + ↑` |
| Next Workspace | `Cmd + Shift + ↓` | `Ctrl + Shift + ↓` |

* **Cycling**: follows the sidebar order through the Workspaces of every Device, and wraps around at either end.
* **Starting point**: when no Workspace is shown, `↓` goes to the first Workspace and `↑` to the last.

---

## Tabs

| Action | macOS | Windows / Linux |
| :--- | :--- | :--- |
| New Tab | `Cmd + T` | `Ctrl + Shift + T` |
| Next Tab | `Cmd + }` or `Ctrl + Tab` | `Ctrl + Tab` |
| Previous Tab | `Cmd + {` or `Ctrl + Shift + Tab` | `Ctrl + Shift + Tab` |
| Go to Tab 1~9 | `Cmd + 1` ~ `Cmd + 9` | `Alt + 1` ~ `Alt + 9` |

* **Physical keys**: `Cmd + }` / `Cmd + {` are pressed as `Cmd + Shift + ]` / `Cmd + Shift + [`.
* **Cycling**: switching Tabs wraps around at either end.

---

## Pane splits and layout

| Action | macOS | Windows / Linux |
| :--- | :--- | :--- |
| Split horizontally (new Pane to the right) | `Cmd + D` | `Alt + Shift + =` |
| Split vertically (new Pane below) | `Cmd + Shift + D` | `Alt + Shift + -` |
| Close the current Pane | `Cmd + W` | `Ctrl + Shift + W` |
| Maximize / restore the focused Pane | `Cmd + Shift + Enter` | `Alt + Shift + Enter` |
| Move focus | `Cmd + Option + Arrow` | `Alt + Arrow` |
| Resize the split | `Cmd + Ctrl + Arrow` | `Alt + Shift + Arrow` |

* **Resize step**: each shortcut press changes the split by 5%.
* **Drag and drop**:
  * **Swap places**: drag a Pane's title bar and release it on the center of the target.
  * **Split and dock**: drag it to the target's edge to split the target Pane in that direction and dock there.

---

## Terminal: copy and paste text

| Action | macOS | Windows | Linux |
| :--- | :--- | :--- | :--- |
| Copy the selection | `Cmd + C` | `Ctrl + C` / `Ctrl + Shift + C` | `Ctrl + Shift + C` |
| Paste text | `Cmd + V` | `Ctrl + Shift + V` / `Ctrl + V` | `Ctrl + Shift + V` |
| Copy the selection (everywhere) | `Ctrl + Insert` | `Ctrl + Insert` | `Ctrl + Insert` |
| Paste text (everywhere) | `Shift + Insert` | `Shift + Insert` | `Shift + Insert` |

### Pass-through rules

* **Selection lifetime**: copying clears the current selection highlight.
* **`Cmd + C` with nothing selected on macOS**: passes through to the foreground program. Supported programs, such as Claude Code, copy their own selection, while the shell and other programs receive nothing. On a Windows Device, `Cmd + C` reaches Claude Code too, as long as Condr has recognized it in the Pane.
* **Conflicts on Windows**:
  * `Ctrl + C` with nothing selected passes through to the foreground application, usually to interrupt the current command.
  * `Ctrl + V` is always intercepted by Condr to paste, and the raw key is never sent to the child process.
* **Linux**: `Ctrl + C` and `Ctrl + V` always pass through to the current child process.

---

## Terminal: send an image to a remote Device

| Action | macOS | Windows / Linux |
| :--- | :--- | :--- |
| Paste the clipboard image | `Option + V` | `Alt + V` |

* **How it works**: only in Panes running on a remote Device. Condr uploads the local clipboard image to a temporary directory on that Device in the background, and fills its absolute remote path in at the command-line cursor for a CLI Agent or another program to read.
* **Fallback**: with no image on the clipboard, or when the current Pane is on the local Device, the key passes straight through to the foreground program.
* **Limits and lifetime**:
  * One image can be at most **16 MiB**.
  * The staged file on the remote Device is removed when this client disconnects or the remote Server process stops.

---

## Terminal: scrolling and scrollback

| Action | macOS | Windows / Linux |
| :--- | :--- | :--- |
| Page up through scrollback | `Shift + PageUp` | `Shift + PageUp` |
| Page down through scrollback | `Shift + PageDown` | `Shift + PageDown` |
| Scroll line by line | Mouse wheel | Mouse wheel |

* **Snap to bottom**: while you browse the scrollback, any key typed into the terminal jumps straight back to the newest line.
* **Alt screen mode (vim, less and the like)**: `Shift + PageUp / PageDown` pass through to the TUI program; the mouse wheel is translated into `↑` / `↓` arrow key events.
* **Mouse reporting**: when the foreground program explicitly turns on mouse tracking, its own event loop takes over wheel events.

---

## Terminal: mouse interaction

| Action | macOS | Windows / Linux |
| :--- | :--- | :--- |
| Open a URL | `Cmd + click` | `Ctrl + click` |
| Force text selection (when a program owns the mouse) | `Shift + drag` | `Shift + drag` |

---

## Further reading

* [Shortcuts and preferences](/docs/using/preferences/): everyday shortcuts and external editor setup
* [Workspaces, Tabs and Panes](/docs/using/workspaces/): split layouts and multitasking workflows
* [Configuration and settings](/docs/reference/configuration/): Settings options and the `config.toml` reference
