---
title: Agent automation
description: Install the Skill, set up the environment variables and use the condr CLI for multi-Agent task dispatch, automated control and cross-Device scheduling.
---

In Condr, an Agent does more than answer questions and edit code. Through the built-in `condr` command-line tool it can also drive the terminal interface in the other direction, which enables multi-Agent collaboration across Panes and automated task orchestration.

## How it works

When an Agent runs in a Condr terminal Pane, the system automatically injects the following environment variables so that Agents and scripts can recognize the Condr environment and call `condr` commands directly:

- `CONDR_ENV=1`: marks that the process is running inside a Condr Pane.
- `CONDR_PANE_ID`: the id of the current Pane.
- `CONDR_SOCKET_PATH`: the socket path used to connect to the Condr Server.
- `CONDR_BIN_PATH`: the absolute path of the `condr` executable.

Running a `condr` command directly in a Pane (such as `condr pane split`) acts on the current Workspace and Tab by default. Apart from `pane read`, which prints plain text, every command prints a JSON structure by default so that Agents can parse it easily.

## Install the Condr Skill

To help an Agent (such as Claude Code) understand how to use Condr's control commands, install the Condr Skill for it:

### One-command install

Run the following command in a terminal to install the Condr Skill globally in one step:

```sh
npx -y skills add condrdev/condr -g
```

### Export the Skill text (fallback)

If your network is isolated, or you need to inject the prompt by hand, run the following command in a terminal to print the full Skill content:

```sh
condr --skill
```

> **Tip**: If you run an Agent in a sandboxed environment, make sure the sandbox allows access to the socket path named by `CONDR_SOCKET_PATH`.

## Core control commands

### Prepare space and start an Agent
- **Create a split or a Tab**:
  ```sh
  condr pane split --direction right
  condr tab create
  ```
- **Start a specific Agent and name it**: start an Agent in an idle Pane and give it a name so you can address it later:
  ```sh
  condr agent start reviewer --kind claude --pane 12
  ```

### Send prompts and wait for responses
- **Send a prompt and wait for completion**: with the `--wait` flag, the command blocks until the Agent becomes Idle or Blocked (waiting for approval):
  ```sh
  condr agent prompt reviewer "Review the changes in src/auth.rs and list potential issues" --wait
  ```
- **Wait for a state only**:
  ```sh
  condr agent wait reviewer
  ```

### Read and drive terminals
- **Read the output of a Pane**: after the Agent finishes its task, read that Pane's terminal output:
  ```sh
  condr pane read 12 --lines 50
  ```
- **Send keys and text directly**:
  ```sh
  condr pane run 12 "npm test"        # paste the command and press Enter
  condr pane send-keys 12 enter       # send specific keys (such as enter, esc, ctrl+c)
  ```

> **Note**: Before using `agent start` and `--wait`, the Hook must be installed on the target Agent (see [Agent integrations](/docs/using/agents/)). An Agent without the Hook cannot report its state accurately, so the command times out or never returns.

### Cross-Device control
Add the `--device` flag on the command line to control a saved remote Device on another machine:

```sh
# List the saved remote Devices
condr device list

# List the Workspaces on the remote Device named build-box
condr --device build-box workspace list

# Show the Workspaces on every Device
condr workspace list --all-devices
```

`--device` goes before the command group and is followed by the Device name shown in the sidebar; once the `CONDR_DEVICE` environment variable is set, you can omit it. `server` and `agent hooks` act on this machine only and do not accept `--device`.

## Typical collaboration scenarios and prompt templates

Copy the following prompts and send them to your lead Agent to run common automated collaboration flows:

### Scenario 1: investigate in parallel, then compile a report
```text
In this Workspace, use condr to open three Panes and start a claude in each.
One traces the request's handling path, one checks test coverage, one looks for related past regressions.
None of them may modify files. Once all three Agents finish, read their output and merge it into one complete report for me.
```

### Scenario 2: write code with live review (one writes, one reviews)
```text
Use condr to open a Pane next to this one and start codex in it, named reviewer.
From now on, whenever I ask you to complete a code change, send the code diff to reviewer for review, and once it finishes, summarize its review comments and report them to me.
```

### Scenario 3: dispatch a task to a remote machine
```text
Use condr --device build-box to open a Workspace in the ~/code/app directory on that remote machine,
start claude and have it run the full test suite and fix the failures. Once it finishes, read the final result and return it.
```

### Scenario 4: automated build monitoring and response
```text
Use condr to run npm run dev in the left Pane, and monitor the left Pane's log from the right Pane with condr pane read.
As soon as a compile error is detected, extract the error message and start fixing the code.
```
