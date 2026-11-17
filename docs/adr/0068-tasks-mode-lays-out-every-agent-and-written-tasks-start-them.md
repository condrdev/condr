# Tasks Mode Lays Out Every Agent, and Written Tasks Start Them

Status: accepted 2026-11-17, after an independent review of the 2026-11-16 proposal. Builds on ADR 0067, whose Agents section stays as it is and whose urgency order and sticky selection Tasks mode reuses; ADR 0002 and 0034, whose Managed Worktrees a task starts in; ADR 0020, whose connection states Tasks mode shows without the sidebar; ADR 0036, whose sizing rule the interaction area follows; ADR 0043, whose "shown in another window" page it shows; ADR 0044, which makes the hooks optional for a task's Agent; ADR 0047, whose Client hint in `Hello` it follows; and ADR 0053, whose lock rules the task store keeps. Departs from AGENTS.md's "persistence is protobuf" for tasks, which a Server keeps in SQLite. ADR 0002, 0020, 0021, 0023, 0028, 0035, 0036, 0043, 0063 and 0067 carry notes pointing here.

A person running several projects at once switches between them by unfolding Workspaces and selecting Panes, and every switch replaces the whole content area: the Tab strip, the layout, the right sidebar. ADR 0067 put every Agent in one list, but choosing a row still leaves the list behind. People think top-down, from what they mean to get done to who is doing it to the terminal it happens in, and Condr has nothing above the Workspace. This decision adds that level in two steps: **Tasks mode**, a whole-window view of every Agent in which one Agent's terminal opens beside the rest without leaving the view; then **tasks**, things the person writes down before any Agent starts, which Tasks mode starts and follows.

Terms:

- A **Task** is a piece of work the person wrote down, which an Agent does once it is started. A started Task holds one Workspace, and every Agent in that Workspace works on it. _Avoid_: issue, ticket, job. The Workspace stops being "a project- or task-level container" and becomes the project-level container where the code is.
- **Tasks mode** is the whole-window mode that shows every Agent and every Task. The interface calls it Tasks. _Avoid_: Overview (`SessionOverview` is an unrelated protocol name), Board (one of its two views), dashboard, Agents view (the sidebar has an Agents section).
- **To do** is the column, and the group, of Tasks not started.
- **Archive** ends a Task, finished or dropped, and can be undone. _Avoid_: Close, which ends a Workspace's terminals; Done and 完成, which are an Agent's state.

By ADR 0066, Task, Tasks and To do keep their English names in the Chinese interface, as Workspace, Agent, Needs you and the Agent states do; the actions translate (归档, 恢复, 保存, 开始). CONTEXT.md gains these entries, and the Workspace entry changes, with the implementation.

## Prior art

Surveyed 2026-11-16 from source; the notes are in the wayfinder map's research.

- **herdr** turned down a built-in board ("I don't want to bring big specialized features like this to main herdr") and points to its layouts. The community's boards all take their columns from Agent state, and their cards open the Agent's terminal beside or below the board.
- **paseo** groups its sidebar by status, Needs input, Failed, Ready to review, Working and Done, one row per Workspace at its most urgent Agent; it turned down a Kanban for a plugin.
- **T3 Code** has no board on `main`; a read-only, status-derived `/board` is an open pull request, and every proposal that added a task model or Server state was closed. Its answer to too many threads is putting finished ones away: settle, snooze, archive.
- **Superset (Paradev)** has a Workspaces page that shows as a list or a board, a list by default, whose columns are derived and fixed: "you cannot drag a PR to merged". Its separate Tasks board, which people do drag, shows no Agent state, and the two never meet.
- **Vibe Kanban**, the tool named after its board, removed it and kept a sidebar grouped by attention. Its users missed the workflow, write a ticket and come back to a diff, and asked for one view across projects and a way to archive.

What these show: columns derived from Agent state are read-only and say what a sorted list says; a board adds something only with work written before an Agent starts, or with PR state after it; and what people build for themselves is a board with the selected Agent's terminal beside it.

## Tasks mode

