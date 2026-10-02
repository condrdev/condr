---
title: Troubleshooting
description: Find where to look first, fix common problems, and gather the details for an issue.
---

Check the Server status and logs first, then follow the steps for your kind of problem.

## Check the status and logs first

**Server status.** On the machine that runs the Server, run:

```sh
condr server status
```

The command shows whether the Server is running, its version, its listen address, the number of Workspaces and Agents, the number of connected clients, and the 20 most recent warnings and errors. When a saved listen address or Peer-to-peer setting has not taken effect yet, an extra Pending line appears. Add `--json` to get a single object you can attach to an issue. A remote Device's status also appears in **Settings › Device › General**.

**Logs.** Logs roll over daily and are kept for 7 days. For their location, see [Configuration and settings](/docs/reference/configuration/). **Settings › Developer › Locations** → **Open** opens the log directory. The Server log is `condr-server-<id>.<date>.log`, and the window log is `condr-gui.<date>.log`. When a background Server crashes, its output is in the `.stderr` file next to it.

For more detail, restart the Server with `CONDR_LOG`:

```sh
CONDR_LOG=debug condr server restart
```

`CONDR_LOG` uses the `tracing` filter syntax, for example `condr_server=trace`. The window reads this variable too, so start the window from a terminal.

## The window cannot connect to the local Server

Each time the window connects locally and finds no Server running, it starts one and waits 1.5 seconds. If that times out, it shows "condr-server did not become ready" along with the last few lines of the `.stderr` file.

Check for these causes:

- **The port is in use.** When the `listen` address in the config cannot be bound, the whole Server fails to start. The log shows "failed to listen on tcp://…". Change the port, or remove `listen` from `config.toml`.
- **`condr` is not found.** The message is "condr is not installed beside condr-gui". The desktop app ships with `condr`, so this usually means your own build did not build `condr`.
- **The versions are incompatible.** The window and the running Server use different protocols, and the message says "speak different protocol versions". Run `condr server restart` so the new version takes over.

If you stop the local Server while the window is open, the window starts a new Server within 45 seconds.

## Agent state stays Unknown

State comes only from the Agent hook. Check in this order:

1. **Is the hook installed?** Run `condr agent hooks status claude`, with your own Agent in place of `claude`. `missing` means you need to install it, and `outdated` means you need to reinstall it. For a remote Device, check **Settings › Device › Agent integrations**.
2. **Is the Agent running in a Condr Pane?** The hook works only in a shell where `CONDR_ENV=1` is set. Condr cannot see Agents in other terminals.
3. **Does the Agent report state?** Codex, Copilot, Cursor and Antigravity report nothing before the first prompt. Kimi is not supported. See the support table in [Agent integrations](/docs/using/agents/).
4. **Does Codex trust the hook?** After installing, run `/hooks` in Codex.
5. **Did the hook go to another directory?** At install time, the hook finds the Agent's config directory from environment variables such as `CLAUDE_CONFIG_DIR`. The window's environment may differ from the terminal's.

## A Pane cannot find the Agent

The Server looks up Agents with its own PATH, which comes from the program that started the Server. A window launched from the Dock or the Start menu gets the system's minimal PATH and does not read `.zshrc`.

Try the following:

- Run `condr server restart` from a terminal to start the Server once. It inherits the terminal's PATH, and the window then connects to it.
- Install the Agent on the system PATH, for example in `/usr/local/bin`.

The shell in a Pane is a non-login shell. On macOS it does not read `~/.zprofile`, only `~/.zshrc`.

## A remote Device shows a different version

A yellow triangle on a Device's title means the two sides run different Condr builds. Hover over it to see which side to update. Once both sides have the same version installed, run `condr server restart` on the remote Device, or use **Settings › Device › General** → **Restart Condr**.

The red warning "speak different protocol versions" means the versions are too far apart. You must update both sides to the same version before you can connect.

## A remote Device cannot connect

First check the hint on the Device's title, or open the Device and click **Details**.

**SSH could not reach the device.** SSH did not connect, or the remote machine has no `condr`. Run `ssh user@host` in a terminal, and once that logs in, run `ssh user@host condr --version`. If it says `command not found`, the PATH the install script added does not apply to non-interactive shells, so give the path in the link:

```text
ssh://user@host?bin=/home/user/.local/bin/condr
```

Condr does not prompt for a password. Set up keys, ssh-agent and host trust in a terminal first.

**SSH reached the device, but Condr is not running there.** The remote Server is not running, and starting it automatically also failed. Run `condr server start` on the remote machine and read its output.

**this device is not authorized.** The invite has expired or been used, or the Device has been revoked. Run `condr server invite` again on the remote machine and paste the link within 10 minutes.

**Peer-to-peer cannot connect, but TCP and SSH work.** The remote machine has not enabled `--p2p`, or the relay and DNS services are temporarily unavailable. Run `condr server status` on the remote machine to confirm that Peer-to-peer is on.

**Connecting spins for more than 10 seconds.** The TCP timeout is 10 seconds and the SSH timeout is 15 seconds. Check the address, the port and the firewall.

A connection that fails on its first attempt when Condr starts is not retried automatically. Click **Connect** on the Device page.

## Condr will not install or open

- **Windows SmartScreen blocks it.** Click **More info** → **Run anyway**. Preview installers are not signed.
- **The Linux AppImage does not open.** It needs FUSE. Install your distribution's `libfuse2` package, or unpack it with `--appimage-extract` and run it.
- **Another Condr is already running.** The window allows only one instance, and a second one exits immediately.

## A config change does not take effect

- After you edit `[client]` keys in `config.toml` by hand, restart the window. After you edit `[server]` keys, restart the Server. Changes made in the interface take effect immediately.
- When the file is malformed, every key uses its default value. The window shows "Failed to load" at startup and disables editing of the Device list.
- On Windows, when an editor has the file locked, Condr overwrites it in place, and the editor may warn that the file has changed.

## Include this when you open an issue

Open an issue in [GitHub Issues](https://github.com/condrdev/condr/issues) and include:

1. Your Condr version and platform. The window version is in **Settings › About**. For the Server version, run `condr --version`.
2. The output of `condr server status --json`.
3. Log excerpts from before and after the problem. If you can, reproduce it once with `CONDR_LOG=debug`.
4. Short steps to reproduce.
5. For a remote Device, the version on each side and the connection type.

Do not report security problems publicly. See [Security model](/docs/help/security/#report-a-security-problem).
