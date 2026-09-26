---
title: Workspaces, Tabs, and Panes
description: Open projects, give Agents separate branches, and arrange terminals.
---

Open a project, run several Agents on separate branches, and learn what closing or restarting does.

## Open a project

Click **New Workspace** beside a Device in the sidebar, or click **Open Project** on the Welcome page. Choose a folder. The local Device opens the system folder picker. A remote Device asks for an absolute path on that machine and lists directories below it for checking.

Condr opens the Workspace and starts a shell in its first Tab. The Workspace name comes from the folder. Right-click it and choose **Rename Workspace** to change the name.

A Workspace is a folder and the terminals you open there. Its root stays fixed when you run `cd` in a shell. For a Git repository, the sidebar shows the current branch and the commits ahead of and behind upstream on a second line.

You can also create one from a terminal:

```sh
condr workspace create --cwd ~/code/app --label app
```

## Give each Agent its own branch

Two Agents that edit one working directory can overwrite each other. A Git worktree gives each Agent another directory on its own branch.

Right-click a Git repository Workspace, choose **Create Worktree**, and enter a branch name. The default is `worktree/` followed by the Workspace name. If the branch does not exist, Condr creates it from the current HEAD. If it exists, Condr checks it out.

Condr places the new directory at `<repo>.worktrees/<branch>` beside the repository and adds it as a Workspace in the sidebar. Set `[server] worktree_root` in the Server configuration to use another location. See [Configuration and settings](/docs/configuration/).

Choose **Open Existing Worktree** to use a worktree you already have. Condr never deletes one opened this way.

To delete a worktree Condr created, right-click it and choose **Remove Worktree**. Condr deletes the directory and keeps the branch. If the directory has uncommitted or untracked files, Condr refuses. Clean it up first.

The command line cannot create worktrees yet. `condr workspace list` reports which Workspace is a worktree of which.

## Use Tabs to separate work

A Tab is one screen in a Workspace. Click **+** in the tab strip to add one. Its shell starts in the directory of the Pane you are viewing. Drag Tabs to reorder them. Right-click a Tab for **Rename Tab** or **Close Tab**. An unnamed Tab shows only its number.

Closing the last Tab closes the Workspace, so Condr asks you to confirm.

## Split and arrange Panes

A Pane is one terminal. Open its **⋮** menu to choose:

- **Split Right** and **Split Down**: open a shell in the same directory at a 50/50 split.
- **Swap**: exchange places with the neighbor in a direction.
- **Toggle Zoom**: fill the Tab with this Pane. Click again to restore it. The header also has a zoom button.
- **Close Pane**.

Drag a Pane header onto another Pane. Drop on an edge to move it there, or in the center to swap the two. Drag the divider between Panes to resize them. These actions do not work while a Pane is zoomed.

The active Pane has a blue border. See [Keyboard shortcuts](/docs/keybindings/) for shortcuts.

## Use terminal features

Every Pane is a real terminal built on the alacritty core. `TERM` is `xterm-256color` with true color. Scrollback holds 10,000 lines.

**Select and copy.** Drag to select. Double-click a word or triple-click a line. When a program enables mouse mode, hold Shift while dragging. The Server keeps the selection, so it follows output and clears when you resize the window or switch screens. Copying ends the selection. Condr does not copy when you select.

**Open links.** Hold Cmd on macOS or Ctrl elsewhere and click a URL. The URL opens in your browser. Hover to see its full address.

**Paste images.** With a remote Device, press Alt+V, or Option+V on macOS, to upload a clipboard image to that machine. Paste the file path into the terminal to give the image to a remote Agent. PNG, JPEG, GIF, WebP, and BMP work, up to 16 MiB. The image stays in the Server runtime directory and is deleted when you disconnect. You do not need this on a local Device. Drag the file into the terminal instead.

**Set terminal properties.** A program can set the window title, write the clipboard with OSC 52, send OSC 8 hyperlinks, and change foreground, background, and palette colors. Condr sends OSC 52 clipboard content to every connected window.

**Not supported yet.** Condr does not support terminal search, block selection, the kitty keyboard protocol, Sixel, kitty graphics, or reading the clipboard through OSC 52.

## What happens when you close something

Closing a Pane ends every program it started. Condr sends SIGHUP, then SIGTERM, then SIGKILL. On Windows, each Pane has its own Job Object, and closing it ends the whole Job. Processes started with `setsid` or `nohup` on Unix, or with `CREATE_BREAKAWAY_FROM_JOB` on Windows, survive because they detached on purpose.

Closing the last Pane in a Tab closes that Tab. Closing the last Tab in a Workspace closes that Workspace. Condr asks for confirmation only when the close would also close the Workspace. Closing a Workspace stops its terminals and never deletes files.

When a shell exits on its own, the Pane keeps its last screen and is marked as exited. Condr ends the other programs it started in the same way.

## Keep two windows independent

Each window chooses its own Workspace and Tab. Switching in one window does not switch the other, even when both use the same Server.

`condr workspace focus` and `tab focus` are one-time “look here” commands. They switch every connected window, but do not create shared state.

A Server accepts control from one window at a time. When a second window connects, its tab strip shows “Viewing only”. It can watch and copy, but cannot type. When the first window disconnects, the second takes control.

## What survives a Server restart

After each change, the Server saves the structure to a snapshot file. A restart restores:

- **Kept**: every Workspace, Tab, and Pane, plus each Pane's directory, split ratios, focus, and worktree links.
- **Lost**: terminal contents and scrollback, running programs, and Agent state.

A restart ends the programs in every Pane. Condr opens a fresh shell in each Pane's previous directory. If that directory is gone, it uses the Workspace root. If the root is also gone, it removes the Pane.

Agents with hooks installed resume. Condr records their native session id and types the Agent's own resume command in the same Pane after the restart, such as `claude --resume <id>`. The conversation continues where it stopped. If resume fails, the command stays in the terminal for you to retry.

Run `condr server restart` in a terminal outside Condr, or click **Restart Condr** in **Settings › Device › Daemon**, to restart the Server. When the window restarts, it also restores its position, sidebar widths, and the Tab shown in each Workspace.
