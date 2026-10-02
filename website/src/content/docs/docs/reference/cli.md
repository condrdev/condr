---
title: CLI reference
description: Look up every condr subcommand, parameter, JSON output, and exit code.
---

Find a command here, then run `condr <group> <command> --help` for its complete parameters. The help output is authoritative.

## Use the command format

```text
condr [--device <name>] <group> <command> [args]
```

- **`--device <name>`** runs the command on a saved remote Device. The name matches the sidebar. Set the default with `CONDR_DEVICE`. `server` and `agent hooks` do not accept it.
- **`--skill`** prints the bundled Agent skill without connecting to a Server.
- **Inside a Pane**, optional Pane, Tab, and Workspace arguments default to the current Pane.
- **Output**: `workspace`, `tab`, `pane`, `agent`, and `device` print JSON. `pane read` prints text. `server` prints text for people.
- **Exit codes**: 0 means success. 1 means failure, with `{"error":{"code","message"}}` on stderr. 2 means a usage error. These commands never start a Server. When no Server runs, the error code is `server_not_running`.
- **Common error codes**: `server_not_running`, `protocol_mismatch`, `not_authorized`, `device_not_found`, `workspace_not_found`, `pane_not_found`, `agent_unavailable`, `agent_not_ready`, `local_only`.

## Manage the Server with `condr server`

| Command | What it does |
| --- | --- |
| `start` | Starts the Server in the background, or does nothing if it already runs. `--listen <IP:port>` and `--p2p` are written to the configuration and apply from now on. |
| `restart` | Stops and starts. Ends every program in every Pane and restores the structure from the snapshot. Cannot run inside a Condr Pane. |
| `stop` | Asks the Server to shut down. Returns 0 even when nothing runs. |
| `run` | Runs in the foreground. `--listen` and `--p2p` apply only to this run and are not written to the configuration. |
| `status` | Shows whether it runs, its uptime, Workspace/Tab/Pane/Agent counts, connected client count, and recent errors. `--json` prints one object. |
| `install` | Copies the current executable into the user's directory and adds it to PATH. `--start` starts it after installation. `--restart` lets the new version take over. |
| `uninstall` | Stops the Server and removes the installed `condr` and its PATH entry. Keeps configuration, data, and logs. |
| `invite` | Prints a one-time pairing link valid for 10 minutes. Turn on the TCP listener or Peer-to-peer first. |
| `clients` | Lists paired Devices: name, last connection time, and fingerprint. |
| `revoke <fingerprint>` | Revokes a Device and drops its connections. A unique fingerprint prefix also works. The match is case-sensitive. |
| `bridge` | Connects stdin/stdout to this machine's Server socket. SSH connections run it on the remote. You do not call it yourself. |

Fields of `status --json`: `running`, `endpoint`, `fingerprint`, `version`, `protocol`, `listen`, `p2p`, `connected_devices`, `uptime_secs`, `workspaces`, `tabs`, `panes`, `agents`, `clients`, `recent_errors`.

## Manage Workspaces with `condr workspace`

| Command | What it does |
| --- | --- |
| `list` | Lists every Workspace. `--all-devices` adds every saved Device. Each entry carries a `device` field. Unreachable Devices appear under `unreachable`. |
| `create` | Opens a Workspace. `--cwd` defaults to the current directory, `--label` to the directory name, and `--focus` asks every window to show it. |
| `get <id>` | Shows one Workspace. A worktree includes a `worktree` object. |
| `focus <id>` | Asks every connected window to show it. |
| `rename <id> <label>` | Renames it. |
| `close <id>` | Closes it and stops all its terminals. Deletes no files. |

## Manage Tabs with `condr tab`

| Command | What it does |
| --- | --- |
| `list` | Lists Tabs. `--workspace <id>` limits the result to one Workspace. |
| `create` | Creates a Tab in the current Pane's Workspace by default. Its shell starts in the current Pane's directory. `--label` names it. `--focus` switches to it. |
| `get <id>` | Shows one Tab. |
| `focus <id>` | Asks every window to show it. |
| `rename <id> <label>` | Renames it. |
| `close <id>` | Closes it and stops its terminals. Closing the last Tab closes the Workspace too. |

## Manage Panes with `condr pane`

Commands that omit `<id>` act on the current Pane.

| Command | What it does |
| --- | --- |
| `list` | Lists Panes with Agent state and `blocked_on`. `--workspace <id>` limits the scope. |
| `get <id>` | Shows one Pane. |
| `current` | Shows the current Pane. |
| `layout [id]` | Describes the Pane's Tab: the split tree, every Pane's edges, and its neighbors. |
| `split --direction right\|down [id]` | Splits. The new shell starts in the same directory. `--focus` gives the new Pane focus. |
| `focus [id]` | Focuses a Pane and asks every window to show its Tab. `--direction` focuses that direction's neighbor instead. |
| `resize --direction <dir> [id]` | Moves one edge. `--amount` is a fraction of the split, default 0.05. |
| `swap --direction <dir> [id]` | Swaps with the neighbor. |
| `move --to <id> --side <dir> [id]` | Detaches the Pane and attaches it to one side of another Pane in the same Tab. |
| `zoom [id]` | Zooms to fill the Tab, or returns it. `--on` and `--off` set the direction. |
| `close <id>` | Closes the Pane and stops its terminal. Closing the last Pane closes the Tab too. |
| `read <id>` | Prints the last lines as text, including scrollback. `--lines` defaults to 80. |
| `send-text <id> <text>` | Types the text as given, without Enter. |
| `send-keys <id> <keys>...` | Presses keys: `enter`, `esc`, `tab`, `up`, `f5`, `ctrl+c`, `alt+shift+x`, `a`. |
| `run <id> <command>` | Pastes a command and presses Enter. |

## Manage Agents with `condr agent`

An Agent target is the name given by `start` or a Pane id. Names must match `[a-z][a-z0-9_-]{0,31}` and last until the Agent exits or the Server restarts.

| Command | What it does |
| --- | --- |
| `available` | Lists Agents found on the Server's PATH. |
| `list` | Lists recognized Agents: name, Pane, state, `blocked_on`, and native session id. |
| `start <name> --kind <kind> --pane <id> [-- args]` | Starts an Agent in an idle shell and waits until it is ready. `--timeout` is in milliseconds, default 30000. |
| `prompt <target> <text>` | Sends a prompt. `--wait` waits for Idle or Blocked. `--until` changes the state to wait for. `--timeout` is in milliseconds. |
| `wait <target>` | Waits for a state without sending input. Defaults to Idle or Blocked. `--timeout` defaults to 120000. |
| `hooks install\|uninstall\|status <kind>` | Installs, removes, or checks hooks. Acts on this machine only. |

Values for `--kind`: `claude`, `codex`, `opencode`, `pi`, `omp`, `antigravity`, `grok`, `cursor`, `copilot`, `kimi`.

## List Devices with `condr device`

| Command | What it does |
| --- | --- |
| `list` | Lists saved remote Devices: name, address, whether each answers now, and its Workspace count. |
