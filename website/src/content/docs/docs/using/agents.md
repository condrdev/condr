---
title: Agent integrations
description: Use command-line Agents in Condr and see what state each one is in.
---

When several Agents work on your code at the same time, the key to keeping the work flowing is knowing whether each command-line Agent (such as Claude Code or Codex) has finished its task in the background or is waiting for your approval. Condr gives you clear state monitoring and notifications.

## Support levels

Condr supports only the Agents it ships an integration for, at two levels depending on what the Agent's Hooks report. If a CLI you use is not supported yet, open an [Issue](https://github.com/condrdev/condr/issues) or a pull request.

### Fully supported

A fully supported Agent's Hooks report when a turn starts and ends, telling the main Agent apart from subagents, so Working and Idle are reliable. Claude Code, Codex, OpenCode, Pi, Oh My Pi, Antigravity CLI, Grok Build, Cursor CLI and GitHub Copilot CLI are fully supported.

Cursor CLI and Antigravity CLI do not emit events for permission prompts, so they do not enter the Blocked state when awaiting user approval. Antigravity CLI still enters Blocked when waiting for an answer to a question.

### Recognition only

Condr detects recognition-only Agents by their process: the sidebar displays the Agent's icon while running, removes it upon exit and allows starting it via `condr agent start`. Their Hooks cannot report a reliable state, so Condr installs none and their state stays Unknown. With no session ID from a Hook, their conversations do not resume after a Server restart.

For these Agents, `condr agent wait` and `condr agent prompt --wait` exit immediately with `agent_reports_no_state` rather than waiting for a timeout. Running `condr agent prompt` without `--wait` delivers prompts normally.

Kimi Code is currently the only Agent in this category, as its Hooks cannot distinguish subagent completion events from the main Agent's. It is labeled as Recognition only in **Settings › Agent integrations**.

## Prepare and check Agents

Before you run an Agent in Condr, make sure the Condr Server can find the matching command-line tool.

- **Check available Agents**: run `condr agent available` in a terminal, and Condr lists every supported Agent tool it currently finds in the `PATH` environment variable.
- **How `PATH` is inherited**: the Condr Server inherits the system `PATH` when it starts, but it does not read aliases or custom paths from shell configuration files such as `.zshrc`.
- **If a command is not found**: install the Agent into the system-wide `PATH` (such as `/usr/local/bin`), or run `condr server restart` from your terminal environment to restart the Server.

## Install Hooks for accurate states

Traditional terminal managers match screen text against regular expressions, so a change in the output easily leads them to the wrong state. Condr uses the Agents' native Hook mechanism instead: the Agent reports its own state, which makes Condr's view of it 100% accurate.

- **Install a Hook in one step**: run `condr agent hooks install <agent>` (for example `condr agent hooks install claude`).
- **Manage Hooks in the GUI**: go to **Settings › Agent integrations** and install them there.
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