**Entering and leaving.** A **Tasks** row at the top of the left sidebar, above "Needs you", enters it; while the sidebar is collapsed to its avatars, a Tasks icon heads them. The new `toggle-tasks` action, `Cmd+0` on macOS and `Alt+0` elsewhere, enters and leaves it, the level above Tabs 1 to 9, and the Command Palette lists it (ADR 0063, 0064). Three ways leave: `toggle-tasks` again, **‹ Workspaces** at the top left where the sidebar toggle was, and **Open in Workspace** on the selected card. The first two return to the Workspace and Tab shown before; the third shows the card's Pane in its own Workspace and Tab. The Command Palette's Go to, in Tasks mode, selects an Agent's card when it names an Agent and otherwise leaves Tasks mode for the place it names. Entering, selecting and typing in Tasks mode never change the window's View (ADR 0021), and entering marks nothing seen. Escape is left to the terminal.

Only the main window enters Tasks mode; a Device, Workspace or Tab window (ADR 0043) has neither the row nor the shortcut.

**The window in Tasks mode.** Both sidebars are hidden, and `toggle-sidebar` and `toggle-changes` do nothing. The Tab strip gives way to a title row holding only ‹ Workspaces and the mode's name. Under it a toolbar spans the window, over the cards and the interaction area alike, as a project board has one: the scope on the left, as `MacBook › api-server ▾`, and **Layout** on the right, a popover with Table and Board as one segmented control and a Compact cards switch, which the Table leaves off since its rows always are compact. Neither row carries counts; every column says its own. Every layout and Tab action is inert: split, close Pane and close Tab (`Cmd+W` too, which closes neither a Pane nor the area), zoom, focus moves, resize, swap, `Cmd+1…9`, new, next and previous Tab and Workspace. Actions on a terminal's content act on the area's Pane.

**Connection states.** With the sidebar hidden, ADR 0020's mark has nowhere to be: a band at the top of the card area names each Device in the scope that is not connected, "build-box is not connected · Reconnecting…", with its state and Reconnect, except one the person disconnected, which needs no telling, and the interaction area shows ADR 0020's veil over a Pane of a Device that is retrying. A disconnected Device's cards are hidden, as ADR 0067 hides its rows, and the band says why.

**Columns and groups.** Five, in one order in both views: To do, Needs you (a bell, `Blocked` and `Error`), Done (unseen), Working, Idle (with `Unknown`). They are read-only and derived from state; nothing is dragged between them. Within a column the order is ADR 0067's: by urgency, then Session order. The card selected keeps the column it had when it was selected until another is selected or the interaction area closes, as ADR 0067's row does. Every column and every group folds; a folded column on the Board is a narrow strip with its count above its name written one letter per line, since GPUI does not rotate text, and a click unfolds it. A folded Needs you unfolds by itself when an Agent enters it; the others only update their count.

**Cards.** A card is an Agent, and any recognized Agent on a connected Device has one; a Pane running only a shell has none. Its title is its Task's title, or the Pane's terminal title for an Agent without a Task. Two exceptions are Tasks: a To do card, and a started Task whose Workspace has no Agent (see Tasks). The views:

- **Board, Cards**: the Agent's mark with its state badge, the title, the state's name in its colour unless Idle or Unknown, `Workspace · branch` with a Device tag, the diff stat and `↑` against the upstream, and the `detail` while there is one. The Workspace shows even when it repeats the Task's title.
- **Board, Compact**: one line per Agent, with a second for its `detail`.
- **Table**: always compact: mark, Title, Workspace, Branch, Device, Changes, and the State or the `detail`.

**Scope.** One choice: All, one Device, or one Workspace. A repository Workspace's scope includes the Managed Worktrees made from it, so choosing `api-server` keeps the Agents working on its Tasks; a worktree's own scope is just itself. The scope's breadcrumb opens a tree of All, Devices and their Workspaces, worktrees indented under their repository, each with its Agent count. Agent cards filter by their Workspace and Device and To do cards by their Task's location; a Task with no repository shows only under All or its Device. `next-agent` and `previous-agent` step within the scope.

