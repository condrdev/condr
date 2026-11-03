# A New Workspace Can Clone a Repository or Make a Directory

Status: accepted 2026-11-03. Builds on ADR 0002, whose directory pickers the new forms open, and ADR 0055, whose prompt marks tell the Server a new shell is ready. Amends ADR 0005 and `condr agent start`, which now wait for that mark too; ADR 0055, whose scripts announce themselves before the person's startup files; and the rule in AGENTS.md that Condr never shells out to `git`, since Condr's own Git layer still does not but Clone types `git clone` into a Pane. Departs from ADR 0028's gating, as explained below. Enables the Welcome page's "Clone Repository" row, which was disabled until now. A list of GitHub repositories, `owner/repo` shorthand, clone options, `git init` and a default clone directory were considered and left out.

New Workspace could only open a directory that already existed: the native picker on a local Server, and the Server-path field everywhere else (ADR 0002). A repository that was not yet on the machine took three steps. The person opened some directory, cloned into it from a Pane, and then opened the clone as a second Workspace. This mostly happened on a remote Device, where nothing else on the screen reaches the Server's filesystem.

**Choosing how.** New Workspace, through its shortcut or a Device's "+" in the sidebar, now opens a chooser titled "New Workspace on <Device>" that lists three ways, as Paseo's Add project does:

- **Open Project**, "Choose a directory on <Device>". This is what New Workspace did until now: the native picker for a local Server, and the Server-path field inside the chooser otherwise.
- **Clone Repository**, "Clone a Git repository into a new directory on <Device>".
- **New Directory**, "Create an empty directory on <Device>".

Up and Down move the highlight, Enter takes the row, Esc closes, and a footer shows those keys. Clone Repository and New Directory turn the chooser into their form in place, and Esc goes back to the list. The Welcome page lists the same three rows, under the same names, above Connect Remote Device.

**The forms.** New Directory asks for a parent and a name. Clone Repository adds a URL above them.

- **Parent.** The form shows the parent as one line of text with a "Change…" button beside it. The button opens the picker Open Project uses, the native one on a local Server and ADR 0002's Server-path field otherwise, and the chosen directory comes back into the form. The parent is not an editable field of the form. In ADR 0002's field Tab steps into a directory, which would clash with Tab moving between the form's fields. That field also lists the directory before the last separator, not the one named.
- **Default parent.** The parent starts as the parent of the presented Workspace's Root Directory on that Device. The Server names that parent in its `BrowseDirectory` answer, since a Client cannot split another platform's path. A Managed Worktree uses its parent repository Workspace's Root Directory instead, so a clone does not land among `<repo>.worktrees/`. With no Workspace presented on that Device, or a Root Directory the Server can no longer read, the parent is the home directory. No last-used directory is remembered.
- **Name.** Clone's name follows the URL until the person edits it. It is derived as git's own `git_url_basename` derives it: trailing `/`, `/.git`, `.git` and `.bundle` come off, then the last segment after `/`, `\`, or the `:` of an scp-like `git@host:owner/repo.git` is taken, and a host's port is not a name. `condr_core::clone_directory_name` holds the rule, and the CLI uses it too.
- **Existing names.** Once the parent is chosen, the GUI sends one `BrowseDirectory` for it. When that listing already holds the name, the form says so under the name field before anything is sent. The listing holds only directories, skips `.git` and stops at 2000 entries. Whatever it cannot show, such as a file with that name or a directory made in the meantime, the Server refuses.
- **URL.** Whitespace around the URL is trimmed, so a pasted trailing newline does no harm. The field then refuses:
  - empty text;
  - control characters;
  - the quote characters `'`, `"` and `` ` ``, and U+2018 to U+201F, which PowerShell also takes for quotes;
  - more than 2048 bytes;
  - a local path that is not absolute. A path without a scheme or an scp-like `host:` is a local path, and git would resolve a relative one against the shell's directory, not the CLI's.

  Spaces inside are allowed, since the line is quoted. Whether the URL names a repository is for git to say.
- **Submitting.** The form closes when it sends the request. A refusal from the Server is one toast with its reason, as for every layout request today (ADR 0020).

