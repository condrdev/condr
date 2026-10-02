---
title: Agents
description: See supported Agents, install their hooks, and understand each state.
---

Install an Agent's hooks so the sidebar can show its state and what it needs from you. For orchestration commands, see [Agent automation](/docs/using/automation/). For command options, see the [CLI reference](/docs/reference/cli/).

## Prepare an Agent

Condr provides neither models nor accounts. Install the Agent CLI and sign in on the machine that runs the Server.

Check which Agent CLIs the Server can find:

```sh
condr agent available
```

The command lists Agents on the Server's PATH. The program that starts the Server sets that PATH. The Server does not read your shell configuration again, so an alias or path added only in `.zshrc` is invisible.

## See supported Agents

| Agent | Command | Reports state | Says what it waits for | `agent start` returns |
| --- | --- | --- | --- | --- |
| Claude Code | `claude` | Yes | Permissions and questions | After it reports Idle |
| Codex | `codex` | Yes | Permissions | Once the process is recognized |
| OpenCode | `opencode` | Yes | Permissions and questions | After it reports Idle |
| Pi | `pi` | Yes | Questions | After it reports Idle |
| Oh My Pi | `omp` | Yes | Questions | After it reports Idle |
| Antigravity | `agy` | Yes | No | Once the process is recognized |
| Grok Build | `grok` | Yes | Permissions | After it reports Idle |
| Cursor CLI | `cursor-agent` | Yes | No | Once the process is recognized |
| GitHub Copilot | `copilot` | Yes | Permissions | Once the process is recognized |
| Kimi Code | `kimi` | No | No | Once the process is recognized |

Agents that return “Once the process is recognized” report nothing before their first prompt. They show Unknown after they start. This behavior comes from their hooks, not from Condr.

Condr identifies an Agent by its process name. It also recognizes Agents started through node, bun, python, or a shell, and follows symlinks to the real target. If a wrapper hides the name, such as `mise exec -- claude`, the Agent's hook events report its identity.

## Install hooks

A hook is a small command that an Agent calls when it starts, finishes, or waits. Condr trusts only these reports. Without hooks, an Agent stays Unknown.

On the machine that runs the Server, install hooks once for each Agent:

```sh
condr agent hooks install claude
```

You can also open **Settings › Agents** and click **Install** for the selected Device. Install hooks on remote Devices there too because `agent hooks` on the command line does not accept `--device`.

The command changes only the Agent's own configuration file. It merges Condr's entries with your existing entries. If the file is damaged or larger than 4 MiB, it reports an error and writes nothing.

| Agent | File it changes |
| --- | --- |
| Claude Code | `~/.claude/settings.json` |
| Codex | `~/.codex/hooks.json`, and runs `codex features enable hooks` |
| OpenCode | `~/.config/opencode/condr-tui.js`, added to the `plugin` list in `tui.json` |
| Pi | `~/.pi/agent/extensions/condr-pi.ts` |
| Oh My Pi | `~/.omp/agent/extensions/condr-omp.ts` |
| Antigravity | `~/.gemini/antigravity-cli/plugins/condr/` |
| Grok Build | `~/.grok/hooks/condr.json` |
| Cursor CLI | `~/.cursor/hooks.json` |
| GitHub Copilot | `~/.copilot/hooks/condr.json` |
| Kimi Code | Not supported, see below |

The Agent's environment variables still apply. For example, `CLAUDE_CONFIG_DIR` and `CODEX_HOME` change where the file goes.

Check the hook with `status`, and remove it with `uninstall`:

```sh
condr agent hooks status claude
condr agent hooks uninstall claude
```

The four hook states are `installed`, `outdated` (reinstall after upgrading Condr), `missing`, and `unsupported`.

Some Agents need one more step:

- **Codex**: run `/hooks` inside Codex after installation and trust the new hooks.
- **Antigravity**: turn on the Condr plugin in its settings. It stays Unknown until the first tool call.
- **Pi** needs 0.85.1 or later. **Oh My Pi** needs 18.1.17 or later, and needs one install per profile.

Hooks work only in a Condr Pane. They do nothing when you run the Agent in another terminal.

## Read each state

| State | Meaning | Sidebar mark |
| --- | --- | --- |
| Unknown | A plain shell, or an Agent that has not reported yet | Gray info icon |
| Idle | The turn is finished and waits for your next prompt | Hollow circle |
| Working | Working | Amber |
| Blocked | Needs you | Red |
| Done | Finished while you were away | Green |

Done exists only in the window. When an Agent changes from Working or Blocked to Idle while its Pane is not focused, the sidebar shows Done until you click that Pane.

A Workspace row sums its Agent states. It shows one count each in the order bell, Blocked, Done, Working. Unknown and Idle are not counted.

## See what an Agent needs

An Agent becomes Blocked when it waits for permission or an answer. Agents that explain the reason show it on the second line of the sidebar row as the tool name and the command's first line, for example:

```text
Bash: cargo test
```

The line can also show the question. Condr cuts it at 200 characters.

You can see this reason in three places:

- The second line of the Agent's sidebar row.
- The body of the system notification.
- The **Needs you** list at the top of the sidebar. It gathers every Blocked Agent on every connected Device and disappears when none remain. Click a row to jump to its Pane.

Cursor CLI and Antigravity have no permission event, so they do not become Blocked while waiting.

## Get notifications

When a Pane is not focused, these changes send a system notification:

- The Agent finishes: “Claude finished”.
- The Agent needs you: “Claude needs your input”.

The notification body includes what the Agent waits for, the Workspace name, and the terminal title. Clicking it switches to that Pane. If the reason changes while the Agent is Blocked, Condr replaces the notification instead of stacking it.

To turn notifications off, open **Settings › Notifications** and turn off **Enable notifications**. That page also has a button for a test notification.

## Understand Agent detection

The Server reads the process table to identify the Agent in a Pane. Installed hooks run `condr agent-hook` for every event. The command writes the event into the Pane's terminal, and the Server intercepts it before terminal parsing. State therefore goes from the Agent to the Server without screen-text detection.

Nested Agents do not count. When an Agent starts another Agent as a child process, Condr tracks only the outer Agent.

## Know the limitations

- **Kimi Code** is recognized, but its hooks cannot distinguish the main task from a subtask. Condr installs no hooks, so its state stays Unknown.
- **OpenCode** uses a TUI plugin that still lacks testing on real machines. Delete the old development `plugins/condr.js` before installing.
- **Grok Build** may show completion early when other Stop hooks block.
- **GitHub Copilot** may stay Working after an API error.
- A program Condr cannot recognize is a plain terminal with state Unknown. This looks the same as a recognized Agent without hooks. Run `condr agent list` to tell them apart. The unrecognized program is not in the list.