**The interaction area.** Hidden at first. A click on a card opens it, beside the Table and below the Board, with a handle to drag; a click on the same card, or its ✕, closes it and ends the selection. For an Agent the area is a Dock holding only that Agent's Pane, whose header offers no zoom, Pane menu or drag, since their actions do nothing here, under a header of its own: the mark, the title, `Workspace · branch · Agent · Device`, Archive task when it has a Task, Open in Workspace and ✕. For a To do Task it is the Task's form (see Tasks).

- **The keyboard.** A click on a card, `next-agent`, `previous-agent` and an OS notification's click select a card and give the keyboard to its terminal, so answer, `Cmd+]`, answer needs no mouse. In the card area, which a click on its background focuses, the arrows move the selection and keep the keyboard there: the area previews the Pane without taking it, so no arrow or Enter reaches an Agent by accident, and stepping through cards resizes nothing. Enter, or a click in the terminal, gives it the keyboard. A click in the terminal does not run `select_pane`.
- **No selection, no target.** With no card selected, or with the keyboard in the card area, Tasks mode has no terminal target and reports `Focus(false)`; nothing in a hidden Tab receives a paste or is marked seen.
- **Sizing.** The window sizes the PTY while the area's terminal holds the keyboard (ADR 0036, 0041), so the Agent redraws once for the area and once more back in its Tab, as zooming in and out does today; moving the keyboard away, or closing the area, gives the sizing up.
- **No layout.** The Pane's own header is not shown: no split, zoom, close, menu or drag. The Dock is not subscribed to and is locked, so it sends no Layout command; zoom is Session state (ADR 0010) and is not borrowed. Changing the layout means Open in Workspace.
- **Shown elsewhere.** When a Device, Workspace or Tab window shows the Pane, the area shows ADR 0043's page for it, with Go to Window; a terminal is never drawn twice.
- **The Agent exits.** When it exits and its Pane stays, the area keeps showing the Pane, marked "Agent exited", until another card is selected or the area closes; if its Task is left without Agents, the selection moves to that Task's card. When the Pane closes, the area closes.

**Navigation.** `next-agent` and `previous-agent` move the selection among the Agent cards in column order. In Tasks mode an OS notification's click selects its Agent's card, first widening the scope to All when the Agent is outside it. In normal mode a notification's click and a "Needs you" row still run `select_pane`, as ADR 0067 has them. `Activated` (ADR 0021) brings the window forward and changes the View underneath, and does not take the person out of Tasks mode.

**Remembered** in the GUI state file (ADR 0023): whether the main window is in Tasks mode, Table or Board, Cards or Compact, which columns and groups are folded, the scope, and each view's split. The scope returns to All when its Device or Workspace is gone. The selected card is not kept.

**Rendering.** The card area is a view of its own, `.cached()`, rendered again whenever `Condr` is notified, which a terminal frame never does (frames notify only their panels, `events.rs:470`); so every input a card shows, state, title, `detail`, branch, diff stat, upstream, the unseen bit, a bell, a Task, the scope or a connection, reaches it, and no frame does. "The Pane the keyboard targets" and "the Device input goes to" come from the selection in Tasks mode rather than from the Tab the window shows: `focused_pane()`, `input_connection()` and `refresh_target_pane()` change, and a `LayoutChanged` no longer pulls the target back. The Tasks Dock is one surface per window under a key of its own, not a Device and Tab pair, whose center is replaced when the card changes; `rebuild_window_dock` stops assuming every surface of a window has one size, and a Pane header's `solo` check stops counting the Tasks surface, which would otherwise drop the focus border in normal mode.

The left sidebar is rebuilt today on every terminal frame of a mounted Pane: GPUI re-renders a dirty view's ancestors up to the root, and `Condr::render` builds the sidebar with no cached view in between. It becomes a cached view the same way, and the comment at `events.rs:470` that says notifying only the panels spares the root is corrected.

## Tasks

