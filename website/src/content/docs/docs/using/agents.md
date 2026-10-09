---
title: Agent integrations
description: Use command-line Agents in Condr and see what state each one is in.
---

When several Agents work on your code at the same time, the key to keeping the work flowing is knowing whether each command-line Agent (such as Claude Code or Codex) has finished its task in the background or is waiting for your approval. Condr gives you clear state monitoring and notifications.

## Support levels

Condr supports only the Agents it ships an integration for, in two levels. The level depends on what the Agent's Hooks can report. If a CLI you use is not on the list yet, open an [Issue](https://github.com/condrdev/condr/issues) or a pull request.

### Fully supported

A fully supported Agent's Hooks report a turn starting and the main Agent's turn ending, told apart from a subagent's, so its Working and Idle states can be trusted. Claude Code, Codex, OpenCode, Pi, Oh My Pi, Antigravity CLI, Grok Build, Cursor CLI and GitHub Copilot CLI are fully supported.

Cursor CLI's and Antigravity CLI's Hooks have no event for a permission prompt, so they do not show Blocked while waiting for approval. Antigravity CLI still shows Blocked when it asks you a question.

### Recognition only

Condr recognizes a recognition-only Agent from its process: the sidebar shows its icon, the icon goes away when the process exits, and `condr agent start` can launch it. Its Hooks cannot report a state worth trusting, so Condr installs none and its state stays Unknown. Without a session ID from a Hook, its conversation is not resumed after a Server restart.

`condr agent wait` and `condr agent prompt --wait` refuse a recognition-only Agent at once with `agent_reports_no_state` instead of waiting out their timeout. `condr agent prompt` without `--wait` still delivers the prompt.

Kimi Code is the only one so far, because its Hooks cannot tell a subagent's Stop from the main Agent's. **Settings › Device › Agent integrations** shows it as Recognition only.

## Prepare and check Agents

Before you run an Agent in Condr, make sure the Condr Server can find the matching command-line tool.

- **Check available Agents**: run `condr agent available` in a terminal, and Condr lists every supported Agent tool it currently finds in the `PATH` environment variable.
- **How `PATH` is inherited**: the Condr Server inherits the system `PATH` when it starts, but it does not read aliases or custom paths from shell configuration files such as `.zshrc`.
- **If a command is not found**: install the Agent into the system-wide `PATH` (such as `/usr/local/bin`), or run `condr server restart` from your terminal environment to restart the Server.

## Install Hooks for accurate states

Traditional terminal managers match screen text against regular expressions, so a change in the output easily leads them to the wrong state. Condr uses the Agents' native Hook mechanism instead: the Agent reports its own state, which makes Condr's view of it 100% accurate.

- **Install a Hook in one step**: run `condr agent hooks install <agent>` (for example `condr agent hooks install claude`).
- **Manage Hooks in the GUI**: go to **Settings › Device › Agent integrations** and install them there.
- **Check and uninstall**: run `condr agent hooks status <agent>` to see the status. To remove a Hook, run `condr agent hooks uninstall <agent>`.

## Agent state indicators

Once the Hook is installed, the sidebar automatically shows the live state of the Agent in each Pane:

| State | Meaning |
| :--- | :--- |
| **Working** | Running a task (thinking, generating text or calling tools). |
| **Blocked** | Waiting for your input or approval (listed in Needs you). |
| **Done** | Finished in the background. Resets to Idle once focus returns to its Pane. |
| **Idle** | Idle: the turn is over and it waits for the next instruction. |
| **Unknown** | A plain shell session, or an Agent without Condr hooks. |
| **Bell** | The terminal rang its bell (Bell takes priority over the regular lifecycle states). |

> **Tip**: when an Agent in the background finishes or becomes blocked, Condr sends a native system notification; clicking it goes straight to that Pane.

## Notes on configuring common Agents

- **Codex**: after installing the Hook, run the `/hooks` command inside Codex and confirm that you trust it.
- **Antigravity**: after installing, turn on the Condr plugin in its settings.
- **Pi / Oh My Pi**: if you use several profiles, install the Hook once for each profile.