**The Server.** A new layout request, `CreateWorkspaceDirectory { parent_directory, directory_name, name, clone_url }`, answers with the same `WorkspaceCreated` that `CreateWorkspace` does. It is a request of its own, and `CreateWorkspace` gains no fields, because an older Server ignores fields it does not know. Given a parent and a name, it would open a Workspace on the parent.

- **Validating.** The Server checks `clone_url` by the form's rules before it touches the disk.
- **Joining the path.** The Server joins `parent_directory` and `directory_name`, since a Client cannot join a path on another platform (ADR 0002). `directory_name` must be exactly one `Component::Normal` equal to the text itself. That rules out separators, `.`, `..` and, on Windows, a drive-relative `C:foo`, which `PathBuf::join` would put outside the parent. On a Windows Server the name also may not contain `<>:"|?*`, may not end in a dot or a space, and may not be a reserved device name such as `CON` or `NUL`.
- **Making the directory.** The Server creates exactly one level and refuses a name that already exists. It does this off the Session lock in `prepare_external_layout`, where `CreateWorkspace` validates its root. If the Workspace is not created after all, the Server removes the directory with a non-recursive `remove_dir`, so only a directory that is still empty goes.
- **Typing the clone.** Once the Workspace exists, the Server types the clone into the Workspace's first Pane. It waits for the shell's first prompt, as described below, checks the Terminal is still the same one, and submits the line once. The target is the absolute path the Server just made, not `.`, so a startup file that changes directory cannot redirect it. What it types depends on the shell, which is the program the Pane started:
  - POSIX shells type `git clone -- '<url>' '<path>'`.
  - PowerShell types `& git clone -- '<url>' '<path>'`. This is plain, not the `Start-Process` form `condr agent start` uses, which exists for quotes inside arguments that the URL rules exclude here.
  - cmd runs the same PowerShell line through `-EncodedCommand`, as `condr agent start` does, since cmd would expand `%` inside quotes.

  `--` keeps a URL that starts with `-` from being taken as an option.
- **When it cannot.** The Server prints the plain command to run in the Pane instead of typing it, as it does for a resume that fails, in three cases: the shell never reaches its prompt within 30 seconds, the shell ends first, or the person types into the Pane before the line is sent. In the last case the clone would mix with their keys.
- **Reply and Snapshot.** The reply comes when the Workspace exists, not when the clone ends. The clone is not part of the Snapshot, so a Server that restarts before the line is typed types nothing.

**Older Servers.** ADR 0028 has a Client hide a request the Server is too old for and has the CLI refuse it with `feature_unavailable`. That needs a protocol number to compare, and the protocol stays `1` until the release ADR 0041 names raises it. Until then `CreateWorkspaceDirectory` is not gated. A Server older than this build finds it unknown and answers "this Server does not support that request; update it". The GUI shows that as its one toast, and the CLI fails with that message. When the protocol is raised, this request is gated with the rest.

**Waiting for the prompt.** Until now the Server decided a new shell was ready with the process probe of ADR 0005. Every 500 milliseconds it read the process table. On Unix it called the shell idle when the shell led the foreground process group alone; on Windows, when the shell's Job held no other process. That is also true while the shell is still reading its startup files and runs only builtins at that moment. The line then arrives before the prompt. Usually the line editor reads it later, but a startup file that reads the terminal takes it instead. zsh's `compinit` asking "Ignore insecure directories and continue [y] or abort compinit [n]?" eats the `g` and leaves `it clone …` as the command. A new Pane is always in that window. A clone, a resume after a restart, and `condr agent start` right after `pane split`, which the automation guide recommends, all hit it.

The shell's first OSC 133;A (ADR 0055) is sent only once the startup files have run. Waiting for it alone is not enough. A shell that never sends marks would have to be timed out into the probe, and the probe would again type into a startup file stuck in `read`. So the integration scripts now also announce themselves before the person's startup files run, with `OSC 777;notify;condr://shell;integration`. It is the same notify form `condr agent-hook` uses, and the VT intercepts it the same way.

- **zsh** sends it from Condr's `.zshenv`.
- **bash** sends it from Condr's wrapper, only when `BASH_VERSINFO` is 4.4 or later. Before 4.4, bash sends no marks.
- **fish** sends it from Condr's `vendor_conf.d` script, which runs before `config.fish`.

