---
title: CLI reference
description: The syntax of the condr command line, its core workflows, the subcommand reference and its automation conventions.
---

`condr` is a high-performance terminal multiplexing control tool built for multi-device and AI Agent collaboration. It can be used in an interactive terminal, or as a headless script that drives back-end sessions.

Run `condr <group> <command> --help` to see each command's live help.

---

## Command format and syntax

```text
condr [--device <name>] <group> <command> [args...]
condr [options]
```

### Notation

* `<arg>`: a required argument.
* `[arg]`: an optional argument (inside a Condr Pane it usually inherits the current context).
* `a|b`: mutually exclusive options.
* `--`: everything after it is passed verbatim to the downstream process (such as the shell or Agent).

### Global options

| Option | Description |
| --- | --- |
| `--device <name>` | Targets a remote device (the name must match the sidebar configuration); may be placed before the group or at the end. Set a default with the `CONDR_DEVICE` environment variable. *Note: the `server` group and `agent hooks` do not support this option.* |
| `--skill` | Prints the bundled Agent Skill description text (for injecting straight into an AI context) without connecting to the Server process. |
| `-V, --version` | Shows version information. |
| `-h, --help` | Shows the usage. Without a subcommand, returns the help (exit code `2`). |

---

## Context and execution environment

`condr` inherits context intelligently depending on the environment it runs in:

```text
[Current environment]
 ├── Inside a Condr Pane ──► reads the CONDR_SOCKET_PATH / CONDR_PANE_ID variables
 │                           Workspace, Tab and Pane arguments are omitted by default
 │
 ├── External terminal ────► connects straight to this machine's default Server socket
 │   (Bash/Zsh)               no current context, ids must be passed explicitly
 │
 └── With --device ────────► connects to the remote device's Server
                             local context is void, paths and ids must be passed explicitly
```

> **Tip**: if the Server is not yet running, most commands do not start it implicitly; they return the standard error `server_not_running` (exit code `1`).

---

## Quick Recipes

### Split a Pane and run a task

Split a new Pane to the right in the current Tab and run a build script in it:

```bash
# Split the Pane and capture the new Pane ID
NEW_PANE=$(condr pane split --direction right | jq -r '.pane.pane_id')

# Send the command and press Enter
condr pane run "$NEW_PANE" "cargo build --release"
```

### Remote Execution

Create a workspace on the remote device named `mac-mini` and view its terminal output:

```bash
# Create the workspace remotely
condr --device mac-mini workspace create --cwd "/home/dev/project" --label "Backend"

# Read the latest 50 lines of a remote Pane's terminal output
condr --device mac-mini pane read 3 --lines 50
```

### Agent automation

Start an autonomous Agent and wait for it to finish:

```bash
# Start a Claude Agent in a given Pane
condr agent start claude-agent --kind claude --pane 1

# Send a task prompt and wait until it is ready / awaiting confirmation (120 s timeout)
condr agent prompt claude-agent "Optimize the build configuration in the current directory" --wait --timeout 120000
```

---

## Command reference

### `server`

Manages the lifecycle, listening ports and device pairing of this machine's service. This group acts on the local machine only and does not support `--device`.

```bash
# Start the Server in the background; does nothing when it is already running
# --listen and --p2p are persisted to the configuration file
condr server start [--listen <IP:PORT>] [--p2p] [--snapshot <path>]

# Restart the Server, restoring the layout from the latest snapshot; never inside a Condr Pane
condr server restart [--listen <IP:PORT>] [--p2p] [--snapshot <path>]

# Stop the Server safely; exit code 0 even when it is not running
condr server stop

# Temporary foreground run for debugging; settings apply to this run only
condr server run [--listen <IP:PORT>] [--p2p] [--snapshot <path>] [--endpoint <path>]

# Show the running state, uptime, session counts and recent errors; exit code 1 when not running
condr server status [--json]

# Install the current binary into the user's executable path and register PATH
condr server install [--start] [--restart] [--yes] [--json]

# Stop the Server and clean up the binary and PATH entry (user data and logs are kept)
condr server uninstall [--yes] [--json]

# Generate a one-time pairing link (valid for 10 minutes); listening or P2P must be enabled first
condr server invite

# List the fingerprints and connection states of all authorized remote devices
condr server clients

# Revoke a client's access and disconnect it immediately; a unique prefix is enough
condr server revoke <fingerprint or prefix>

# Bridge stdio to the local socket; called automatically by SSH, no need to run by hand
condr server bridge [--endpoint <path>]
```

---

### `workspace`

```bash
# List all workspaces
# --all-devices aggregates every remote device in parallel; unreachable ones are listed under unreachable
condr workspace list [--all-devices]

# Create a workspace; --cwd defaults to the current directory (required with --device)
condr workspace create [--cwd <path>] [--label <name>] [--focus]

# Get the workspace's metadata and its Tab and Pane counts
condr workspace get <id>

# Focus every connected GUI window on the workspace
condr workspace focus <id>

# Rename the workspace
condr workspace rename <id> <new name>

# Close the workspace and terminate all of its terminal processes (files on disk are unaffected)
condr workspace close <id>
```

---

### `tab`

```bash
# List Tabs, optionally filtered by Workspace
condr tab list [--workspace <id>]

# Create a Tab; the shell inherits the current Pane's directory by default
# --workspace is required outside Condr or when calling across machines
condr tab create [--workspace <id>] [--label <name>] [--focus]

# Query, switch focus or rename
condr tab get <id>
condr tab focus <id>
condr tab rename <id> <new name>

# Close the Tab; closing the last Tab closes its parent Workspace too
condr tab close <id>
```