**A Task and its Workspace.** A started Task holds exactly one Workspace, and a Workspace belongs to at most one Task. Every Agent in that Workspace, whenever and in whichever Pane it starts, is the Task's, and its card carries the Task's title; Agents have no identity of their own, and a Pane can run one Agent after another, so neither a Pane nor an Agent could carry the link. Comparing two Agents' takes on one piece of work is two Tasks. An Agent outside any Task's Workspace shows as itself; Tasks mode makes no Task for it.

**What a Task holds.** A per-Device number; a title; a plain-text description of at most 64 KiB; its repository, as the root directory of a repository Workspace on its Device, or none; its order in To do; whether it is archived; the Workspace it holds, or none; and the last place it was worked: the worktree's path, its branch, its Base Branch (ADR 0034), whether Condr made it and from which repository. No state field: To do, started and archived follow from these, so a stored "in progress" can never disagree with a closed Workspace.

**Where Tasks live.** On each Device's Server, in a SQLite database in its data directory (`rusqlite`, bundled, WAL, its schema's version in `PRAGMA user_version`), behind a `TaskStore` module. This departs from AGENTS.md, which keeps the Snapshot and the GUI state in protobuf; the line changes with the implementation.

- **Only the Server opens it.** The GUI and the CLI go through the protocol, so a stopped Server means no `condr task`, as it means no `condr workspace`.
- **Not in the GUI.** `condr-server` gains a cargo feature for the store; the GUI's builds leave it off and its tests, which run a Server in process, turn it on.
- **Off the lock.** Tasks live in the Server's state under its lock, and a worker writes them to the database off that lock, as the Snapshot worker does (ADR 0005); a poisoned lock aborts as every Server-level lock does (ADR 0053). Tasks are not part of the Session Snapshot, which every `LayoutChanged` carries whole.
- **Never destroyed.** A database whose `user_version` is newer than the Server's, after a downgrade, or that cannot be opened, is renamed aside to `tasks.sqlite.unreadable-<time>`, the Server starts with no Tasks and says so in its log and `server status`, and the file is left for the person (ADR 0028's rule for files a Server cannot restore).
- **On the wire**, Tasks are protobuf (ADR 0028), and only for a Client that says `tasks` in `Hello`, a new Client hint after ADR 0047's `push`. An older GUI or Companion never receives a record or event it does not know, which would make it ask for a Bootstrap and, twice in ten seconds, disconnect.
- **What a Client receives.** The Bootstrap carries the To do and started Tasks without their descriptions, a few hundred bytes each, and every change follows as a reliable event for that one Task. A form opening fetches its description, and the Archived list unfolding fetches the archived Tasks, with the same requests `condr task get` and `list` make.
- **Old Servers.** A Server that knows Tasks answers the hint with the Tasks record in its Bootstrap, empty or not; without it, the GUI offers no To do for that Device and no Device for a new Task, and its Agents still show. The CLI turns the older Server's `UNSUPPORTED_REQUEST` into `feature_unavailable`.
- **Every Client** of a Device sees and edits its Tasks, and the last save wins. A form open on a Task another Client deletes closes; on one another Client starts, the selection follows to its Agent's card.
- **A disconnected Device's** Tasks are hidden with its Agents, under the band above.
- **The Server keeps the link.** When a Task's Workspace closes, or fails to come back after a restart, it clears the link in the same step and records the last place, so a Workspace number used again never points at an old Task.

**Lifecycle.**

- **To do**, a Task without a Workspace: a card in To do, ordered by dragging, new ones on top. One that was worked before shows `↺ branch`.
- **Starting**: while a Start is under way the Task is reserved and its To do card reads "Starting…"; another Start of it is refused.
- **Started**, a Task holding a Workspace: shown as its Agents' cards. While its Workspace has no Agent, a Task card in Idle reads "No agent running", and its interaction area shows the Workspace's focused Pane, where an Agent can be started again.
- **Back to To do**: when its Workspace closes, keeping its last place; the next Start continues there by default.
- **Archived**: in a folded Archived list at the bottom of To do, until restored to To do or deleted. Archiving a To do Task asks nothing. Archiving a started one asks:
  - **Close its Workspace**, checked only when a Start of this Task created the Workspace, a new worktree or one reopened to continue, and unchecked when the Task borrowed one, such as the repository's own: it says how many Agents end and names one that is working or waiting for the person.
  - **Remove the worktree**, unchecked: only for a Managed Worktree, not with uncommitted changes, and the branch stays (ADR 0002).
  - Left open, the Workspace leaves the Task, and its Agents become cards of their own.
