---
title: Workspaces, Tabs and Panes
description: Open projects, give Agents isolated branches with Git worktrees, and arrange your terminal layout.
---

A Workspace is the basic unit Condr uses to manage a project. Each Workspace maps to one project folder, where you can run several terminals or Agents in split Panes and multiple Tabs.

## Open a project (Workspace)

- **From the interface**: click **New Workspace** in the sidebar and choose one of three ways; the Welcome page lists the same three.
  - **Open Project**: an existing folder. Locally you pick it directly; on a remote server you enter its absolute path.
  - **Clone Repository**: paste the repository URL, check where it goes and what the folder is called, and click **Clone**. Condr makes the folder, opens the Workspace and types `git clone` into its first terminal once the shell is ready. The clone runs there with that machine's own git, keys and credentials, so you answer any password or host-key question in the terminal; press Ctrl+C to stop it.
  - **New Directory**: an empty folder, made on the machine the Workspace belongs to.
- **From the command line**: run `condr workspace create --cwd ~/code/my-app`, or `condr workspace clone <url> [<directory>]` to clone into a new folder.

### Read the project status

The project list in the sidebar shows each project's status directly:

- **Project name**: the folder name by default. Right-click → **Rename Workspace** to rename it.
- **Git status**: the second line shows the current branch and how many commits it is ahead of and behind its upstream, for example `main ↑1 ↓2`. Outside a Git repository it shows `no git`.
- **Fold a device**: hover a device's heading and click the arrow before its name to hide its projects. A folded device counts its Agents by state beside its name, stays folded after a restart, and lights up while the project you are looking at is inside it.

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

When you run an Agent on a remote server, copy a screenshot locally and press the same key you paste images into a local Agent with. It follows the computer you are on, not the remote Device:

- macOS / Linux: Ctrl+V
- Windows: Alt+V

Condr uploads the image to the server automatically and pastes its remote path into the command line, ready for the Agent to read.

### Open links in the terminal

Hold the key and click a URL in the terminal to open it in your default browser:

- **macOS**: `Cmd + left click`
- **Windows / Linux**: `Ctrl + left click`

### Force text selection

While `vim` or another terminal program that captures the mouse is running, hold `Shift` and drag with the mouse to force a selection and copy the terminal text.

### Images in the terminal

Programs that draw with the kitty graphics protocol show their images right in the Pane: yazi's previews, `kitten icat`, viu, OpenCode's image preview, image.nvim, snacks.nvim, presenterm, timg and anything built on ratatui-image. Images scroll with the text, animations play, and they show on remote Devices and in the phone app too.

A few programs decide by environment variables alone and need to be told:

- **chafa**: `chafa -f kitty`
- **Codex**: draws no images in Condr yet; it only looks for kitty, Ghostty or WezTerm in its environment.

Sixel and iTerm2's image protocol are not supported. On Windows, programs run behind the system's console layer (ConPTY), which never passes the protocol's answers back, so most programs there decide images are unsupported and fall back to text.
