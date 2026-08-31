# Herdr v0.8.2: Workspace cwd and Git worktrees

Scope: Herdr tag [`v0.8.2`](https://github.com/herdrdev/herdr/tree/v0.8.2) (commit `9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c`) and the official Git worktree manual. Facts and Condr recommendations are separated below.

## Verified Herdr behavior

### Workspace and pane cwd

- `terminal.new_cwd` defaults to `follow`; the other policies are `home`, process `current`, or a configured path. `follow` uses the supplied source cwd, then `$HOME`, process cwd, and `/` as fallbacks. ([config model](https://github.com/herdrdev/herdr/blob/v0.8.2/src/config/model.rs#L228-L269), [resolver](https://github.com/herdrdev/herdr/blob/v0.8.2/src/app/creation.rs#L13-L30))
- A new Workspace follows the selected/active Workspace's focused Pane cwd and falls back to that Workspace's identity cwd. A new Tab follows its Workspace's focused Pane; a Split follows its target Pane. An explicit API `cwd` overrides this policy. ([Workspace creation](https://github.com/herdrdev/herdr/blob/v0.8.2/src/app/creation.rs#L84-L134), [Tab API](https://github.com/herdrdev/herdr/blob/v0.8.2/src/app/api/tabs.rs#L47-L67), [Split API](https://github.com/herdrdev/herdr/blob/v0.8.2/src/app/api/panes.rs#L32-L57))
- On Unix, the followed cwd prefers the PTY foreground process-group leader's usable cwd and falls back to the shell cwd. On non-Unix it uses the shell/reported cwd. The shell is spawned with that resolved cwd. ([runtime cwd](https://github.com/herdrdev/herdr/blob/v0.8.2/src/pane.rs#L2952-L2985), [shell spawn](https://github.com/herdrdev/herdr/blob/v0.8.2/src/pane.rs#L1731-L1769))
- A Workspace's effective identity is dynamic: it is the first Tab's root Pane cwd, with stored `identity_cwd` only as fallback. This effective cwd drives its automatic label and Git identity. ([identity resolution](https://github.com/herdrdev/herdr/blob/v0.8.2/src/workspace.rs#L1101-L1158))
- Non-Git directories are valid Workspaces. Git discovery returns optional metadata, while the label falls back to the directory name; Workspace construction does not require Git. ([optional discovery](https://github.com/herdrdev/herdr/blob/v0.8.2/src/workspace.rs#L63-L75), [constructor](https://github.com/herdrdev/herdr/blob/v0.8.2/src/workspace.rs#L397-L476))

### Git identity and presentation

- Herdr discovers a repository by walking upward for a Git directory/file. It derives a repo key from the common Git directory, a checkout key from the worktree root, and detects a linked worktree when its Git dir differs from its common dir. ([discovery types and derivation](https://github.com/herdrdev/herdr/blob/v0.8.2/src/workspace/git/discovery.rs#L3-L99), [upward search](https://github.com/herdrdev/herdr/blob/v0.8.2/src/workspace/git/discovery.rs#L249-L267))
- Workspace runtime state caches branch, ahead/behind, and Git-space metadata; the sidebar can render branch and ahead/behind. An uncustomized grouped child uses its branch as its label and strips a leading `worktree/`. ([Workspace fields](https://github.com/herdrdev/herdr/blob/v0.8.2/src/workspace.rs#L177-L205), [child label](https://github.com/herdrdev/herdr/blob/v0.8.2/src/ui/sidebar.rs#L284-L299), [sidebar tokens](https://github.com/herdrdev/herdr/blob/v0.8.2/src/ui/sidebar.rs#L1317-L1326))

### Managed worktrees

- Worktree actions require a parent Workspace inside a non-linked Git worktree; Herdr rejects starting create/open from a linked child. ([source validation](https://github.com/herdrdev/herdr/blob/v0.8.2/src/app/worktrees.rs#L14-L77))
- New worktrees default to branch `worktree/<generated-slug>` and checkout path `~/.herdr/worktrees/<repo>/<branch-slug>`; the user edits the branch, which recomputes the path. ([dialog defaults](https://github.com/herdrdev/herdr/blob/v0.8.2/src/app/worktrees.rs#L80-L124), [path config](https://github.com/herdrdev/herdr/blob/v0.8.2/src/config/model.rs#L833-L838), [default](https://github.com/herdrdev/herdr/blob/v0.8.2/src/config/model.rs#L1093-L1098), [path derivation](https://github.com/herdrdev/herdr/blob/v0.8.2/src/worktree.rs#L154-L156))
- If the local branch already exists, Herdr runs `git -C <repo> worktree add <path> <branch>`; otherwise it runs `git -C <repo> worktree add -b <branch> <path> <base>`. The TUI uses `HEAD` as base. It does not delete the branch when the worktree is removed. ([command choice](https://github.com/herdrdev/herdr/blob/v0.8.2/src/worktree.rs#L228-L305), [TUI submission](https://github.com/herdrdev/herdr/blob/v0.8.2/src/app/worktrees.rs#L566-L603))
- Open Existing uses `git worktree list --porcelain`, excludes bare and prunable entries, focuses an already-open checkout or creates a Workspace for it, and records explicit parent/child membership. ([listing command](https://github.com/herdrdev/herdr/blob/v0.8.2/src/worktree.rs#L404-L498), [open dialog](https://github.com/herdrdev/herdr/blob/v0.8.2/src/app/worktrees.rs#L154-L233), [open result](https://github.com/herdrdev/herdr/blob/v0.8.2/src/app/worktrees.rs#L344-L413))
- Sidebar grouping is based only on explicit `WorktreeSpaceMembership`, requires at least two members and a non-linked parent, and does not automatically group ordinary Workspaces that happen to share a Git repo. ([group construction](https://github.com/herdrdev/herdr/blob/v0.8.2/src/ui/sidebar.rs#L327-L430))

### Close, remove, and persistence

- Closing a linked child only closes its Workspace; closing the managed parent closes every Workspace in that explicit group. Close removes in-memory Workspace/terminal state and does not invoke Git or delete checkout files. ([close behavior](https://github.com/herdrdev/herdr/blob/v0.8.2/src/app/actions.rs#L1673-L1740), [close API](https://github.com/herdrdev/herdr/blob/v0.8.2/src/app/api/workspaces.rs#L298-L330))
- Remove is offered only for a Herdr-managed linked child. It first runs `git worktree remove <path>` without force; a dirty/untracked error changes the confirmation into a force confirmation, then it runs with `--force`. On success it closes the child Workspace. ([eligibility](https://github.com/herdrdev/herdr/blob/v0.8.2/src/app/worktrees.rs#L127-L151), [remove command](https://github.com/herdrdev/herdr/blob/v0.8.2/src/worktree.rs#L158-L184), [confirmation transition](https://github.com/herdrdev/herdr/blob/v0.8.2/src/app/worktrees.rs#L878-L958))
- This follows Git's contract: `worktree remove` accepts only clean worktrees unless `--force` is supplied; the main worktree cannot be removed. ([official Git manual](https://git-scm.com/docs/git-worktree#Documentation/git-worktree.txt-remove))
- The session snapshot stores Workspace identity cwd, explicit worktree membership, and each Pane cwd. On restore Herdr rediscovers branch/Git metadata and retains membership only if the checkout still exists and maps to the same repo key. ([snapshot fields and capture](https://github.com/herdrdev/herdr/blob/v0.8.2/src/persist/snapshot.rs#L49-L69), [Workspace capture](https://github.com/herdrdev/herdr/blob/v0.8.2/src/persist/snapshot.rs#L279-L307), [Pane cwd capture](https://github.com/herdrdev/herdr/blob/v0.8.2/src/persist/snapshot.rs#L311-L381), [restore validation](https://github.com/herdrdev/herdr/blob/v0.8.2/src/persist/restore.rs#L405-L443))

## Recommended Condr MVP interpretation

1. Keep `Workspace.root_cwd` stable at creation/open time; do not copy Herdr's dynamic “first root Pane cwd becomes Workspace identity” behavior. Pane cwd remains runtime state. This matches Condr's folder-oriented GUI and avoids a shell `cd` silently changing Workspace identity/grouping.
2. Support arbitrary directories. `Open Folder` sets `root_cwd` to the chosen folder; `New Terminal Workspace` uses the user home. New Tabs and Splits follow the focused/target Pane's current cwd when available, then fall back to `root_cwd`.
3. Treat Git as optional derived metadata. For MVP show only the current branch; defer dirty and ahead/behind status until it drives a concrete workflow.
4. Keep first-class create/open worktree because isolated checkouts are central to multi-agent work, but model explicit managed membership as Herdr does. Ordinary `Open Folder` must never grant delete authority merely because Git reports a linked worktree.
5. `Close Workspace` closes only the selected Workspace and never touches disk. Do not copy Herdr's parent-close cascade.
6. `Remove Worktree` is a separate action only for a Condr-created linked worktree. MVP should attempt clean `git worktree remove` only and surface Git's dirty/untracked refusal; omit force deletion and branch deletion until users explicitly require them.
7. Use one fixed portable checkout root, `<program>/worktrees/<repo>/<branch-slug>`, for MVP. Add a TOML override only when a real portability or storage-location need appears.

## Unresolved by this research

- Herdr supports bare-repository discovery, but Condr's MVP need for bare repositories is not established; defer it unless a target workflow requires it.
- Windows foreground-process cwd fidelity is limited to Herdr's shell/reported-cwd fallback. Condr must validate the corresponding ConPTY cwd signal on Windows rather than assume Unix foreground-process behavior.