- **Deleted**: only from To do and Archived, after "Delete ‘…’? This can't be undone."
- Archived Tasks are kept. Nothing archives by itself: Condr follows no PR, and idleness is no evidence of being finished.

**Writing a Task.** "+ New task" heads the To do column; the Command Palette's New Task, the `new-task` action, has no default key, like `new-workspace`. The interaction area's left half is the form: Title, required; Description, plain text, sent to the Agent with the title; Device, required, defaulting to the scope's or this machine's; Repository, optional, among that Device's repository Workspaces. Nothing is kept until **Save**, or `Cmd+S` in the form. A new Task has no card until it is saved; then it heads To do and the right half, Start, appears. While a saved Task has unsaved changes, Start is disabled with "Save your changes first."; leaving with changes asks Save, Discard or Cancel. Changing a To do Task's Device creates it on the new Device first and deletes it from the old one second; a failed delete leaves both and says so, and the new copy starts without a last place. A started Task does not move.

**Starting a Task**, in the right half of the area:

- **Where**:
  1. **Continue in ↺ branch**, for a Task worked before: the default. When a Workspace is already open on that folder and holds no Task, Continue uses it; it is disabled when another Task holds that Workspace, and when the folder is gone ("The folder no longer exists"), which makes New worktree the default with the old branch filled in.
  2. **Existing Workspace**: the default otherwise, the Task's repository Workspace when no Task holds it, or, for a Task without a repository, the Device's first Workspace no Task holds. A Workspace another Task holds is listed greyed with that Task's title. A note says when the chosen Workspace already has Agents, since they will carry this Task's title.
  3. **New worktree** from the Task's repository, with an editable branch: the default when another Task holds the repository's Workspace. It needs that Workspace open, and says "Open api-server first" when it is not. A branch that exists and is checked out nowhere is checked out, recording no Base Branch, as Create Worktree does today; one checked out elsewhere is refused under the field. The branch defaults to today's `worktree/<slug>`; a title with any character outside ASCII gives `worktree/<repository slug>-<first 8 hex digits of SHA-256(title)>` instead, so 中文 and mixed titles do not become `worktree/change` or `fix----ime-bug`.

  With nothing to offer, no free Workspace on the Device and no repository open, Start is disabled with "Open a Workspace on build-box first."
- **Agent**: the fully supported Agents (ADR 0035) on that Device's PATH, the last one used first; the button says "Start Claude Code". The hooks are not required, since state comes from OSC 7501 (ADR 0044). Without them the selector says "Hooks not installed: the conversation won't resume after a restart." with Install.
- **The prompt** is the title, a blank line and the description, and the area does not show it; to change it, edit the description.
- **Failures**: a branch checked out elsewhere shows under its field; anything else is one `report_error` toast with Copy (ADR 0020), and the Task stays in To do.
- **Success**: the Task leaves To do, its card reads "Starting…" in Working until its Agent appears, the card is selected, and the area shows its terminal.

**The Server's side.** One request, `StartTask { task, where, agent }`, where `where` is continue, a Workspace or a new worktree with its branch, used by the GUI and the CLI alike:

