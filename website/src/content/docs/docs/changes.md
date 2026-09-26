---
title: Changes and files
description: See an Agent's file changes and project files inside Condr.
---

Open the right sidebar to review an Agent's changed files, diffs, and project files without leaving Condr.

## Find the four views

| Place | Where | Shows | Open it by |
| --- | --- | --- | --- |
| **Changes** | Right sidebar | Which files changed | The sidebar button in the title bar, or its shortcut |
| **Files** | Right sidebar | What's in the project | The same, then the Files tab |
| **Diff** Tab | Tab strip | What changed in one file | Clicking a file in Changes |
| **Preview** Tab | Tab strip | The contents of one file | Clicking a file in Files |

All four views are read-only and work for remote Workspaces.

Open the right sidebar with the **Show Changes & Files** button in the title bar, or press Cmd+Alt+B on macOS or Ctrl+Alt+B. A Git repository opens on Changes. Any other folder opens on Files. Each Workspace remembers whether the sidebar is open. Drag its left edge to resize it.

## Review changed files

Changes lists every working-directory difference from HEAD in three groups: **Conflicts**, **Tracked**, and **Untracked**. Staged and unstaged changes are not separate. Each file has one status.

Files appear in a directory tree. A chain of single directories folds into one row. Each file shows its status and added and removed lines. Deleted files are struck through. The tab number counts changed files. The end of the tab shows total added and removed lines.

When there are more than 1000 changes, Condr lists only the first 1000. This usually means build output is not ignored. Add a `.gitignore`.

## Review a file diff

Click a file in Changes to open a **Diff** Tab in the Workspace. Clicking another file reuses that Tab. The diff compares with HEAD, uses the terminal font, shows line numbers, and supports search.

The **Show File** button in the header opens Preview at the line under the cursor. Deleted files do not have this button.

A file larger than 1 MiB on either side has no diff. A binary file shows "Binary file".

## Browse and preview files

Files loads the directory tree as you open it. A directory with changes below it has a dot. Ignored files are dimmed. Changed files use their status color.

Click a file to open it in the **Preview** Tab. Preview shows text files only, highlights them by extension, and supports files up to 1 MiB. When a file changes on disk, Preview refreshes and keeps its scroll position. Images and other binary files show "Binary file".

Right-click a file in either view for:

- **Show File**: available only in Changes and opens Preview.
- **Copy Relative Path** and **Copy Path**: copy the relative path or the absolute path on that Device.
- **Open in …**: open the file in an external editor. Local Devices only.
- **Insert Path into Terminal**: paste the relative path and a space into the terminal this Workspace last used. Condr quotes it when needed.

## Open a file in an external editor

The **Open in** button in the title bar opens the Workspace root. Condr finds installed Zed, VS Code, Cursor, and IntelliJ IDEA copies. It always offers Finder, Explorer, or your file manager last. Condr remembers the choice per repository, and its worktrees share the choice.

Add another editor in the configuration file:

```toml
[[client.editors]]
name = "Helix"
command = ["hx"]
```

The path is appended as the last argument. See [Configuration and settings](/docs/configuration/).

The button works only for local Devices. Remote Workspace files stay on the remote machine, so your local editor cannot open them.

## Know the limits

- Condr cannot edit files. Use an external editor, or ask the Agent.
- Diff compares only with HEAD, not another branch.
- Condr cannot stage, commit, or discard changes.
- Preview cannot show images.
