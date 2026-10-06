---
title: Troubleshooting
description: Check the Server status and logs first, then work through startup, connection, Agent state and configuration problems by symptom, and gather what an Issue needs.
---

When something goes wrong, start with the Server status and the relevant logs, then follow the section for your symptom.

---

## Check the Server status

On the Device that runs the Server, run:

```sh
condr server status
```

* **What it shows**: run state, socket path, Device fingerprint, version and protocol number, uptime, TCP listen address, Peer-to-peer status, and the counts of Workspaces, Tabs, Panes, Agents, windows and TCP Devices.
* **Pending**: when the saved listen address or Peer-to-peer setting differs from the running one, the output includes a `Pending` line saying the Server must restart for the change to apply.
* **Recent errors**: the latest 20 warnings and errors since this Server started, oldest first.
* **JSON output**: add `--json` for structured JSON output, handy to attach to an Issue.
* **Exit code**: when the Server is not running, the command ends with exit code 1 and prints the reason.

For a remote Device's status, go to **Settings › Device › General**.

---

## Read the logs

| Log | File name | Rotation and lifetime |
| :--- | :--- | :--- |
| Server log | `condr-server-<id>.<date>.log` | One file a day; the latest 7 log files are kept |
| Window log | `condr-gui.<date>.log` | One file a day; the latest 7 log files are kept |
| Server error output | `condr-server-<id>.stderr` | Captures the background Server process's stdout and stderr. Use it to diagnose a Server that fails to start or crashes. This file is not rotated |

For the default log directory, see [Configuration and settings](/docs/reference/configuration/). You can also open it from **Settings › Application › Developer › Locations** by clicking **Open** on the Logs row.

### Raise the log level

Run this in a terminal outside Condr:

```sh
CONDR_LOG=condr_server=debug,condr_core=debug condr server restart
```

* **Where to run it**: `condr server restart` cannot run in a Pane inside Condr. Restarting ends every active Pane, so the command is refused there.
* **Syntax**: the `CONDR_LOG` environment variable uses the `tracing` EnvFilter syntax and replaces the default filter. Setting it to just `debug` also turns on debug logging for every underlying dependency, which produces a lot of output.
* **Invalid values**: if the filter fails to parse, Condr falls back to the default filter and adds a warning to Recent errors.
* **Window log level**: the window also reads `CONDR_LOG`. To raise the window's own log level, set the variable in a terminal and start the window from there. A Server that this window starts inherits the same environment.

---

## The window cannot start the local Server

When the window connects to this machine and finds no running Server, it starts a new one and waits for its handshake. If that fails, the connection page shows:

```text
Condr's own server could not start on this device. Connect tries again.
```

The detailed reason under it includes `condr-server did not become ready`, the path of the `.stderr` file, and that file's output since this start. Common causes:

* **The TCP listen address cannot be bound**: `.stderr` shows `failed to listen on tcp://…`, and the Server does not start at all. Change the port, or remove `[server] listen` from `config.toml`.
* **Device key permissions too broad**: with a TCP listener, the Server must read this machine's Device key. On Linux and macOS, if the key file grants access to the group or other users, `.stderr` records `… is readable by other users (mode …); make it 0600 or delete it`. Follow the prompt and reset the permissions to `0600`.
* **The `condr` binary is missing**: the error says `condr is not installed beside condr-gui`. The released desktop app bundles `condr`; this usually happens when you build the window yourself without also building the `condr` CLI.

If `[server] listen` is malformed, the Server does not fail; it treats it as no listener configured. A Peer-to-peer startup failure does not bring the whole Server down either; it is written to the log as `p2p endpoint not bound`.

---

## The Server starts again after you stop it

If you run `condr server stop` while the window is open, the window notices the Server is gone within its 0.5-second reconnect cycle and immediately starts a new Server. To stop the Server for good, quit the window first, then run `condr server stop`.

---

## The two sides run different versions