1. Under its lock the Server reserves the Task, so a second `StartTask` for it fails with `task_starting`, and works the rest off the lock, as `agent start` does.
2. It prepares the Workspace. A new worktree is created from the repository's `HEAD`, as Create Worktree does today. Continuing opens the last path again, or uses a Workspace already open on it; when the Task's record says Condr made it, the folder is still a linked worktree of that repository and the repository's Workspace is open, the Workspace is associated as a Managed Worktree of it again, since that record is the Server's own; otherwise it opens as a plain Workspace. An existing Workspace gets a new Tab named after the Task, leaving its Panes alone.
3. It writes `tasks/<id>.md` in its data directory, the title, a blank line and the description, once at Start. The file stays until the Task is archived or deleted, and the next Start rewrites it. It lives outside the worktree, so `git status` does not show it.
4. It waits for that first Pane's shell to be ready, then types the Agent's command, with one short ASCII sentence as the initial prompt: read that file and do the task it describes. The prompt rides the CLI's own initial-prompt argument, so every CLI waits out its own trust dialog before submitting it. `AgentSpec` gains how that argument is given: positional (Claude Code, Codex, Pi, Oh My Pi, Grok Build, Cursor), `--prompt` (OpenCode) or `-i` (Copilot, Antigravity). A data directory whose path holds a character an npm `.cmd` shim refuses (`& | < > ^ ( ) % ! "`) fails this step with a message that says so.
5. Typing the command is the commit point. The Server links the Task, writes it, and answers with the Workspace, Tab and Pane; it does not wait for the Agent, as `agent start` does, and takes no Agent name.

A failure before the commit point undoes what this Start made: a Workspace it created is closed, a worktree it created is removed, and a branch it created is deleted if it still points at the commit it was made from; a Tab it opened is closed; the reservation ends and the Task stays in To do. The answer names the step and a code. After the commit point nothing is undone, since an Agent may already have written files or shown an error worth reading: when no Agent appears in its Workspace within 30 seconds, the card turns to "No agent running" and one toast says the Agent did not start and to look at its Pane.

A requesting Client that disconnects does not stop a Start; it finishes on the Server, and every Client sees the result. A Server that stops in the middle loses the reservation, and what it had made before the commit point comes back as a Workspace without a Task.

**The CLI.** `condr task` follows the other groups: JSON results, `.error.code` and `.error.message` on failure, `--device` before the group, numeric ids per Device, and, run in a Pane, an omitted id meaning the Task holding that Pane's Workspace.

- `list [--archived]`, `get [id]`, `create --title <text> [--description <text> | --description-file <path|->] [--repository <workspace>]`, `edit [id] …`, `start [id] --kind <agent> [--workspace <id> | --worktree [<branch>] | --continue] [--focus]`, `archive [id] [--close-workspace] [--remove-worktree]`, `restore [id]` and `delete [id]`.
- `start` takes the GUI's defaults for `where` and fails with `where_required` when there is none, requires `--kind`, since the CLI has no "last used", and returns `workspace`, `tab` and `pane`.
- `archive` closes and removes only when told. When the caller sits in the Workspace it closes, the Server answers first.
- A Task is `{ task, title, description, status: todo|starting|started|archived, repository, workspace, last: { branch, base, path, managed } }`. New codes: `task_not_found`, `no_task`, `task_starting`, `task_started`, `workspace_has_task`, `where_required`.
- `condr --skill` gains a `task` row and a rule: an Agent started on a Task reads the file its prompt names, may read its Task again with `get`, creates one when the person asks for follow-up work to be recorded, and never archives or deletes unless asked; whether work is finished is the person's call.

## Considered

