---
title: Workspaces, Tabs and Panes
description: Open projects, give Agents isolated branches with Git worktrees, and arrange your terminal layout.
---

A Workspace is the basic unit Condr uses to manage a project. Each Workspace maps to one project folder, where you can run several terminals or Agents in split Panes and multiple Tabs.

## Open a project (Workspace)

- **From the interface**: click **New Workspace** in the sidebar, or **Open Project** on the Welcome page.
  - **Local project**: pick the folder directly.
  - **Project on a remote server**: enter the absolute path of the folder on the remote server.
- **From the command line**: run `condr workspace create --cwd ~/code/my-app` in a terminal.

### Read the project status

The project list in the sidebar shows each project's status directly:

- **Project name**: the folder name by default. Right-click → **Rename Workspace** to rename it.
- **Git status**: the second line shows the current branch and how many commits it is ahead of and behind its upstream, for example `main ↑1 ↓2`. Outside a Git repository it shows `no git`.

## Split and organize the terminal view

### Tabs

- **New**: click the **+** at the top to open a new Tab. Each Tab inherits the current working directory.
- **Organize**: right-click a Tab → **Rename Tab** to rename it, or press and hold a Tab to **drag it into a new order**.

### Split layout (Panes)

Click the menu in the top-right corner of a Pane:

- **Split**: choose **Split Right** (side by side) or **Split Down** (one above the other).
- **Zoom to focus (Zoom)**: choose **Toggle Zoom** to let the current Pane fill the whole window for a while, and choose it again to restore the layout (handy for reading long logs).
- **Rearrange**: press and hold a Pane's title bar and **drag** it to swap places with a neighboring Pane or regroup the layout.

## Tips for working efficiently

### Send a screenshot to a remote Agent

When you run an Agent on a remote server, copy a screenshot locally and press the shortcut directly in the remote terminal:

- macOS: Option+V
- Windows / Linux: Alt+V

Condr uploads the image to the server automatically and pastes its remote path into the command line, ready for the Agent to read.

### Open links in the terminal

Hold the key and click a URL in the terminal to open it in your default browser:

- **macOS**: `Cmd + left click`
- **Windows / Linux**: `Ctrl + left click`

### Force text selection

While `vim` or another terminal program that captures the mouse is running, hold `Shift` and drag with the mouse to force a selection and copy the terminal text.
