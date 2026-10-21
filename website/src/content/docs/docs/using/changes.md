---
title: Changes and file preview
description: Quickly see which files an Agent changed, compare code diffs (Diff) and preview project files in Condr.
---

When an Agent modifies or refactors your code, you need to quickly confirm which files changed and whether the changes are correct. Condr provides built-in views for file changes and previews and lets you make small edits to files directly, so you can review work without opening an external IDE.

## Open the changes panel (Changes)

- **Shortcut**: press `Cmd + Option + B` (macOS) or `Ctrl + Alt + B` (Windows/Linux).
- **From the interface**: click the sidebar button in the title bar of the right sidebar.

The Changes view sorts modified files into three groups by Git status:
- **Conflicts**: files with merge conflicts.
- **Tracked**: tracked files that have been modified.
- **Untracked**: files the Agent created that Git does not track yet.

Each file shows how many lines were added and deleted, and the top sums up the total number of changed lines.

## Compare against HEAD or the base branch

The button above the list names what the changes are compared against, such as `Against main`, and its menu switches it:

- **HEAD**: only what is not committed yet. Once an Agent commits, those files drop off the list.
- **The base branch**: the menu shows the branch name, such as `main`. The list holds everything this branch changed since it forked, committed or not, and none of the commits the base branch gained since.

When Condr creates a Worktree, it records the branch the parent Workspace was on as the base branch. Other Workspaces use the remote's default branch (such as `origin/main`), then a local `main`, then `master`. When the current branch is the base branch itself or its upstream, only HEAD is available.

Each Workspace remembers its own choice.

## View code diffs (Diff)

1. Click any file in the **Changes** list, and a **Diff** Tab opens in the center area automatically.
2. The page highlights exactly how the file differs from what you compare against, and the title bar names it, for example `Against main`.
3. **Split** shows the old and new file side by side; **Unified** shows them in one column. The choice in the title bar applies to every Diff Tab and is remembered.
4. Unchanged stretches are folded; click a fold to show more lines above or below it, or all of them.
5. Click **Show File** in the title bar to open the file in the Preview Tab at the line you last clicked in the diff.

> **Tip**: For very large binary files, or any single file over 1 MiB, the detailed comparison is hidden automatically to protect performance.

## Browse and preview project files (Files & Preview)

- **Switch to the file tree**: switch the tab at the top of the right sidebar to **Files** to expand and browse project folders as you would in a file manager.
- **Change markers**: a folder with changes carries a dot, and files ignored by `.gitignore` are grayed out automatically.
- **Click to preview**: click any text file to open it in the **Preview** Tab. Syntax highlighting and scroll position refresh automatically when the file changes on disk.
- **Rendered Markdown and images**: Markdown and SVG files open rendered by default; switch between **Preview** and **Source** in the Tab header. Images (PNG, JPEG, GIF, WebP, BMP, ICO and TIFF under roughly 2 MiB) scale automatically to fit, and animated images play automatically. Images in Markdown files are loaded locally from the project; web images display their alt text instead.

## Edit files (Preview)

The **Source** view of the Preview Tab is an editor, on local and remote Devices alike:

- **Edit and save**: type into the file, then press `Cmd + S` (macOS) or `Ctrl + S` (Windows/Linux). A dot in the Tab header marks unsaved changes. Nothing is saved automatically, so your edits never race the Agent's.
- **Line endings are kept**: a file that uses Windows line endings (`CRLF`) keeps them for the lines you add.
- **When the file changes on disk**: if the Agent writes the file while you have unsaved changes, your text stays as it is and the header says **Changed on disk**. Saving then asks whether to **Overwrite** the file with your text or to **Discard and Reload** it.
- **Unsaved changes are never dropped silently**: before you switch to another file, close the Tab or quit Condr, Condr asks whether to save. If another device switches or closes the Tab, you can still save your text, copy it or discard it.
- **Read-only files**: files that are not valid UTF-8 text, binary files and files over 1 MiB can be viewed but not edited; the header says why.

## Right-click menu

Right-click a file in the Changes or Files list to choose one of these common actions:

- **Open in ...**: open the current file or project in a locally installed VS Code, Zed, Cursor or IntelliJ IDEA.
- **Insert Path into Terminal**: paste the file's relative path into the most recently used terminal Pane, ready to pass to a command-line tool.
- **Copy Relative Path / Copy Path**: copy the file's relative or absolute path in one click.
- **Reveal in Finder / Reveal in File Explorer / Open Containing Folder**: show the file or folder in the system file manager (local Devices only).
- **New File… / New Folder…**: create an entry in the Files view: inside a folder, beside a file or at the project root from the empty area below the tree. A new file opens in the Preview Tab, ready to type.
- **Rename…**: rename a file or folder within its folder. A name that is already taken is refused.
- **Delete…**: delete a file or a folder with everything in it, after you confirm. Deletion is permanent and skips the system trash.

Condr never changes anything inside `.git`.