- **A board as the whole answer**, columns the person drags. Rejected: derived columns cannot be dragged, and every surveyed tool that let people drag them dropped or sidelined the board.
- **Tasks and Tasks mode as two pages.** Rejected: switching between them is the context switch this removes; Superset's two unconnected boards are the cost.
- **Keeping the sidebar beside Tasks mode**, as paseo does. Rejected: its Agents section would repeat the Table, and the Board needs the width.
- **A "Needs you" row or a notification entering Tasks mode from normal mode.** Rejected: it would change what ADR 0067's rows do for people who never open Tasks mode.
- **A Task tied to a Pane, or to several Workspaces.** Rejected: a review Agent opened in another Pane would not count, a Pane cannot tell one Agent from the next, and two takes are two Tasks.
- **Tasks in a file on the GUI's machine**, shared by the GUI and the CLI. Rejected: a `ServerId` is a hash of a socket path and can repeat across machines, a closed Workspace's number can return while the GUI is away, the GUI would need a file watcher it does not have, and an Agent on a remote Device could not reach its Task. A mix of both stores was rejected for keeping two.
- **A protobuf file instead of SQLite.** It fits version one, a few hundred records with one writer, with one schema shared with the wire, and an independent review preferred it. Chosen against it because sources from issue trackers, beyond this decision but expected, will need incremental sync and search, and moving later would mean an import; the cargo feature keeps it out of the GUI.
- **Syncing Tasks only while Tasks mode is open**, a `WatchTasks` like ADR 0041's `WatchTerminals`. Rejected: Tasks without descriptions or archives weigh less than one terminal view in the same Bootstrap, and a subscription would delay every entry by a round trip and need per-Client state that survives reconnects.
- **Committing when the Agent is recognized.** Rejected: the process table re-probes an idle terminal only every 30 seconds, so a slow start would hang or be undone, and undoing it would remove a worktree the Agent had written to and the Pane that shows why it failed.
- **Pasting the Task once the Agent is ready.** Rejected: Codex, Copilot, Cursor and Antigravity report nothing before their first turn, so the paste goes in blind and can answer a trust dialog, which Antigravity shows for every new folder and so for every worktree.
- **The whole Task as an argument**, through a temporary file and the shell's substitution. Rejected for needing a Windows path of its own: npm's `.cmd` shims refuse common punctuation.
- **`condr task get` as the Agent's first step.** Rejected: running a command needs approval in most Agents, so every Task would begin `Blocked`.
- **Requiring the hooks to start**, or marking which Agents speak OSC 7501. Rejected: the protocol reports state without the hooks, an Agent without state still works, and whether a CLI speaks it depends on its version, which a static field would get wrong.
- **Saving as one types.** Rejected: the person wanted a Task kept only when they say so, and a Start never runs on half-typed text.
- **Counts in the title row.** Rejected: every column has its own.

## Not in this decision

Tasks from GitHub, Jira or other trackers, and writing back to them; PR state; choosing a worktree's start point, which the roadmap's "Worktrees ready to use" adds to `CreateWorktree`; a diff or Changes in the interaction area, for which Open in Workspace remains; ordering To do from the CLI, `task focus` and `task wait`; a version check on saving a Task; Tasks mode in detached windows; lightening the Bootstrap's terminal views and Git state, a question of its own; and the Companion's view of Tasks, which needs only the `tasks` hint to receive them later.

Consequences:

- **The sidebar changes after all**, by one row at its top, and becomes a cached view.
- **Resizes.** Giving an Agent the keyboard in the area, and going back to its Tab, each make it redraw for a new size, as zoom does.
- **One Task per Workspace.** The repository's own Workspace can hold one Task at a time; the second Task on a repository starts in a worktree by default.
- **A file per Task** in the Server's data directory from its first Start until it is archived or deleted.
- **The task file must be readable without approval**, which is checked per Agent before the Server's side is built; the documentation names any Agent that asks.
- **No hooks, no resume.** A Task's Agent started without hooks is not resumed after a Server restart; its Task shows "No agent running" until it is started again in the same place.
- **A failed start leaves its Workspace.** After the commit point a Workspace stays even when its Agent never ran, for the person to read and close.
- **Older Clients see no Tasks**, and a GUI that has the feature off sees none from an older Server.
- **A dependency.** `rusqlite` builds SQLite's C source into `condr`, though not into `condr-gui`.

Tests:

- **GUI**, a new `visual/tasks.rs` with `gpui-kit/test-support`:
  - Entering and leaving: both sidebars hidden, the View unchanged, nothing marked seen and `Focus(false)` reported on entering with an unseen Done in a hidden Tab, the three exits, Go to, the main window only, `Activated` not leaving.
  - The connection band for a disconnected Device in the scope, and the veil over a retrying Device's Pane.
  - Order: To do first, urgency within a column, the selected card staying, a folded Needs you unfolding, the scope and its worktrees, a notification widening it; in normal mode a notification and a "Needs you" row still selecting the Pane.
  - The area: opening and closing; arrows and Enter in the card area never reaching a terminal and taking no sizing until Enter; no Layout command from it; a click in its terminal leaving the View; layout and Tab actions inert; ADR 0043's page for a Pane a Device, Workspace or Tab window shows; the normal-mode focus border unaffected; an exited Agent's Pane kept.
  - A real drag-selection in the area's terminal.
  - Tasks: Save, Discard and the disabled Start; a form whose Task another Client deletes or starts; Where's defaults, greyed Workspaces and its disabled cases; the branch rule; "Starting…"; the delete confirmation; the archive dialog's default for a created and a borrowed Workspace.
