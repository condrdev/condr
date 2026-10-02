---
title: Getting started
description: Install Condr, run your first Agent, and connect a remote Device in ten minutes.
---

Install Condr, run one Agent, and connect a second Device by following these steps.

Condr has two parts:

- **Server**: the `condr` command. It runs your Agents and keeps working after you close the window.
- **Window**: the `condr-gui` app. It is the interface you use. One window can show several Devices.

:::caution
Condr 0.1 is a public preview, and things may still change between versions. Update the window and the Server together.
:::

## 1. Install the desktop app

Download the installer for your platform from the [download page](/download/) and install it on the computer you work at. It includes the Server.

| Platform | Package |
| --- | --- |
| Linux x86_64 / arm64 | AppImage |
| macOS x86_64 / arm64 | `.dmg` |
| Windows x86_64 | `.exe` |

Windows may show SmartScreen, because the preview installer is not signed yet. See [Install](/docs/install/) for platform notes, install script options, and the headless Server.

## 2. Run your first Agent

1. Install the Agent CLI you want and sign in, for example Claude Code or Codex. Condr does not provide models or accounts.
2. Open Condr. It starts the local Server.
3. Click **Open Project**, choose a project folder, and let Condr open a Workspace with a shell.
4. Let Condr read the Agent's state. In that shell, run this command once:

   ```sh
   condr agent hooks install claude
   ```

   Replace `claude` with your Agent. Install hooks once for each Agent.
5. In the same shell, run the Agent as usual, for example `claude`. The sidebar shows its state: amber while it works and red when it waits for your approval.
6. Give it a task, then close the window. The Server and the Agent keep running.
7. Open Condr again. Your Workspace is where you left it. If the Agent finished, the sidebar marks it green.

You can now use Condr. To run several Agents, choose **Split Right** in the Pane menu. You can also right-click the Workspace and choose **Create Worktree** to give each Agent its own branch. See [Workspaces, Tabs, and Panes](/docs/workspaces/).

## 3. Connect a remote Device

Run Agents on another machine and watch them in the same window:

1. Install the headless Server on that machine:

   ```sh
   curl -fsSL https://condr.dev/install.sh | sh
   ```

   On Windows, use `irm https://condr.dev/install.ps1 | iex`.
2. On your computer, open Condr and click **Connect Remote Device** at the bottom of the sidebar. Choose **SSH address**, then enter the address you use to log in with SSH:

   ```text
   user@build-box
   ```

3. The machine appears in the sidebar. Click **New Workspace**. From there, it works like the local Device.

SSH uses your existing OpenSSH configuration and keys. For a machine without SSH, connect over TCP or Peer-to-peer. See [Remote devices](/docs/remote/).

## Next steps

- [Agents](/docs/agents/): read each state and see which Agents are supported.
- [Agent automation](/docs/automation/): let one Agent direct others.
- [Keyboard shortcuts](/docs/keybindings/).
- [Troubleshooting](/docs/troubleshooting/): find the first checks when something goes wrong.
