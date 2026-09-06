---
name: condr
description: Operate Condr sessions through the condr CLI, including workspaces, tabs, terminal panes, and coding agents.
---

# Condr

Condr organizes Workspaces into Tabs and terminal Panes. Each Pane runs a shell, command, or agent CLI.

Discover syntax with `condr --help`, `condr <group> --help`, or `condr <group> <command> --help`. Always include `--help` when exploring: `workspace create` alone creates a Workspace.

## Commands

All commands below start with `condr`.

| Command | Purpose |
| --- | --- |
| `workspace`, `tab` | `list`/`get` inspect; `create` opens a shell; `focus` selects; `rename` changes the label; `close` stops all contained terminals. |
| `pane list`, `pane get`, `pane current` | Inspect Panes or identify the caller. |
| `pane layout` | Inspect splits, dimensions, and neighbors. |
| `pane split` | Open a sibling shell right or down, inheriting cwd. |
| `pane focus`, `pane resize`, `pane swap`, `pane zoom` | Select, resize, exchange neighbors, or toggle full-Tab view. |
| `pane close` | Stop and remove a Pane; the last Pane also closes its Tab. |
| `pane read` | Read recent terminal rows, including scrollback. |
| `pane send-text`, `pane send-keys`, `pane run` | Type without Enter, send keys, or submit a shell command. |
| `agent available` | Find supported agents on the Server's PATH. |
| `agent list` | List running agents and pending launches, with names and Pane IDs. |
| `agent start` | Start a named agent in an idle shell and await readiness. Requires `--kind` and `--pane`; native arguments follow `--`. |
| `agent prompt` | Prompt an idle agent; `--wait` awaits its next settled state. |
| `agent wait` | Await a state without sending input. |
| `server start`, `server status`, `server run` | Start a background Server, check availability, or run it in the foreground. |
| `server stop`, `server restart` | Stop all terminals; restart restores layout and cwd with fresh shells. Run restart from outside Condr. |
| `server invite`, `server clients`, `server revoke` | Pair remote devices, list paired devices, or revoke access. |

## Targets and Results

- Inside Condr (`CONDR_ENV=1`), `CONDR_SOCKET_PATH` selects the Server; `CONDR_PANE_ID` identifies the caller. Omitted optional Pane targets mean the caller; use explicit IDs for other Panes.
- Read IDs from responses: creation returns `.workspace`, `.tab`, and/or `.root_pane`; splitting returns `.pane.pane_id`. Create/split preserve GUI focus unless `--focus` is requested.
- Agent targets are names assigned by `start` or numeric Pane IDs. Names match `[a-z][a-z0-9_-]{0,31}`, are unique, and last until exit or Server restart.
- Workspace/Tab/Pane/Agent commands return JSON, except `pane read` returns text. Their failures emit `.error.code` and `.error.message` JSON to stderr, exiting 1; usage errors exit 2. Server administration uses text.
- `prompt --wait` and `wait` default to `idle` or `blocked`; `--until` selects states, `--timeout` is milliseconds. `blocked` needs input; `unknown` is not ready. Wait returns state; use `pane read` for output.
- Blocked starts return `agent_not_ready` and retain the name. Timeouts leave processes running; startup timeouts release pending names. Inspect `agent list` and `pane read` before retrying; relay approvals or questions requiring the user's decision.
