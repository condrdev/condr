---
title: Troubleshooting
description: Find the first checks, fix common problems, and report the details that help.
---

Start with the Server status and logs, then follow the section that matches your problem.

## Check status and logs first

**Server status.** On the machine that runs the Server:

```sh
condr server status
```

The command shows whether the Server is running, its version, listen address, Workspace and Agent counts, connected clients, and the last 20 warnings and errors. Add `--json` to get one object for an issue. For a remote Device, see **Settings › Device › Daemon**.

**Logs.** Logs roll daily and stay for 7 days. Find their location in [Configuration and settings](/docs/reference/configuration/). **Settings › Developer › Locations** has **Open** to open the log folder. The Server log is `condr-server-<id>.<date>.log`. The window log is `condr-gui.<date>.log`. When a background Server crashes, its output is in the nearby `.stderr` file.

For more detail, restart the Server with `CONDR_LOG`:

```sh
CONDR_LOG=debug condr server restart
```

`CONDR_LOG` uses `tracing` filter syntax, such as `condr_server=trace`. The window reads the same variable. Start it from a terminal.

## The window cannot connect to the local Server

Each local connection starts a Server when none runs and waits 1.5 seconds. On timeout, the window says "condr-server did not become ready" and shows the last lines of the `.stderr` file.

Check these causes:

- **The port is taken.** If the `listen` address in the configuration cannot bind, the whole Server fails to start. The log says "failed to listen on tcp://…". Change the port or remove `listen` from `config.toml`.
- **`condr` is missing.** The message is "condr is not installed beside condr-gui". The desktop app ships with `condr`, so this usually means a self-build omitted `condr`.
- **Versions are incompatible.** The window and the running Server speak different protocols, and the message is "speak different protocol versions". Run `condr server restart` so the new version takes over.

If you stop the local Server while the window is open, the window starts a new one within 45 seconds.

## An Agent state stays Unknown

State comes only from Agent hooks. Check these items in order:

1. **Are hooks installed?** Run `condr agent hooks status claude` with your Agent. Install hooks for `missing` and reinstall for `outdated`. For a remote Device, open **Settings › Device › Agents**.
2. **Does the Agent run in a Condr Pane?** Hooks work only in a shell with `CONDR_ENV=1`. Condr cannot see an Agent in another terminal.
3. **Does the Agent report state?** Codex, Copilot, Cursor, and Antigravity report nothing before the first prompt. Kimi is not supported. See the support table in [Agents](/docs/using/agents/).
4. **Did Codex trust the hooks?** Run `/hooks` in Codex after installation.
5. **Did hooks use another folder?** Hooks find the Agent config folder from environment variables at install time, such as `CLAUDE_CONFIG_DIR`. The window and a terminal can have different environments.

## An Agent is not found in a Pane

The Server finds Agents on its own PATH. The program that starts the Server supplies that PATH. A window launched from the Dock or Start menu gets a minimal system PATH and does not read `.zshrc`.

Try one of these fixes:

- Start the Server once from a terminal with `condr server restart`. It inherits that terminal's PATH, and the window connects to it afterward.
- Install the Agent on the system PATH, such as `/usr/local/bin`.

A Pane shell is non-login. On macOS it does not read `~/.zprofile`, only `~/.zshrc`.

## A remote Device shows a different version

A yellow triangle on a Device heading means its Condr build differs. Hover to see which side to update. Install the same version on both, then run `condr server restart` on the remote Device or click **Restart Condr** in **Settings › Device › Daemon**.

A red warning that says "speak different protocol versions" means the versions are too far apart. Update both sides to the same version before connecting.

## A remote Device will not connect

Start with the hint on the Device heading or open the Device and click **Details**.

**SSH could not reach the device.** SSH did not connect, or the remote has no `condr`. Run `ssh user@host` in a terminal. After login works, run `ssh user@host condr --version`. If it says `command not found`, the install script's PATH line does not apply to a non-interactive shell. Put the path in the link:

```text
ssh://user@host?bin=/home/user/.local/bin/condr
```

Condr never shows a password prompt. Configure keys, ssh-agent, and host trust in a terminal first.

**SSH reached the device, but Condr is not running there.** The remote Server is not running, and automatic startup failed. Run `condr server start` on the remote and read its output.

**this device is not authorized.** The invite expired, was used, or this Device was revoked. Run `condr server invite` on the remote and paste the link within 10 minutes.

**Peer-to-peer will not connect, but TCP and SSH work.** The remote did not enable `--p2p`, or the relay and DNS services are down. Run `condr server status` on the remote and check that Peer-to-peer is on.

**Connecting spins for more than 10 seconds.** TCP times out after 10 seconds and SSH after 15 seconds. Check the address, port, and firewall.

A connection that fails when Condr starts is not retried automatically. Click **Connect** on the Device page.

## Condr will not install or open

- **Windows SmartScreen blocks it.** Click **More info**, then **Run anyway**. Preview installers are unsigned.
- **The Linux AppImage will not open.** It needs FUSE. Install your distribution's `libfuse2` package, or unpack it with `--appimage-extract` and run it.
- **Another Condr is already running.** The window allows one instance. A second one exits immediately.

## A configuration change has no effect

- A hand edit to a `[client]` key in `config.toml` needs a window restart. A `[server]` key needs a Server restart. Window changes take effect immediately.
- A malformed file makes every key use its default. The window says "Failed to load" at startup and disables Device-list editing.
- On Windows, when an editor locks the file, Condr overwrites it in place. The editor may report that the file changed.

## Include these details in an issue

Open an issue at [GitHub Issues](https://github.com/condrdev/condr/issues) and include:

1. The Condr version and platform. Find the window version in **Settings › About** and the Server version with `condr --version`.
2. The output of `condr server status --json`.
3. Log lines around the problem. Reproduce it once with `CONDR_LOG=debug` when possible.
4. Short reproduction steps.
5. For a remote Device, both versions and the connection type.

Do not report security problems publicly. See [Security model](/docs/help/security/#report-a-security-problem).
