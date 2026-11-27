# Git Graph Shows Every Branch as IntelliJ's Log Does

Status: accepted 2026-11-27. Supersedes ADR 0046's exclusion of a commit graph and of branches, tags and remotes other than `HEAD`'s history; renames ADR 0046's History, which ADR 0073 and ADR 0076 also name.

History (ADR 0046) lists `HEAD`'s commits two lines each in a sidebar 400 px wide, with no graph and no other branch. Reviewing what Agents did across branches and Managed Worktrees asks for more: which branch a commit is on, where branches fork and merge, who wrote what and when. IntelliJ IDEA's Git Log answers that with a branch tree, a commit table with a graph, and the selected commit's files and details. It is the reference here: its defaults, as intellij-community's `master` has them in November 2026, are copied unless this decision says otherwise. Condr still only reads: nothing here checks out, resets, reverts, cherry-picks, fetches or creates anything.

## Names

**History becomes Git History**, "Git 历史" in Chinese: "History" alone says too little beside a terminal's history and a shell's. The new viewer is **Git Graph**, in both languages. Code follows the names: the GUI's `history` module, state, `t!` keys and debug selectors become `git_history`, the core's `git/history.rs` becomes `git/log.rs` and `HISTORY_PAGE` `GIT_LOG_PAGE`. Earlier ADRs keep their words.

## The Git Graph Pane

**A viewer Pane** (ADR 0078), at most one per Workspace, in a Tab of its own when it first opens, opened through `ShowViewer` by a button beside Refresh in the right sidebar's header while Git History shows (Lucide `git-graph`), and by "Open Git Graph" in the Command Palette (ADR 0064); it has no default shortcut. Only a Workspace that has Git History offers it. The Session holds only that the Pane exists; the filters, the selection, column widths and split positions are this Client's view and are not remembered. With several repositories (ADR 0073) it shows the one Git History shows, through the same choice, as a menu heading the branch tree, since the repository scopes the branches and the commits alike; the "Branch or tag" field moves under it. The menu names each repository alone, without its branch, in Git History too. Opening Git Graph closes the Workspace's right sidebar, whose Git History it shows more of.

**Three columns, as IDEA lays them out**: the branches; the commits; and the selected commit's files above its details. The branches start 180 px wide and the right column 300 px, so the commits take the rest; the details start 240 px high under the files; every divider drags. Git History in the sidebar stays as it is but for the button that opens Git Graph.

### Branches

The tree has IDEA's groups, each shown only when it has entries: `HEAD` with the current branch, Local, Remote with a node per remote, and Tags. Names group by `/` into folders. The current branch comes first, then folders, then names. A "Branch or tag" field above the tree filters it as typed. A single click selects; a double-click filters the commits to that branch, `HEAD` included, as the Branch filter does. One branch at a time. The tree starts folded but for the folders on the way to the checked-out branch; a click toggles a folder against that. The tree's rows take no keyboard, so IDEA's Enter there is left out.

### Commits

**Every branch and tag, and `HEAD`, by default**, as IDEA opens its log. The order is IDEA's default "By Commit Date", which is `git rev-list --date-order`: no parent before all its children, otherwise newest commit time first, so the graph never draws an edge upwards when commit times disagree with ancestry. Git History keeps ADR 0046's commit-time order: that walk stops at its page, where the date-order walk first visits everything the tips reach (measured below), and Git History draws no graph to break.

**The table is GPUI Kit's `DataTable`**: virtualized, asking the Server for the next page as its end scrolls into view rather than with "Show more", which Git History keeps. Its columns are the graph and the subject with the commit's refs at the subject's right end, the author with `*` when the committer is someone else, the date, and the short id; IDEA hides the id by default, Condr shows it as the screenshot this was asked from does. Kit's columns have fixed widths, so the subject is sized to the table's width less the other three, measured as it draws; a narrow table keeps 160 px of subject and scrolls sideways, and in a cramped row the labels shrink to their icons before the subject gives up its room. Columns resize by hand until the table's width changes again; they do not sort, since the order is the graph's, and their widths are not kept.

**The graph is drawn per row in the subject cell.** A lane's colour comes from the name of the first ref at its head, hashed to one of the theme's six colours, which the terminal scheme gives (ADR 0062), so a branch keeps its colour and the graph sits in the palette around it; a lane with no ref takes its index. Lanes whose parents are not loaded yet run off the bottom of the table. As IDEA highlights them by default: a merge commit's row is grey, and the person's own commits, whose author email is the repository's `user.email`, are bold. Unlike IDEA, commits reachable from `HEAD` are not tinted: the table keeps the background of the columns beside it, and a selected row the selection colour of every other list.

**Refs are labels**, at the subject's right end: the refs on one commit merge into one label, `origin & main`; a tag shows as its icon alone; `HEAD` is yellow, local branches green, remote ones violet, as in IDEA.

**Dates are the author's**, as IDEA formats them: "N minutes ago" within the hour, "Today 14:25", "Yesterday 14:25", and otherwise `2026/11/25 14:25` in local time, where IDEA uses the system's format. Local time needs a time zone database, so `chrono`, which GPUI Kit already builds with its clock, becomes the GUI's sixth declared dependency on Kit's version.

**The filters**, in IDEA's order above the commits, with Refresh at the far end; each set filter shows its value and a clear button. The Server applies them all as it walks, so pages count matches.