* **Different builds**: an amber warning triangle appears next to the Device heading. Hover over it to compare the two versions and see which side to update. Clicking the triangle hides the notice until that Device's version changes again.
* **The local Server is older than the window**: after you update Condr, the window asks whether to restart the local Server. Restarting ends every process running in its terminals, then reopens the current Workspaces and Agent conversations.
* **Incompatible protocols**: the connection is refused, the connection page shows `Condr there and here speak different protocol versions`, and the CLI returns the error code `protocol_mismatch`. Retrying does not help; both sides must be updated to matching versions.

Once both sides run matching versions, go to the target Device and run `condr server restart` in a terminal outside Condr, or open **Settings › Device › Remote access** in the window and click **Restart Condr**. That button is available only over a local or SSH connection.

---

## Agent state stays Unknown

Agent state comes entirely from hooks. Check these in order:

1. **Are the hooks installed?** Run `condr agent hooks status <agent>` (for example, `condr agent hooks status claude`). `missing` means install them; `outdated` means reinstall them; `unsupported` means the Agent is recognition only: Condr installs no hooks for it and its state stays Unknown (see [Support levels](/docs/using/agents/#support-levels)). For a remote Device, view and install them under **Settings › Device › Agent integrations**; the command-line `agent hooks` acts on this machine only.
2. **Is the Agent running in a Condr Pane?** Hooks report only in terminal sessions where `CONDR_ENV=1` is set. Condr cannot see Agents running in other terminals.
3. **Does the Agent report at startup?** Codex, Copilot and Antigravity send no state before their first prompt, and Cursor sends none while resuming a session; during that time the state shows Unknown.
4. **Has Codex trusted the hooks?** Codex requires its hooks feature to be enabled and the hooks to be trusted explicitly. Installing Condr's hooks tries to enable the feature; if that fails, add `[features] hooks = true` to Codex's `config.toml`, then run `/hooks` inside Codex to trust them.
5. **Is the Antigravity plugin enabled?** After the hooks are installed, enable the condr plugin inside Antigravity CLI.
6. **Did the hooks go to another directory?** At install time, Condr decides where to put the hooks from environment variables such as `CLAUDE_CONFIG_DIR` and `CODEX_HOME`. The CLI uses the current terminal's environment, while Settings uses the Server process's environment. If the Agent runs with different values than at install time, it cannot find the hooks.

---

## A Pane cannot find the Agent

* **Where the Server's environment comes from**: when you open Condr from the Dock, Finder or a desktop menu, it reads your login shell's environment once at launch, so the Server it starts sees the same PATH as your terminal. A Server started with `condr server start` in a terminal uses that terminal's environment. Run `condr agent available` to list the Agents the Server can see.
* **After installing an Agent or changing PATH**: the Server keeps the environment it started with. Run `condr server restart` in a terminal that already sees the change.
* **If your login shell is slow**: Condr waits at most 5 seconds for it; past that, it keeps the bare launch environment and notes this in the GUI log.

---

## A remote Device cannot connect

Open the Device's page, or click **Details** in the disconnect notice above the Workspace, and match the advice against this table:

| Advice | Cause and what to check |
| :--- | :--- |
| `SSH could not reach the device.` | Run `ssh user@host` in a terminal. Once a plain login works, run `ssh user@host condr --version`. If it cannot find the program, put the absolute path in the address, as described below |
| `SSH reached the device, but Condr is not running there.` | Condr tried to start the Server on the remote Device but still could not connect to it. Log in to that Device, run `condr server start` and read its error output |
| `The device refused this connection.` | This machine is not paired with the target Device, its pairing was revoked, or the remote Device's key changed. Run `condr server invite` on the target Device and pair again with the new link |
| `Nothing answers at this address.` | The Server on the remote Device is not running, or the address or port is mistyped |
| `The device did not answer.` | The target Device is asleep or offline, or a firewall blocks the traffic. TCP, SSH and Peer-to-peer all give up after 10 seconds |
| `This device cannot reach that address.` | A routing problem or a VPN blocks the traffic. Check this machine's network and VPN settings |
| `The device closed the connection.` | The Server on the target Device stopped, crashed or is restarting |

### SSH cannot find `condr`

The PATH line the install script adds to your shell profile is not read by non-interactive SSH sessions. Put the absolute path of `condr` in the SSH address:

```text
ssh://user@host?bin=/home/user/.local/bin/condr
```

Condr runs `ssh` in fully non-interactive mode, so it cannot show a password prompt or let you confirm an unknown host key fingerprint. Set up key-based authentication (public keys or ssh-agent) in a terminal first, and add the host key to `known_hosts` beforehand.

### Peer-to-peer does not connect

* **Not turned on remotely**: run `condr server status` on the remote Device and check that Peer-to-peer is on.
* **The invite is no longer valid**: when the reason shows `invite unknown, used or expired`, the invite has expired or been used; generate a new one on that Device.
* **The local Server is not running**: Peer-to-peer connections go out through this machine's Server. If the reason shows `this machine's Server is unreachable`, make sure the local Server is running first.

---

## How reconnecting works

* **The first connection after startup fails**: there is no automatic retry. Click **Connect** on the Device's page.
* **An established connection drops**: the window retries every 0.5 seconds for up to 45 seconds. The disconnect notice appears only after 2 seconds of reconnecting.
* **The computer wakes from sleep**: if the heartbeat gets no answer for 10 seconds, the window treats the connection as lost and starts reconnecting.
* **The remote protocol is newer**: if the other Device keeps sending messages this build cannot parse, the window disconnects and stops retrying, with the error `this Device sends messages this build cannot read; update it`.

---

## Installing and launching

* **Windows SmartScreen blocks the installer**: click **More info** → **Run anyway**. Preview installers are not code-signed yet.
* **The Linux AppImage does not start**: AppImages need FUSE. Without FUSE, add the `--appimage-extract-and-run` argument when you run it.
* **Launching again shows no window**: Condr allows only one instance. Extra instances exit silently and do not bring the existing window forward. Check the taskbar or other virtual desktops for a running Condr window.
* **Error 448 in a Pane on Windows**: for example, pnpm reports `untrusted mount point`. The Server inherited the Redirection Guard restriction from its parent process and passed it on to every Pane process. Save your work in the Panes, then run `condr server restart` in a regular system terminal so the Server starts again in an unrestricted environment.

---

## A configuration change does not take effect

* **Reloading after hand edits**: after editing `[client]` or its sub-tables by hand, restart the window; after editing `[server]` or its sub-tables by hand, run `condr server restart` in a terminal. Settings changed in the window take effect immediately, except on the Remote access page.
* **A broken configuration file**: if the TOML has a syntax error, the window reports `Failed to load … Device list changes are disabled; fix the file and restart Condr.` on startup. Every setting falls back to its default and the Device list cannot be changed. Fix the file and restart the window to recover.
* **The editor warns the file changed on Windows**: while another editor holds `config.toml` open, Condr overwrites the file in place, and the editor usually warns that the file was changed outside it.

---

## Filing an Issue

File Issues on [GitHub Issues](https://github.com/condrdev/condr/issues), and include:

1. **Versions**: the window's version is under **Settings › Application › About**. For the running Server, use the Version field from `condr server status`; `condr --version` reports only the version of that binary, which can differ from the running Server.
2. **Status**: the output of `condr server status --json`.
3. **Logs**: log excerpts from around the failure (ideally captured after raising the log level and reproducing it).
4. **Steps to reproduce**: a clear sequence of actions.
5. **Remote Devices**: for a failure involving a remote Device, the versions on both sides and the connection type in use.

Do not report possible security vulnerabilities in a public Issue; follow the private process in [Security model](/docs/help/security/#reporting-security-issues).
