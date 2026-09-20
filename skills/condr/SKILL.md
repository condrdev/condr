---
name: condr
description: Operate Condr sessions through the condr CLI, including workspaces, tabs, terminal panes, and coding agents.
---

# Condr

Condr organizes Workspaces into Tabs and terminal Panes. Each Pane runs a shell, command, or agent CLI.

If a child shell resets PATH, invoke `"$CONDR_BIN_PATH"` instead of `condr` (PowerShell: `& $env:CONDR_BIN_PATH`). Panes inherit this absolute executable path from the Server.

Discover syntax with `condr --help`, `condr <group> --help`, or `condr <group> <command> --help`. Always include `--help` when exploring: `workspace create` alone creates a Workspace.

## Commands

All commands below start with `condr`.

| Command | Purpose |
| --- | --- |
| `workspace`, `tab` | `list`/`get` inspect; `create` opens a shell; `focus` asks every GUI to show it; `rename` changes the label; `close` stops all contained terminals. |
| `pane list`, `pane get`, `pane current` | Inspect Panes or identify the caller. |
| `pane layout` | Inspect splits, dimensions, and neighbors. |
| `pane split` | Open a sibling shell right or down, inheriting cwd. |
| `pane focus`, `pane resize`, `pane swap`, `pane zoom` | Focus (by id also shows its Tab in every GUI), resize, exchange neighbors, or toggle full-Tab view. |
| `pane move` | Detach a Pane and reattach it beside another Pane (`--to <id> --side left\|right\|up\|down`); this is how the split tree is reshaped. |
| `pane close` | Stop and remove a Pane; the last Pane also closes its Tab. |
| `pane read` | Read recent terminal rows, including scrollback. |
| `pane send-text`, `pane send-keys`, `pane run` | Type without Enter, send keys, or submit a shell command. |
| `agent available` | Find supported agents on the Server's PATH. |
| `agent list` | List running agents and pending launches, with names, Pane IDs, and hook-reported native `session_id`. |
| `agent start` | Start a named agent in an idle shell and await readiness. Requires `--kind` and `--pane`; native arguments follow `--`. |
| `agent prompt` | Prompt an idle agent; `--wait` awaits its next settled state. |
| `agent wait` | Await a state without sending input. |
| `agent hooks` | `install`, `uninstall` or `status` the hooks that report an agent's state; states stay `unknown` until installed. |
| `server start`, `server status`, `server run` | Start a background Server, check it (`status --json`: uptime, Workspace/Tab/Pane/Agent counts, clients, recent errors), or run it in the foreground. |
| `server stop`, `server restart` | Stop all terminals; restart restores layout/cwd and resumes saved native conversations in fresh shells. Run restart from outside Condr. |
| `server invite`, `server clients`, `server revoke` | Pair remote devices, list paired devices, or revoke access. |
| `device list` | List the remote Devices saved on this machine, whether each answers, and its Workspace count. |
| `--device <name>` (before any group) | Run the command on a saved Device instead of the local Server; `CONDR_DEVICE` sets the default. `workspace list --all-devices` lists every Device's Workspaces, each tagged `device`, plus `unreachable` Devices. |

## Targets and Results

- Inside Condr (`CONDR_ENV=1`), `CONDR_SOCKET_PATH` selects the Server; `CONDR_PANE_ID` identifies the caller. The command sandbox must permit access to this socket. Omitted optional Pane targets mean the caller; use explicit IDs for other Panes.
- Devices are the machines the GUI connected to and saved; names match the GUI sidebar. `--device` connects to that machine's Server for one command, so IDs and names in the result belong to that Device: pass the same `--device` to every follow-up (`workspace create`, `agent start`, `agent prompt`, `agent wait`, `pane read`). Without `--device`, results with no `device` field are local. Caller defaults (`CONDR_PANE_ID`) do not apply on another Device; give explicit targets.
- Read IDs from responses: creation returns `.workspace`, `.tab`, and/or `.root_pane`; splitting returns `.pane.pane_id`. Each GUI shows what it chose; create/split leave every GUI where it is unless `--focus` is requested, which asks all of them to show the result. `tab create` without `--workspace` needs to run inside a Pane.
- Resume requires installed hooks and the native transcript on the Server. Hook reports save resume IDs; any observed process exit clears its ID. Server shutdown saves the existing IDs without an extra process check. Resume reopens the conversation without sending a prompt. Resume submits once; read any failure in the Pane and retry with the native resume command. OpenCode uses a TUI plugin installed with `agent hooks install opencode`; remove an old development `plugins/condr.js` before installing it.
- Agent targets are names assigned by `start` or numeric Pane IDs. Names match `[a-z][a-z0-9_-]{0,31}`, are unique, and last until exit or Server restart.
- Workspace/Tab/Pane/Agent commands return JSON, except `pane read` returns text. Their failures emit `.error.code` and `.error.message` JSON to stderr, exiting 1; usage errors exit 2. Server administration uses text.
- `prompt --wait` and `wait` default to `idle` or `blocked`; `--until` selects states, `--timeout` is milliseconds. `blocked` needs input; `unknown` is not ready. Wait returns state; use `pane read` for output.
- Codex reports nothing until its first turn: `agent start --kind codex` returns `unknown` once the process is identified, and the first `agent prompt` is accepted in that state; check `pane read` first if a trust or hooks-review dialog might be showing.
- Blocked starts return `agent_not_ready` and retain the name. Timeouts leave processes running; startup timeouts release pending names. Inspect `agent list` and `pane read` before retrying; relay approvals or questions requiring the user's decision.
