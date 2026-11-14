---
title: Keyboard shortcuts
description: Every Condr key mapping on macOS, Windows and Linux, how to change the window shortcuts and make Option act as Meta, and how copy and paste, scrolling and mouse interaction behave in the terminal.
---

Condr's shortcuts work on two levels:
* **Window-level shortcuts**: drive the window, Workspaces, Tabs and Panes. The client intercepts and consumes them, and never passes them through to the terminal's foreground process. The tables below give their defaults; you can [change them](#change-a-shortcut).
* **Terminal-level shortcuts**: cover the text and image clipboard, scrollback and mouse interaction, and work only while the current terminal Pane has focus.

Every key combination not declared on this page passes through to the child process running in the current Pane.

> **macOS note**: by default the `Option` key types special characters (`Option + S` types `ß`), as it does in Terminal.app and iTerm2. For word movement in the shell, such as `Option + B` and `Option + F`, turn on [Option as Meta](#option-as-meta).

---

## Window and sidebars

| Action | macOS | Windows / Linux |
| :--- | :--- | :--- |
| Open Settings | `Cmd + ,` | `Ctrl + ,` |
| Close Settings | `Esc` | `Esc` |
| Toggle the left sidebar | `Cmd + B` | `Ctrl + Shift + B` |
| Toggle the right sidebar (Changes & Files) | `Cmd + Option + B` | `Ctrl + Alt + B` |

---

## Command Palette

| Action | macOS | Windows / Linux |
| :--- | :--- | :--- |
| Open the Command Palette | `Shift` twice | `Shift` twice |
| Open it on Actions | `Cmd + Shift + P` | `Ctrl + Shift + P` |
| Open it on Go to | `Cmd + P` | `Ctrl + Shift + K` |
| Next / previous page | `Tab` / `Shift + Tab` | `Tab` / `Shift + Tab` |
| Run the highlighted row | `Enter` | `Enter` |
| Close | `Esc` or the key that opened it | `Esc` or the key that opened it |

* **Pages**: **All** shows up to five places and five actions that match; **Actions** lists every window action with its key, greyed where it cannot run in this window; **Go to** lists every connected Device and its Workspaces, Tabs and Agents, the Agents that need you first.
* **Matching**: type letters of a name in order, not necessarily together; separate words must all match. It finds names, not terminal output or file contents.
* **`Shift` twice**: press and release `Shift` twice within a second, with no other key between. It cannot be changed; the other two can.
* The magnifier at the bottom of the left sidebar opens it too.

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
* **Copy confirmation**: copies that pass through Condr show "Copied to clipboard", whether from Condr's own selection or from a program such as Claude Code. A program's own native copy bypasses Condr and shows nothing: Codex, for example, copies to the clipboard of the machine it runs on, so on a remote Device use `Shift + drag` to let Condr copy instead.
* **`Cmd + C` with nothing selected on macOS**: passes through to the foreground program. Supported programs, such as Claude Code, copy their own selection, while the shell and other programs receive nothing. On a Windows Device, `Cmd + C` reaches Claude Code too, as long as Condr has recognized it in the Pane.
* **Conflicts on Windows**:
  * `Ctrl + C` with nothing selected passes through to the foreground application, usually to interrupt the current command.
  * `Ctrl + V` is always intercepted by Condr to paste, and the raw key is never sent to the child process.
* **Linux**: `Ctrl + C` and `Ctrl + V` pass through to the current child process. The one exception is `Ctrl + V` with an image on the clipboard in a remote Pane, which sends the image (see below); macOS does the same.

---

## Terminal: send an image to a remote Device

| Action | macOS | Windows | Linux |
| :--- | :--- | :--- | :--- |
| Paste the clipboard image | `Ctrl + V` | `Alt + V` | `Ctrl + V` |

* **How it works**: only in Panes running on a remote Device. Condr uploads the local clipboard image to a temporary directory on that Device in the background, and fills its absolute remote path in at the command-line cursor for a CLI Agent or another program to read.
* **Which key**: set by the platform the GUI runs on, whatever system the remote Device runs. A Windows GUI connected to a Mac takes `Alt + V`; a Mac GUI connected to Windows takes `Ctrl + V`. It is the key you paste images into a local Agent with on that platform: on Windows `Ctrl + V` pastes text, which is why Claude Code there takes `Alt + V` too.
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
| Jump to the previous / next shell prompt | `Cmd + ↑` / `Cmd + ↓` | `Ctrl + ↑` / `Ctrl + ↓` |
| Scroll line by line | Mouse wheel | Mouse wheel |

* **Snap to bottom**: while you browse the scrollback, any key typed into the terminal jumps straight back to the newest line.
* **Prompt jumps**: work while the shell waits at its prompt and needs [shell integration](/docs/using/preferences/#shell-integration). Anywhere else, such as inside vim, the keys go to the program.
* **Alt screen mode (vim, less and the like)**: `Shift + PageUp / PageDown` pass through to the TUI program; the mouse wheel is translated into `↑` / `↓` arrow key events.
* **Mouse reporting**: when the foreground program explicitly turns on mouse tracking, its own event loop takes over wheel events.

---

## Terminal: mouse interaction

| Action | macOS | Windows / Linux |
| :--- | :--- | :--- |
| Open a URL | `Cmd + click` | `Ctrl + click` |
| Force text selection (when a program owns the mouse) | `Shift + drag` | `Shift + drag` |

---

## Change a shortcut

Every shortcut in the Window, Command Palette, Workspace, Tab and Pane tables above can be rebound or removed, except `Shift` twice. The actions without a default key, such as Close tab, Rename workspace or the Pane swaps, can be given one. The terminal's own keys (copy and paste, image paste, prompt jumps and the mouse) stay as they are.

* **In Settings**: open **Shortcuts**. Click the pencil beside an action and press the new key, or `Esc` to cancel; it replaces the action's keys. The reset button brings back the defaults. A change applies at once.
* **In `config.toml`**: [`[client.keybindings]`](/docs/reference/configuration/#clientkeybindings-window-shortcuts) lists each action's name. Restart Condr after a hand edit.
* **Rules**:
  * A shortcut needs `Cmd`, `Ctrl` or `Option` (`Ctrl` or `Alt` on Windows and Linux), unless it is an F-key: every other key is for typing.
  * A key belongs to one action. Giving it to another takes it away from the first.
  * On macOS, `Cmd + Q`, `Cmd + H`, `Cmd + Option + H`, `Cmd + M` and `Cmd + Ctrl + F` belong to the system and cannot be taken.
  * One key per shortcut: no sequences such as tmux's prefix key.
  * A shortcut reaches Condr before the terminal, even a key a program would want, such as `Ctrl + C`.
  * A key Condr cannot use is left out and the reason shows under its action on the Shortcuts page.

---

## Option as Meta

**macOS only.** Under **Settings > Shortcuts > Option as Meta**, choose which `Option` key sends Meta, `Esc` followed by the key, to the terminal instead of typing a character: **Left Option**, **Right Option** or **Both**. With one side set, `Option + B` on it moves back a word in the shell, while the other `Option` still types `å`, `é` or the `@` some layouts put there. `Option` then never starts an accent such as `Option + E` either. The setting is `macos_option_as_meta` in [`[client.terminal]`](/docs/reference/configuration/#clientterminal-terminal-display).

On Windows and Linux, `Alt` is always Meta.

---

## Further reading

* [Shortcuts and preferences](/docs/using/preferences/): everyday shortcuts and external editor setup
* [Workspaces, Tabs and Panes](/docs/using/workspaces/): split layouts and multitasking workflows
* [Configuration and settings](/docs/reference/configuration/): Settings options and the `config.toml` reference