---

### `pane`

*Note: when `[id]` is omitted, every command targets the Pane of the current context.*

#### Layout and management

```bash
# View all panes and their associated Agent states
condr pane list [--workspace <id>]

# Show the given pane, or the current pane
condr pane get <id>
condr pane current

# Split the pane and start the default shell in the same directory
condr pane split --direction right|down [--focus] [id]

# Focus the given Pane (and make every window show its Tab), or switch by relative direction
condr pane focus [--direction <left|right|up|down>] [id]

# Adjust an edge, by 0.05 by default
condr pane resize --direction <left|right|up|down> [--amount <ratio>] [id]

# Swap places with the adjacent pane in the given direction
condr pane swap --direction <left|right|up|down> [id]

# Re-anchor the pane's relative topology
condr pane move --to <target id> --side <left|right|up|down> [id]

# Maximize the current Pane or restore the split view
condr pane zoom [--on|--off] [id]

# Print the current Tab's split tree as JSON (including each pane's edges and neighbors)
condr pane layout [id]

# Terminate the pane's process and close it; closing the last Pane closes its Tab too
condr pane close <id>
```

#### Interaction and I/O automation

```bash
# Get the pane's terminal text (including scrollback, the last 80 lines by default)
condr pane read <id> [--lines <N>]

# Inject raw text into the terminal (without a carriage return)
condr pane send-text <id> <text>

# Send control keys or combinations: enter, esc, tab, ctrl+c, alt+shift+x, f1~f20
condr pane send-keys <id> <keys...>

# Shortcut: paste the command text into the terminal and immediately inject enter
condr pane run <id> <command>
```

---

### `agent`

`condr` natively recognizes and schedules the mainstream terminal AI Agents. `<target>` accepts the custom alias given at `agent start` (`[a-z][a-z0-9_-]{0,31}`), or the ID of the Pane it runs in.

#### Running and coordinating

```bash
# Scan and list the installed, supported Agent CLI tools on the Server's PATH
condr agent available

# Query every active Agent process, its state, blocking reason (blocked_on) and native session ID
condr agent list

# Initialize and start an Agent in an idle Pane and wait until it is ready; default timeout 30 s (30000ms)
condr agent start <name> --kind <kind> --pane <id> [--timeout <ms>] [-- extra args...]

# Issue a task prompt; --wait blocks until the Agent returns the target state (by default idle or blocked)
# --until and --timeout require --wait
condr agent prompt <target> <text> [--wait] [--until <state>] [--timeout <ms>]

# Block while waiting for the Agent's state to change (asynchronous polling, long-task sync); sends no input
# Default timeout 120 s (120000ms)
condr agent wait <target> [--until <state>] [--timeout <ms>]
```

*Supported kinds (`--kind`)*: `claude`, `codex`, `opencode`, `pi`, `omp`, `antigravity`, `grok`, `cursor`, `copilot`, `kimi`.

`kimi` is recognition only and its state stays Unknown, so `agent wait` and `agent prompt --wait` refuse it at once with `agent_reports_no_state` (see [Support levels](/docs/using/agents/#support-levels)).

#### Hook integration

```bash
# View a given Agent's state-reporting hooks: installed / outdated / missing / unsupported (recognition only)
condr agent hooks status <kind>

# Automatically configure or clean up the integration hooks for the given Agent; edits this machine only
condr agent hooks install <kind>
condr agent hooks uninstall <kind>
```

---

### `device`

```bash
# List every remote device configured in config.toml, with connectivity (reachable) and remote workspace count
condr device list
```

> Tip: adding, renaming and removing devices is best done visually in the GUI sidebar; the command line only reads that configuration.

---

## Automation and scripting conventions

### Output conventions

1. **Structured data (JSON)**: apart from `pane read`, which prints raw terminal characters, and the `server` group, which prints interactive text by default, every other subcommand **prints machine-readable standard JSON by default**. `server status`, `install` and `uninstall` also print JSON with `--json`.
2. **Error output (stderr)**: on a business-logic or network error, the command prints standard JSON to standard error (stderr):

```json
{
  "error": {
    "code": "pane_not_found",
    "message": "pane 9 not found"
  }
}
```

*Note: the `server` daemon commands report errors as interactive text (starting with `error: `).*

### Exit Codes

| Code | Type | When |
| --- | --- | --- |
| `0` | **Success** | The command ran and returned normally. |
| `1` | **Runtime Error** | Business-logic failure, resource not found, permission denied or network unreachable. Stderr carries the error. |
| `2` | **Usage Error** | A missing required argument, an unknown option or a flag that failed to parse. |

### Common Error Codes

In scripts, parse `error.code` for targeted fault handling:

| Code | Cause and suggested handling |
| --- | --- |
| `server_not_running` | The Server is not started. Bring it up first with `condr server start`. |
| `no_current_pane` | Running outside Condr, or with `--device`, without an explicit Pane ID. |
| `cwd_required` | Creating a Workspace across machines with `--device` without passing an absolute `--cwd` path. |
| `workspace_not_found` / `tab_not_found` / `pane_not_found` | The given entity ID does not exist or is already closed. |
| `pane_busy` | The target pane already holds an Agent or a pending launch, or its shell is not ready yet. |
| `agent_timeout` | The target state was not reached within the timeout set for `agent start`, `agent prompt` or `agent wait`. |
| `not_authorized` | This machine's Device key is not on the target device's paired list, or has been revoked. |
| `local_only` | Tried to run `agent hooks` remotely through `--device`; it only runs locally on the target machine. |