The Server then waits for the shell's first prompt in one of three ways:

- **The Pane announced itself.** The Server waits for the first 133;A and does not fall back to the probe. A startup file stuck in `read` therefore gets nothing typed into it.
- **A PowerShell Pane on Windows.** Condr already wraps its prompt to report the directory with OSC 9;9 at every prompt, after the profile has run. The first OSC 9;9 counts as its first prompt.
- **Any other Pane.** This covers bash before 4.4, cmd, other shells, `shell_integration = false`, and a zsh whose `/etc/zshenv` sets `ZDOTDIR`. The Server uses the process probe as before.

A Terminal records the announcement and its first prompt once and never clears either, so a prompt that arrives before anyone waits is not missed. At the mark, the person's own `precmd` hooks and the prompt itself may still be running, and the line editor has not yet turned on bracketed paste. The Server therefore waits up to one more second for bracketed paste. If the editor turns it on, the line is pasted whole. Otherwise it is typed as keys, which a key widget such as zsh-autopair or a fish abbreviation may rewrite. zsh 5.1, bash 5.1 and fish 3 turn it on. The whole wait is bounded by each caller's own limit: 30 seconds for a clone and for a resume, and `--timeout` for `condr agent start`. `condr agent start` now waits for the first prompt instead of answering `pane_busy`.

**Quoting.** The line is quoted by the same `launch.rs` code `condr agent start` uses, and that code had two holes that a URL or an argument could exploit:

- **fish.** Inside single quotes fish treats `\'` and `\\` as escapes, so the POSIX form could close the quote early. fish now gets its own quoting.
- **PowerShell.** PowerShell takes U+2018 to U+201B as single quotes, but only ASCII `'` was doubled. All five are now doubled.

The clone's URL rules shut both holes out a second time.

**The terminal does the rest.** Condr has no progress bar, cancel button, retry or cleanup for a clone:

- git prints its progress in the Pane.
- A password, SSH passphrase, unknown host key or second-factor prompt is answered in the Pane.
- Ctrl+C cancels the clone, and git removes what it wrote.
- After a failure, the person fixes the cause, for example with `ssh-add` or a credential, and runs the line again from the shell's history, or the printed command in a cmd Pane, whose history holds the encoded form.
- A person who no longer wants the clone closes the Workspace. That leaves the directory behind, as closing always leaves files.
- Changes and History appear once `.git` does, because the Git watcher looks for the repository again at every scan.

**The CLI.** `condr workspace clone <URL> [<DIRECTORY>] [--label TEXT] [--focus]` sends the same request, shaped like `git clone <url> [<dir>]`. `<DIRECTORY>` is relative to the current directory, or absolute with `--device`, and defaults to the name derived from the URL. The command fails when no name can be derived. It prints what `workspace create` prints, the root Pane included, and returns once the Workspace exists. How the clone went is read with `condr pane read`. New Directory has no CLI form, because `mkdir` and `workspace create` already do it.

**Why git in a Pane.** All five tools examined (IntelliJ, Zed, VS Code, Paseo and t3code) run the `git` binary to clone. They differ only in how they handle prompts. IntelliJ and VS Code point `GIT_ASKPASS` and `SSH_ASKPASS` at helpers of their own, which turn passwords, passphrases and host keys into dialogs. Zed, Paseo and t3code refuse prompts, and the clone fails. t3code also creates the project first and clones inside it, as Condr now does. A Pane provides what those askpass helpers imitate without any of their code: the Server machine's git, PATH, git config, credential helpers, SSH agent and known hosts, and a terminal to answer any prompt in.

Rejected:

