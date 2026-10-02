---
title: CLI reference
description: Look up condr's subcommands, flags, JSON output and exit codes.
---

:::caution[Under construction]
This page is still being written. Content will follow.
:::

Find the command here first, then run `condr <group> <command> --help` for the full set of flags. The help output is the authoritative description.

## Use the command format

```text
condr [--device <name>] <group> <command> [args]
```

- **`--device <name>`** makes the command act on a saved remote Device; the name matches the sidebar. Set a default with `CONDR_DEVICE`. `server` and `agent hooks` do not accept it.
- **`--skill`** prints the built-in Agent Skill without connecting to the Server.
- **When run inside a Pane**, optional Pane, Tab and Workspace arguments default to the current Pane.
- **Output**: `workspace`, `tab`, `pane`, `agent` and `device` print JSON. `pane read` prints text. `server` prints human-readable text.
- **Exit codes**: 0 means success; 1 means failure and writes `{"error":{"code","message"}}` to stderr; 2 means a usage error. These commands never start the Server. When the Server is not running, the error code is `server_not_running`.
- **Common error codes**: `server_not_running`, `protocol_mismatch`, `not_authorized`, `device_not_found`, `workspace_not_found`, `pane_not_found`, `agent_unavailable`, `agent_not_ready`, `local_only`.

## Manage the Server with `condr server`

| Command | What it does |
| --- | --- |
| `start` | Starts the Server in the background, and does nothing if it is already running. `--listen <IP:port>` and `--p2p` are written to the configuration and take effect immediately. |
| `restart` | Stops and starts again, ends the programs in every Pane, and restores the structure from the snapshot. Cannot be run inside a Condr Pane. |
| `stop` | Asks the Server to shut down. Returns 0 even when it is not running. |
| `run` | Runs in the foreground. `--listen` and `--p2p` apply to this run only and are not written to the configuration. |
| `status` | Shows the running state, uptime, the number of Workspaces/Tabs/Panes/Agents, the number of connected clients and recent errors. When a saved listen address or Peer-to-peer setting is waiting for a restart to take effect, an extra Pending line appears. `--json` prints one object. |
| `install` | Copies the current executable into the user directory and adds it to PATH. `--start` starts it after installing, `--restart` lets the new version take over. |
| `uninstall` | Stops the Server and removes the installed `condr` and the PATH entry, keeping configuration, data and logs. |
| `invite` | Prints a one-time pairing link valid for 10 minutes. Enable TCP listening or Peer-to-peer first. |
| `clients` | Lists paired Devices: name, last connection time and fingerprint. |
| `revoke <fingerprint>` | Revokes a Device and disconnects it. A unique prefix of the fingerprint also works; it is case-sensitive. |
| `bridge` | Connects stdin/stdout to this machine's Server socket. SSH connections run it on the remote side; you never need to call it yourself. |

Fields of `status --json`: `running`, `endpoint`, `fingerprint`, `version`, `protocol`, `listen`, `p2p`, `connected_devices`, `uptime_secs`, `workspaces`, `tabs`, `panes`, `agents`, `clients`, `recent_errors`.

## Manage Workspaces with `condr workspace`

| Command | What it does |
| --- | --- |
| `list` | Lists every Workspace. `--all-devices` adds every saved Device; each entry carries a `device` field, and the ones that cannot be reached are listed under `unreachable`. |
| `create` | Opens a Workspace. `--cwd` defaults to the current directory, `--label` defaults to the directory name, and `--focus` makes every window show it. |
| `get <id>` | Shows one Workspace. A worktree carries a `worktree` object. |
| `focus <id>` | Makes every connected window show it. |
| `rename <id> <name>` | Renames it. |
| `close <id>` | Closes the Workspace and stops every terminal, without deleting files. |

## Manage Tabs with `condr tab`

| Command | What it does |
| --- | --- |
| `list` | Lists Tabs. `--workspace <id>` limits it to one Workspace. |
| `create` | Creates a Tab, by default in the Workspace of the current Pane, with the shell started from the current Pane's directory. `--label` names it, `--focus` switches to it. |
| `get <id>` | Shows one Tab. |
| `focus <id>` | Makes every window show it. |
| `rename <id> <name>` | Renames it. |
| `close <id>` | Closes the Tab and stops its terminals. Closing the last Tab also closes the Workspace. |

## Manage Panes with `condr pane`

Commands that omit `<id>` act on the current Pane by default.

| Command | What it does |
| --- | --- |
| `list` | Lists Panes with their Agent state and `blocked_on`. `--workspace <id>` limits the scope. |
| `get <id>` | Shows one Pane. |
| `current` | Shows the current Pane. |
| `layout [id]` | Describes the Tab the Pane is in: the split tree, and each Pane's bounds and neighbors. |
| `split --direction right\|down [id]` | Splits, with the new shell in the same directory. `--focus` gives the new Pane focus. |
| `focus [id]` | Focuses the Pane and makes every window show its Tab. `--direction` focuses the neighbor in that direction instead. |
| `resize --direction <direction> [id]` | Moves one edge. `--amount` is a split ratio, default 0.05. |
| `swap --direction <direction> [id]` | Swaps with a neighbor. |
| `move --to <id> --side <direction> [id]` | Detaches the Pane within the same Tab and attaches it to one side of another Pane. |
| `zoom [id]` | Zooms to the whole Tab or restores. `--on` and `--off` set the direction. |
| `close <id>` | Closes the Pane and stops its terminal. Closing the last Pane also closes the Tab. |
| `read <id>` | Prints the last lines of text, including scrollback. `--lines` defaults to 80. |
| `send-text <id> <text>` | Types the text as is, without Enter. |
| `send-keys <id> <key>...` | Presses keys: `enter`, `esc`, `tab`, `up`, `f5`, `ctrl+c`, `alt+shift+x`, `a`. |
| `run <id> <command>` | Pastes the command and presses Enter. |

## Manage Agents with `condr agent`

An Agent target is either the name given to `start` or a Pane id. A name must match `[a-z][a-z0-9_-]{0,31}` and lasts until the Agent exits or the Server restarts.

| Command | What it does |
| --- | --- |
| `available` | Lists the Agents found on the Server's PATH. |
| `list` | Lists the recognized Agents: name, Pane, state, `blocked_on` and native session id. |
| `start <name> --kind <kind> --pane <id> [-- args]` | Starts an Agent in an idle shell and waits until it is ready. `--timeout` is in milliseconds, default 30000. |
| `prompt <target> <text>` | Sends a prompt. `--wait` waits until Idle or Blocked, `--until` changes the state waited for, `--timeout` is in milliseconds. |
| `wait <target>` | Waits for a state without sending input. Waits for Idle or Blocked by default; `--timeout` defaults to 120000. |
| `hooks install\|uninstall\|status <kind>` | Installs, removes or checks the hook, on this machine only. |

Values of `--kind`: `claude`, `codex`, `opencode`, `pi`, `omp`, `antigravity`, `grok`, `cursor`, `copilot`, `kimi`.

## List Devices with `condr device`

| Command | What it does |
| --- | --- |
| `list` | Lists the saved remote Devices: name, address, whether it currently responds, and the number of Workspaces. |