- **Text or hash** applies on Enter or when the field loses focus, not per keystroke. It matches the full message, ignoring case, and a word of 7 to 64 hex digits also matches a commit by id prefix.
- **Branch** starts at `HEAD`, as IDEA's log does, so the log follows a checkout; its clear button shows every branch. Its menu has All, `HEAD`, then a submenu each for the local branches, every remote and the tags, folded by `/`. A double-click in the tree sets it too.
- **User** keeps one author, by email, ignoring case. Its menu has All, Me, the repository's `user.email`, and everyone who authored a commit this view has loaded. IDEA's Select…, typing several names, is left out.
- **Date** keeps commits by author date: All, Last 24 hours, Last 7 days, or Select…, a calendar of two months picking the first and last day, or only the first. Days are local and whole.

The commit's file tree has IDEA's Expand All and Collapse All in its header.

**Keys and menu.** Up and Down move the selection, Right goes to the parent commit and Left to the child, `Cmd+L` (`Ctrl+L`) focuses the text filter, and `Cmd+D` (`Ctrl+D`) shows the selected file's diff. A row's menu has Copy Commit ID, Go to Parent Commit and Go to Child Commit.

### Files and details

**The files** are Git History's tree (ADR 0046), each folder counting its files. A single click selects; a double-click shows the file in the Workspace's Diff Pane as the commit changed it, as a click in Git History does. The first time, the Diff Pane opens in a Tab of its own; dragged beside Git Graph once (ADR 0078), each double-click after retargets it there without leaving the Tab.

**The details**: the full message, selectable; `short id  author <email>  on date at time`; `committed by name on date at time` when the committer differs; the branch labels, then the tag labels; and "In N branches: …", up to six with "Show all", asked of the Server when the commit is selected and "Loading…" until it lands.

### Refresh

Git Graph reloads its first page and the refs when `HEAD` or its upstream moves in the repository it shows, as Git History follows `HEAD` (ADR 0046), and on Refresh; not on every Git snapshot, which an Agent editing files sends many times a minute. The watcher's fingerprint covers `HEAD`, the current branch, its upstream, the Base Branch and `packed-refs`; a fetch that moves only another branch, or another worktree committing to its own, shows after Refresh. That is accepted.

## The wire

- **`GitLog`** names where the walk starts: `HEAD`, as Git History asks, newest commit time first; every branch, tag and `HEAD`, as Git Graph opens; or one ref, `HEAD` among them, as its branch filter asks; the last two in date order. It takes an optional text, author email and span of author dates. `from` becomes a repeated field on the same number, which a Server older than this reads as its one `from`. The reply returns the commits the walk started from, which later pages send back, so pages stay consistent while refs move, generalizing ADR 0046's `from`; the refs, each with its name, kind (`HEAD`, local, remote, tag) and peeled commit; and the repository's `user.email`.
- **`CommitSummary`** gains its parents, the author's email, whether the committer differs, and whether `HEAD` reaches it. That mark costs one pass: children come before parents in this order, so it passes from `HEAD` to each marked commit's parents as the walk goes.
- **`CommitDetails`** gains the author's email, and the committer's name and time when they differ.
- **`GitCommitBranches { workspace_id, repository, commit }`** returns the local and remote branches whose tips reach the commit: one date-order walk from every tip, each commit passing the set of tips that reach it to its parents, until the commit comes up. A merge base per branch, tried first, took 13 s for 200 branches. It runs off the Client's request loop, as ADR 0046's requests do.

The date-order walk is gix-traverse's topological walk with `DateOrder`, given the repository's commit-graph file when it has one. Each page walks again from the start, as ADR 0046 accepted. Measured in a release build on a generated repository of 100,600 commits and 201 branches: without a commit-graph, a first page takes 0.55 s, the page at 50,000 0.77 s, a text that matches late 0.95 s, and a commit's branches 0.5 to 1 s; with one, as `git gc` and `git maintenance` write, 29 ms, 95 ms, 144 ms and 25 to 53 ms; Git History's first page 4 ms either way. A cursor kept on the Server waits until a repository makes these too slow.

An older Server refuses `ShowViewer` (ADR 0078). Git History still works against it: it ignores the new fields, walks from `HEAD`, and sends none of the new data, so the GUI draws what ADR 0046 drew.

## Not in this decision

Everything in IDEA that changes the repository; favourite branches; Go to Hash; Show Repository at Revision; Compare Versions and Compare with Local; Open in Browser; signatures; clickable ids in messages; the regex and match-case toggles; the Paths filter; several branches or authors at once; Recent and Favorites in the Branch menu; the diff preview; the Root column; the branch tree's toolbar; the Companion and the `condr` CLI.

## Delivery

1. **refactor**: the renames above, with no text a user reads.
2. **feat**: this ADR; the protocol, Server and core; the GUI with its `en` and `zh-CN` text; `CONTEXT.md` (Git History, Git Graph, the Chinese terms); AGENTS.md's dependency rule for `chrono`; the website's Changes, Overview, Workspaces and Keybindings pages in both languages.

## Tests

`condr-core`: date order keeping parents after children under clock skew; walking from every ref, from one and from `HEAD` as a ref with the refs; the text filter over the full message, ignoring case, and an id prefix of seven digits or more; the author filter by email, ignoring case, and the date filter keeping both ends; pages staying consistent from the returned starts while a branch moves; refs with their kinds and annotated tags peeled; the `HEAD` marks; a differing committer; the branches reaching a commit; every new field round-tripping. GUI: lane layout for a line, a fork and merge, an octopus merge and lanes cut at a page's end; ref labels merging; the branch tree's groups, folders, folding and filter; the Branch menu's submenus; the Date filter's local days; dates at each boundary; with a real Server, Git Graph opening from Git History's header on `HEAD` and closing the right sidebar, its filters and Refresh rendering, a double-click filtering by branch and the clear button showing every branch, Right stepping to the first parent, the text filter focused with its shortcut and applied on Enter, a selected commit listing its files, details and branches, and a double-clicked file opening the Diff Pane; offscreen screenshots in dark and light.
