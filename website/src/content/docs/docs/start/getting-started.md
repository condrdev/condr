---
title: Getting started
description: Install Condr and run and manage multiple Agent instances from one workspace.
---

Condr is a workbench for managing and orchestrating Agent command-line tools such as Claude Code and Codex in one place. Whether an Agent runs locally or on a remote server, Condr lets you monitor it and work with it from a single view.

Condr comes in two forms: the **desktop app** and the **Headless Server**.

:::caution
Condr 0.1 is currently in Public Preview. Features and configuration options may keep changing in later releases.
:::

## Desktop app (recommended)

Go to the [download page](/download/) to get the package for your platform, then unpack or install it and launch it.

The desktop app bundles the Server component and starts it with the app by default, so no extra setup is needed:

1. **Create a Workspace**: click **Open Project** and choose a local project directory. Condr creates a Workspace and opens a terminal automatically.
2. **Install Agent integrations**: go to **Settings › Agent integrations** and click **Install** next to each Agent you want supported, so that Condr can capture and track its state.
3. **Run an Agent**: run the Agent command (for example `claude`) directly in the built-in terminal.

## Headless Server

For cloud hosts, GUI-less dev machines or remote servers, only the core `condr` CLI needs to be installed:

**Linux / macOS:**
```sh
curl -fsSL https://condr.dev/install.sh | sh
```

**Windows:**
```powershell
irm https://condr.dev/install.ps1 | iex
```

After installation, **reopen your terminal window** to pick up the updated environment variables, then start the service:

```sh
condr server start
```

:::note
- The Server keeps running in the background; closing the terminal does not stop it.
- To connect to this machine, see [Remote connections](/docs/using/remote/).
- The current preview does not register a system startup service yet, so after a reboot you need to run the start command above again by hand.
:::

## Dig deeper

- [Core concepts](/docs/start/concepts/): learn the architecture model of Device, Server, Workspace, Pane and Agent.
- [Workspaces, Tabs and Panes](/docs/using/workspaces/): set up split-Pane layouts and give each Agent its own branch environment with Git Worktree.
- [Agent integrations](/docs/using/agents/): see the list of compatible Agents, lifecycle Hooks and the state-tracking definitions.
- [Remote connections](/docs/using/remote/): reach remote devices over SSH, TCP or P2P.
- [Agent automation](/docs/using/automation/): coordinate Agents with each other through the `condr` CLI.
- [CLI reference](/docs/reference/cli/): the complete manual of commands and options.
- [Troubleshooting](/docs/help/troubleshooting/): common connection drops, state drift and how to diagnose them.
