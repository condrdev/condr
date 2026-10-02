---
title: Agent integrations
description: Use command-line Agents in Condr and see what state each one is in.
---

When several Agents work on your code at the same time, the key to keeping the work flowing is knowing whether each command-line Agent (such as Claude Code or Codex) has finished its task in the background or is waiting for your approval. Condr gives you clear state monitoring and notifications.

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

## Sidebar state lights

Once the Hook is installed, the sidebar automatically shows the live state of the Agent in each Pane:

- 🟢 **Green (Idle)**: the Agent has finished the prompt and is standing by, or its task is done.
- 🟡 **Yellow (Working)**: the Agent is thinking, running tools or generating code.
- 🔴 **Red (Blocked)**: the Agent has hit a permission request or a question that needs a person, and is waiting for your reply.
- ⚪ **Gray (Unknown)**: no Hook is installed, or the Agent has just started and has not reported a state yet.

When you are in another app's window and an Agent needs your approval, Condr sends a desktop notification. Click the notification to bring Condr forward and jump straight to that Pane.

## Notes on configuring common Agents

- **Codex**: after installing the Hook, run the `/hooks` command inside Codex and confirm that you trust it.
- **Antigravity**: after installing, turn on the Condr plugin in its settings.
- **Pi / Oh My Pi**: if you use several profiles, install the Hook once for each profile.
