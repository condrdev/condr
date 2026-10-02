---
title: Configuration and settings
description: Change window and Server settings, and find Condr's files on each platform.
---

Use this page to find a setting, know when it takes effect, and locate Condr's files.

## Change settings in the window

Press Cmd+, on macOS or Ctrl+, to open Settings. The window has two tabs. **Application** contains this window's settings. **Device** contains one Device's Server settings. Select the Device in the tab bar.

Each change takes effect immediately and is written to the configuration file. Text fields save when you leave them, press Enter, or click Save.

**Application**

| Page | Settings |
| --- | --- |
| Appearance | Theme (system, light, dark), terminal font, font size, color scheme |
| Notifications | Turn system notifications on or off, send a test notification |
| Power | Keep the screen awake. The coffee cup in the sidebar footer is the same switch |
| Shortcuts | The list of shortcuts, read-only |
| Developer | FPS monitor, buttons that open the application, config, state, and log folders |
| About | Version, update channel, automatic update checks, check now |

**Device**

| Page | Settings |
| --- | --- |
| Terminal | The shell for new Panes. Leave it empty to use the system default |
| Daemon | Status, connection kind, version, uptime, counts, recent errors, TCP listener switch and address, Peer-to-peer switch, restart the Server |
| Paired devices | Generate an invite, the list of paired Devices, with Revoke |
| Agents | Hook status for each Agent, with install, update, and uninstall |

You can change the Network settings in Daemon and the Paired devices page only over a local or SSH connection. TCP and Peer-to-peer connections show them as read-only.

## Where the files are

| Contents | Linux | macOS | Windows |
| --- | --- | --- | --- |
| Config: `config.toml`, keys | `~/.config/condr` | `~/Library/Application Support/condr` | `%APPDATA%\condr` |
| State: snapshot, window state | `~/.local/state/condr` | `~/Library/Application Support/condr` | `%LOCALAPPDATA%\condr` |
| Logs | `~/.local/state/condr` | `~/Library/Logs/condr` | `%LOCALAPPDATA%\condr` |
| Runtime: socket, temporary files | `$XDG_RUNTIME_DIR/condr` | `$TMPDIR/condr` | `%LOCALAPPDATA%\condr\runtime` |
| The installed `condr` command | `~/.local/opt/condr` | `~/.local/opt/condr` | `%LOCALAPPDATA%\Programs\Condr` |

On Linux, `XDG_CONFIG_HOME` and `XDG_STATE_HOME` apply as usual. **Settings › Developer › Locations** has buttons that open these folders.

The window and the command line share `config.toml`, which you can edit by hand. Condr preserves comments when it writes and uses a lock so two processes never write at once. If the file is malformed, every key falls back to its default.

A hand edit to a `[client]` key needs a window restart. A hand edit to a `[server]` key needs a Server restart. Changes made in the window take effect immediately.

## Configure `[server]`

```toml
[server]
listen = "0.0.0.0:2637"
worktree_root = "~/worktrees"
```

| Key | Meaning | Takes effect |
| --- | --- | --- |
| `listen` | An extra TCP address to listen on, as IP and port only. Unset means no listener. `condr server start --listen` writes it. | Server restart |
| `worktree_root` | Where Condr puts the worktrees it creates. Unset means beside the repository in `<repo>.worktrees/`. Absolute or `~`: `<root>/<repo>/<branch>`. Relative: `<repo>/<root>/<branch>`. | Server restart |

## Configure `[server.p2p]`

```toml
[server.p2p]
enabled = true
```

`enabled` lets the Server accept Peer-to-peer connections. It is off by default. `condr server start --p2p` writes it. The change takes effect after a Server restart.

## Configure `[server.terminal]`

```toml
[server.terminal]
shell = "/opt/homebrew/bin/fish"
```

`shell` is the program a new Pane starts. When empty, Unix uses `$SHELL`. Windows looks for `pwsh.exe`, then `powershell.exe`, then `%ComSpec%`. A change made in the window applies to the next new Pane. A hand edit needs a Server restart.