- **Cloning with gix in the Server.** gix's `blocking-http-transport-reqwest` would share the Server's reqwest 0.13 and rustls, and gix reads the machine's git config: `insteadOf`, `core.sshCommand`, `http.proxy` and credential helpers. But the Server has no terminal, so a first-time host key, a password or a passphrase would fail the clone. The protocol would also need its own progress, cancel and cleanup. All of that would buy independence from the git binary, which the Server's Agents need anyway.
- **Running `git clone` from the Server with `GIT_TERMINAL_PROMPT=0`,** as Paseo and t3code do. It has the same prompt problem and adds a progress parser.
- **Relaying askpass prompts to the GUI,** as IntelliJ and VS Code do. It is the most code, and every Client, the Companion included, would need the dialogs.
- **Raising the protocol to `2` to gate the new request now.** The raise is left to the release ADR 0041 names.
- **Waiting for the prompt mark and falling back to the probe after a few seconds.** It types into the same stuck `read` the mark was meant to avoid. Announcing first lets the Server know the mark will come.
- **The Server-path field as an editable parent field inside the form.** Its Tab and its listing mean something else there.
- **A list of the person's GitHub repositories and `owner/repo` shorthand.** Paseo runs `gh` for the list, and t3code calls GitHub's REST API. A URL can be pasted instead. Left out until someone asks for it.
- **Branch, depth and submodule options.** The Pane is right there for `git switch` and `git submodule update`.
- **`git init` in a New Directory,** which t3code does and Paseo does not. The person or an Agent runs it when they want it.
- **A default clone directory setting or a remembered parent.** VS Code and t3code have the setting, and IntelliJ and Paseo remember the last parent. The presented Workspace's parent covers the usual case without new state.

Consequences:

- **git on the Server.** Clone needs git on the PATH of the Pane's shell on the Server. Without git, the Pane shows `command not found`.
- **Whose credentials.** On a remote Device the clone uses that machine's keys, SSH agent and credential helpers, from the environment the Server started with. An agent the Client forwards does not reach the Pane, and an `SSH_AUTH_SOCK` from a session that has ended no longer works.
- **Empty Workspaces.** A clone that fails or is cancelled leaves a Workspace on an empty directory.
- **A half-written `.git`.** A Server killed outright mid-clone, before its Supervisor restarts it (ADR 0058), takes git down without its cleanup. That leaves a partial `.git`, and the person removes it before cloning again.
- **History after a restart.** bash writes its history only on exit, so after a Server restart the line is not in the new shell's history. The printed command or the URL is.
- **A parent inside another repository.** Until `.git` appears, Changes and History show that outer repository.
- **Older Servers.** They answer with their generic error, as described under Older Servers.
- **The Companion.** It shows the new Workspace but creates none, as before.

Tests:

- **Directory name.** `clone_directory_name` is tested on HTTPS with and without a port, `ssh://`, scp-like and local-path URLs, `/.git`, `.git`, `.bundle` and a trailing `/`, and with a URL that yields no name.
- **Name and URL rules.** The Server refuses an existing name, separators, `.`, `..`, and on Windows `C:foo`, reserved characters, a trailing dot or space, and `CON`. It removes only a still-empty directory when the Workspace cannot be created. URLs are refused when empty, or when they hold a control character, an ASCII or Unicode quote, more than 2048 bytes, or a relative local path, all before anything is created. A URL with a space inside and a trailing newline is accepted.
- **Quoting.** fish and PowerShell quoting keep an argument holding `'`, `\'`, `\\` and U+2018 to U+201B as one word, for `condr agent start` as well.
- **Clone end to end.** In zsh and bash, cloning a local repository by its absolute path types the line once at the first prompt, into the absolute target, even when the startup file changes directory. The Workspace then gains a `.git` that the Git watcher reports. Keys typed before the prompt cancel the line and print the command.
- **Waiting for the prompt.** A zsh whose startup file blocks in `read` gets nothing typed, and the command is printed after 30 seconds. A zsh whose startup file sleeps before its first prompt gets the line after the prompt mark. A shell that never announces itself gets the line through the probe. A prompt mark that arrives before the wait begins still counts. The line is pasted bracketed when the editor turns bracketed paste on within the second. An Agent's resume and `condr agent start` on a fresh Pane go through the same wait.
- **CLI.** `workspace clone` derives the directory, resolves a relative one against the current directory, fails without a name, and prints the created Workspace and its root Pane.
- **GUI.** The chooser moves with Up and Down, takes a row with Enter, and goes back from a form with Esc. Clone's name follows the URL until it is edited. The name field marks a name the parent already lists. The default parent skips a Managed Worktree's `<repo>.worktrees/` and falls back to home.
- **By hand.** PowerShell and cmd Panes on Windows, including the OSC 9;9 prompt and a curly-quoted argument, are checked by hand, and so is fish.
