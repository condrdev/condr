---
title: Agent automation
description: Let one Agent open Panes, read output, delegate work, and wait for results.
---

Use these commands to let one Agent direct other Agents or to control Condr from a script. See the [CLI reference](/docs/reference/cli/) for parameters and JSON fields.

## See how automation works

Every Pane shell has the `condr` command and these environment variables:

| Variable | Meaning |
| --- | --- |
| `CONDR_ENV=1` | This shell runs in a Condr Pane |
| `CONDR_PANE_ID` | This Pane's id |
| `CONDR_SOCKET_PATH` | How to reach the Server |
| `CONDR_BIN_PATH` | The absolute `condr` path when PATH was reset |

A `condr` command run in a Pane targets that Pane's Workspace and Tab by default. For example, `condr pane split --direction right` splits the current Pane without an id.

Every command prints JSON except `pane read`, which prints text. On failure, the command writes `{"error":{"code","message"}}` to stderr and exits with 1. Agents can read the output, and scripts can process it.

An Agent in a Pane can use these commands to open Panes, start Agents, send prompts, wait for completion, and read output. You can watch the work in the sidebar.

## Give an Agent the Condr skill

Condr includes a skill that documents every command and its caveats. Print it:

```sh
condr --skill
```

Save the output and place it where your Agent reads skills or custom instructions. For Claude Code, use `.claude/skills/condr/SKILL.md` in the project. Generate the file again after each Condr upgrade because the skill follows the version.

When an Agent runs in a Pane, `CONDR_ENV=1` tells it that `condr` commands are available. Its command sandbox must allow access to the socket named by `CONDR_SOCKET_PATH`, or the commands cannot reach the Server.

## Run common actions

**Make room.** `pane split` opens a shell beside the current one. `tab create` opens a Tab. `workspace create` opens another project. Each command returns the new object's id for later commands.

**Start an Agent.** `agent start` starts an Agent in an idle shell, gives it a name, and waits until it is ready:

```sh
condr agent start reviewer --kind claude --pane 12
```

Use the name instead of the Pane id in later commands. Put the Agent's own arguments after `--`.

**Send a prompt.** `agent prompt` sends text to an idle Agent. Add `--wait` to return only after it reaches Idle or Blocked:

```sh
condr agent prompt reviewer "Review the changes in src/auth.rs and list the problems" --wait
```

**Wait without sending.** `agent wait` waits for an Agent to reach a state. It returns the state, not the output.

**Read output.** `pane read` prints the last few dozen terminal lines, including scrollback. Use it after waiting to collect the result.

**Drive the terminal.** `pane run` pastes a command and presses Enter. `pane send-text` types without Enter. `pane send-keys` presses keys such as `enter`, `esc`, or `ctrl+c`.

Agent states come from hooks. Without hooks, an Agent stays Unknown, so `agent start` waits until it times out and `--wait` never returns. Install hooks first as described on the [Agents](/docs/using/agents/) page. Agents such as Codex that report nothing before their first prompt let `agent start` return when Condr recognizes the process. The first `agent prompt` is accepted in Unknown.

When an Agent waits for approval, `agent wait` returns `blocked`, and `blocked_on` says what it needs. Let the orchestrating Agent report that to you instead of approving for you.

## Paste these prompts

Use these prompts with your main Agent. Change the task and paths before you run them.

**Investigate in parallel, then combine:**

```text
Use condr to open three Panes in this Workspace and start a claude in each.
Have one trace the request path, one check test coverage, and one look for related past regressions.
None of them may edit files. When all three finish, read their output and merge it into one report for me.
```

**One writes, one reviews:**

```text
Use condr to open a Pane next to this one and start codex, named reviewer.
Each time I ask you to finish a change, send the diff to reviewer, wait for it to finish, and summarize its feedback for me.
```

**Hand work to another machine:**

```text
Use condr --device build-box to open a Workspace in ~/code/app on that machine,
start claude, and have it run the full test suite and fix the failures. When it finishes, read the result back.
```

## Run commands across Devices

Saved remote Devices are also available on the command line:

```sh
condr device list
condr --device build-box workspace list
condr workspace list --all-devices
```

Put `--device` before the command group and follow it with the Device name from the sidebar. Set the default with `CONDR_DEVICE`. Each call opens its own connection, so result ids belong to that Device. Use the same `--device` for follow-up commands. Defaults such as `CONDR_PANE_ID` do not apply there, so give explicit targets.

`server` and `agent hooks` act only on this machine and do not accept `--device`.