## Configure `[client]`

```toml
[client]
appearance = "dark"
notifications = true
keep_awake = false
fps_monitor = false
editor = "zed"
```

| Key | Meaning | Default |
| --- | --- | --- |
| `appearance` | `system`, `light`, or `dark` | `system` |
| `notifications` | Send a system notification when an Agent finishes or needs you | `true` |
| `keep_awake` | Keep the screen from sleeping | `false` |
| `fps_monitor` | Show the frame rate | `false` |
| `editor` | The last "Open in" target you used | none |

## Configure `[client.terminal]`

```toml
[client.terminal]
font_family = "JetBrains Mono"
font_size = 13
color_scheme = "Dracula"
```

`font_size` accepts values from 6 to 72. `color_scheme` is the name of a built-in iTerm2 scheme. Leave it empty for the default palette.

## Configure `[client.updates]`

```toml
[client.updates]
auto_check = true
channel = "stable"
```

`channel` is `stable` or `nightly`. When unset, it follows the build you installed. Automatic checks run 5 seconds after launch and every 5 hours after that. Condr only tells you. It never installs anything.

## Save remote Devices in `[[client.servers]]`

```toml
[[client.servers]]
name = "build-box"
address = "ssh://rocky@build-box"

[[client.servers]]
name = "home"
address = "tcp://<device key>@192.168.1.20:2637"
```

These are the saved remote Devices. `name` is the sidebar name and the argument to `--device`. `address` is an `ssh://`, `tcp://`, or `p2p://` address without the invite. The window writes this list when you connect, edit, or delete a Device. The command line only reads it.

## Add editors with `[[client.editors]]`

```toml
[[client.editors]]
name = "Helix"
command = ["hx"]
```

This adds an editor to “Open in”. Condr appends the path to open as the last argument. Restart the window after a hand edit. Condr stores its per-repository choice in `[[client.workspace_editors]]`. Do not edit that table by hand.

## Set environment variables

| Variable | What it does |
| --- | --- |
| `CONDR_LOG` | The log filter in `tracing` EnvFilter syntax, such as `debug` or `condr_server=trace`. Default: `warn,condr_core=info,condr_server=info,condr_gui=info` |
| `CONDR_LOG_DIR` | The log folder |
| `CONDR_CONFIG_DIR` | The config folder |
| `CONDR_SOCKET_PATH` | The socket path of the local Server |
| `CONDR_DEVICE` | The default for `--device` |
| `CONDR_VERSION` | Which version the install script installs: `nightly` or `v0.1.0` |
| `CONDR_INSTALL_DIR` | Where the install script and `server install` put the command |
| `CONDR_INSTALL_ARGS` | Arguments the Windows install script passes to `server install` |

The Server inherits its environment from the process that starts it. Therefore `CONDR_LOG=debug condr server restart` works, but a change to `.zshrc` needs a Server restart before it appears.

See [Agent automation](/docs/using/automation/) for variables set automatically inside a Pane.

## Find other Server files

| File | Location | What it is | If you delete it |
| --- | --- | --- | --- |
| `device-key` | Config folder | This Device's key | A new key is created, the authorized list is cleared, and every pairing must be redone |
| `authorized-clients` | Config folder | The paired Devices, one per line | Every paired Device is refused on its next connection |
| `pending-invite` | Config folder | The invite that is currently valid | That invite stops working |
| `condr-server-<id>.snapshot` | State folder | The Session structure | Deleted while the Server is stopped, the next start is empty. Worktrees stay on disk |
| `condr-gui.state` | State folder | Window position, sidebars, and the Tab each Workspace showed | The window opens at its default size |
| `condr-server-<id>.<date>.log` | Log folder | Server logs, rolled daily and kept for 7 days | No effect |
| `condr-gui.<date>.log` | Log folder | Window logs | No effect |

The key and invite files must have mode 0600. Condr refuses to use them when other users can read them.