- **Rendering.** A test-support render counter on the card view and the sidebar: N frames of the selected Pane leave both counts unchanged, and one Agent state change renders each once. The four terminal performance tests AGENTS.md names pass unchanged.
- **Server.**
  - `TaskStore` creates, edits, orders, archives, restores, deletes and reports its schema version; a newer `user_version` and a corrupt file are renamed aside and the Server starts empty.
  - `StartTask`, for each `where`: a failure before the commit point closes the Workspace or Tab and removes the worktree and the new branch it made, leaving the Task in To do; after it, nothing is undone; an Agent that exits at once and one that never appears leave "No agent running"; two concurrent Starts, one refused with `task_starting`; a requester disconnecting mid-Start; a Server stopped mid-Start; the task file written at Start and removed with archive and delete; a refused character in the data directory's path.
  - A closed Workspace, and one not restored, clear the link and record the last place. Continue restores a Managed Worktree only for a linked worktree of the same repository whose Workspace is open, uses a Workspace already open on the folder, and handles a missing folder.
  - Two Clients that said `tasks` receive the same Tasks through Bootstrap and per-Task events, without descriptions; a Client that did not receives none; the last save wins; a simulated older Server answers `feature_unavailable`.
- **CLI**, a new `tests/task_cli.rs`: `create`, `get`, `edit`, `start`, `archive`, `restore` and `delete` against a local Server, an omitted id inside a Pane, each new error code including `where_required`, and one run through `--device` over TCP.
- **By hand, per Agent**, before the Server's side is built: for each of the nine fully supported Agents, a Task started in a new worktree of a trusted repository, recording whether reading the task file asks for approval (Claude Code's included, which this decision has not checked), whether a trust dialog lets the Task through afterwards (Antigravity above all), and on Windows whether an npm `.cmd` shim starts with the short prompt. The results go to `docs/research/` and may change the prompt's sentence.
- **By hand, on Windows**, with each GUI step: the sidebar's frame rate under Codex streaming; then in Tasks mode, Codex streaming and animating in the area while selecting by drag, typing, stepping cards with `Alt+]` and dragging the split, at display cadence with no backlog, and the redraw on giving an Agent the keyboard judged acceptable.

Delivery, each step merged on its own:

1. **The sidebar as a cached view**, with the render counter that first shows it rebuilding per frame, and its Windows check.
2. **Tasks mode**, without To do and without a count on its row, with no protocol change: entering and leaving, the connection band, both views and their densities, folding, the scope, the interaction area with its keyboard and Dock rules, the cached card view, the GUI state file, the GUI and rendering tests above, and the Windows acceptance. Usable from here.
3. **The per-Agent check** above, with a hand-built prompt and file, recorded in `docs/research/`.
4. **The Server's Tasks**: `TaskStore` behind its cargo feature, the `tasks` hint, the records, requests and per-Task events, the models in `condr-client` and `condr-mobile` decoding them, the link's upkeep, `StartTask` with `AgentSpec`'s initial-prompt field, and their tests.
5. **`condr task`**, the skill's row and rule, and `task_cli.rs`.
6. **Tasks in Tasks mode**: the To do column and the row's count, the form with Save, Start, "Starting…", the archive dialog, and their tests.
7. **Finishing**: CONTEXT.md's Task, Tasks mode, To do and Archive entries and the changed Workspace entry; AGENTS.md's persistence line and the store's feature; the website's pages on Tasks mode and Tasks and the CLI reference, in English and `zh-cn`; and the roadmap's section removed.
