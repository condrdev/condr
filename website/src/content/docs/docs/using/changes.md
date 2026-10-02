---
title: Changes and file preview
description: Quickly see which files an Agent changed, compare code diffs (Diff) and preview project files in Condr.
---

When an Agent modifies or refactors your code, you need to confirm quickly which files it changed and whether the changes are right. Condr has built-in read-only views for file changes and previews, so you can review the work without opening an external IDE.

## Open the changes panel (Changes)

- **Shortcut**: press `Cmd + Option + B` (macOS) or `Ctrl + Alt + B` (Windows/Linux).
- **From the interface**: click the sidebar button in the title bar of the right sidebar.

The Changes view sorts modified files into three groups by Git status:
- **Conflicts**: files with merge conflicts.
- **Tracked**: tracked files that have been modified.
- **Untracked**: files the Agent created that Git does not track yet.

Each file shows how many lines were added and deleted, and the top sums up the total number of changed lines.

## View code diffs (Diff)

1. Click any file in the **Changes** list, and a **Diff** Tab opens in the center area automatically.
2. The page highlights exactly how the file differs from `HEAD` on the current branch.
3. Click **Show File** in the title bar to jump straight to a preview of the file's full content.

> **Tip**: For very large binary files, or any single file over 1 MiB, the detailed comparison is hidden automatically to protect performance.

## Browse and preview project files (Files & Preview)

- **Switch to the file tree**: switch the tab at the top of the right sidebar to **Files** to expand and browse project folders as you would in a file manager.
- **Change markers**: a folder with changes carries a dot, and files ignored by `.gitignore` are grayed out automatically.
- **Click to preview**: click any text file to open a read-only preview in the **Preview** Tab. The syntax highlighting and scroll position refresh automatically when the file changes on disk.

## Right-click menu

Right-click a file in the Changes or Files list to choose one of these common actions:

- **Open in ...**: open the current file or project in a locally installed VS Code, Zed, Cursor or IntelliJ IDEA.
- **Insert Path into Terminal**: paste the file's relative path into the most recently used terminal Pane, ready to pass to a command-line tool.
- **Copy Relative Path / Copy Path**: copy the file's relative or absolute path in one click.
