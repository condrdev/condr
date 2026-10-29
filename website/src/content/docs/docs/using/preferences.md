---
title: Shortcuts and preferences
description: Everyday keyboard shortcuts, custom external editors, and terminal preferences.
---

Settings and keyboard shortcuts let you fit Condr to the way you work.

## Open Settings

- **Shortcut**: press `Cmd + ,` on macOS, or `Ctrl + ,` on Windows and Linux.
- **The two tabs**:
  - **Application**: this window's settings, such as the theme, terminal font size, system notifications and the shortcut list.
  - **Device**: one Device's Server settings, such as the default shell, connection status and Agent integrations.

## Add an external editor (Open in)

Right-click a file in **Files** or **Changes** to open it in an external IDE or editor. The **Open in** button in the title bar opens the whole Workspace root instead. Both work on local Devices only.

Condr finds installed copies of Zed, VS Code, Cursor and IntelliJ IDEA on its own. To add another editor, put a `[[client.editors]]` entry in `config.toml`:

```toml
[[client.editors]]
name = "Helix"
command = ["hx"]

[[client.editors]]
name = "VS Code Insiders"
command = ["code-insiders", "--reuse-window"]
```

### Fields

- `name`: the label shown in the Open in menu.
- `command`: the program and its leading arguments. Condr appends the path to open as the last argument.
- Restart the window after a hand edit. Condr remembers each repository's last choice in `[[client.workspace_editors]]`; leave that table alone.

## Everyday shortcuts

### Window and Workspaces

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Open Settings | Cmd+, | Ctrl+, |
| Toggle the left sidebar | Cmd+B | Ctrl+Shift+B |
| Toggle the right Changes sidebar | Cmd+Option+B | Ctrl+Alt+B |
| Previous Workspace | Cmd+Shift+↑ | Ctrl+Shift+↑ |
| Next Workspace | Cmd+Shift+↓ | Ctrl+Shift+↓ |

### Tabs and Panes

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| New Tab | Cmd+T | Ctrl+Shift+T |
| Split right | Cmd+D | Alt+Shift+= |
| Split down | Cmd+Shift+D | Alt+Shift+- |
| Close the current Pane | Cmd+W | Ctrl+Shift+W |
| Zoom or unzoom the current Pane | Cmd+Shift+Enter | Alt+Shift+Enter |

### Terminal

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Paste an image to a remote Device | Ctrl+V | Alt+V on Windows, Ctrl+V on Linux |
| Select text while a program has mouse mode on | Shift+drag | Shift+drag |
| Open a link in the terminal | Cmd+click | Ctrl+click |

See [Keyboard shortcuts](/docs/reference/keybindings/) for the full list.

## Set the default shell

New Panes use the system's default shell. Change it in either of these ways:

### In the window

Open **Settings › General** and type the shell's path into the **Shell** field of the **Terminal** group, such as `/bin/zsh` or `/opt/homebrew/bin/fish`.

### In the configuration file

Add this to `config.toml`:

```toml
[server.terminal]
shell = "/opt/homebrew/bin/fish"
```

## Shell integration

Condr loads a small script into the zsh, bash or fish that a new Pane starts. The script marks each prompt and each command, which gives you two things:

* **Jump between prompts**: `Cmd + ↑` / `Cmd + ↓` on macOS, `Ctrl + ↑` / `Ctrl + ↓` on Windows and Linux, while the shell waits at its prompt.
* **Long commands notify**: when a command that ran for 10 seconds or more finishes in a Pane you are not looking at, a system notification says so, with its exit code when it failed. Commands an Agent ran in stay quiet; the Agent's own notifications cover them.

Your own startup files still load as before: Condr's script runs alongside them and changes neither your prompt nor your settings. bash before 4.4, such as the one macOS ships, cannot mark when a command starts, so it gets no integration; Homebrew's bash does.

Turn it off under **Settings › General › Shell integration**, or with `shell_integration = false` under `[server.terminal]`. The change applies to new Panes. No shell on a Windows Device gets integration yet, PowerShell included.
