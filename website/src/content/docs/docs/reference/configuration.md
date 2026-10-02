---
title: Configuration and settings
description: Change window and Server settings, and find Condr's files on each platform.
---

:::caution[Under construction]
This page is still being written. Content will follow.
:::

Use this page to find a setting, learn when it takes effect and locate the files Condr saves.

## Change settings in the interface

Press Cmd+, on macOS or Ctrl+, on other platforms to open settings. The window has two tabs. **Application** holds the window's own settings, and **Device** holds the Server settings of one Device. A bar at the top of the Device tab's pages shows which Device you are editing, how it is connected and its connection status; switch Devices there.

Every change takes effect immediately and is written to the configuration file. A text field saves when you leave it, press Enter or click Save.

**Application**

| Page | Settings |
| --- | --- |
| Appearance | Theme (follow system, light, dark), terminal font, font size and color scheme, highlight theme and font size for Preview and Diff |
| Notifications | Turn system notifications on or off, send a test notification |
| Power | Keep the screen awake. The coffee cup at the bottom of the sidebar uses the same switch |
| Shortcuts | The list of shortcuts, read-only |
| Developer | Frame rate monitor, buttons that open the app, config, state and log directories |
| Licenses | Third-party components Condr uses and their licenses |
| About | Version, update channel, automatic update checks, check now; when a new version is available, the Updates group shows its version number and a **View** button |

**Device**

| Page | Settings |
| --- | --- |
| General | The Status group shows the connection method, version, uptime, counts, the most recent error, the listen address the Server actually bound and the Peer-to-peer status, and has a **Restart Condr** button. The Terminal group sets the shell new Panes use; leave it empty to use the system default |
| Remote access | TCP listener switch and Listen address, Peer-to-peer switch. Changes take effect only after the Server restarts, and the button on the General page becomes **Restart to apply** |
| Paired devices | **Generate invite** creates a one-time invite and lists one link each for Peer-to-peer and TCP to copy. The list of paired Devices puts connected ones first with a green dot, and each row has **Revoke** |
| Agent integrations | Each Agent's hook status, plus install, update and uninstall |

The Remote access and Paired devices pages can be changed only over a local or SSH connection. Over a TCP or Peer-to-peer connection, the top of the Device tab shows "Viewing only" and these controls are unavailable.

## Where the files are

| Contents | Linux | macOS | Windows |
| --- | --- | --- | --- |
| Config: `config.toml`, keys | `~/.config/condr` | `~/Library/Application Support/condr` | `%APPDATA%\condr` |
| State: snapshot, window state | `~/.local/state/condr` | `~/Library/Application Support/condr` | `%LOCALAPPDATA%\condr` |
| Logs | `~/.local/state/condr` | `~/Library/Logs/condr` | `%LOCALAPPDATA%\condr` |
| Runtime: socket, temporary files | `$XDG_RUNTIME_DIR/condr` | `$TMPDIR/condr` | `%LOCALAPPDATA%\condr\runtime` |
| Installed `condr` command | `~/.local/opt/condr` | `~/.local/opt/condr` | `%LOCALAPPDATA%\Programs\Condr` |

On Linux, `XDG_CONFIG_HOME` and `XDG_STATE_HOME` apply as usual. **Settings › Developer › Locations** has buttons that open these directories.

The window and the command line share `config.toml`, and you can edit it by hand. Condr keeps comments when it writes and uses a lock so two processes never write at the same time. If the file is malformed, every key falls back to its default.

After editing `[client]` keys by hand, restart the window; after editing `[server]` keys by hand, restart the Server. Changes made in the interface take effect immediately.

## Configure `[server]`

```toml
[server]
listen = "0.0.0.0:2637"
worktree_root = "~/worktrees"
```

| Key | Meaning | Takes effect |
| --- | --- | --- |
| `listen` | An extra TCP address to listen on, given only as an IP and port. When unset, there is no TCP listener. `condr server start --listen` writes it. | After a Server restart |
| `worktree_root` | Where the worktrees Condr creates are stored. When unset, they go in `<repo>.worktrees/` next to the repository. Absolute path or `~`: `<root>/<repo>/<branch>`. Relative path: `<repo>/<root>/<branch>`. | After a Server restart |

## Configure `[server.p2p]`

```toml
[server.p2p]
enabled = true
```

`enabled` lets the Server accept Peer-to-peer connections and is off by default. `condr server start --p2p` writes it, and it takes effect after the Server restarts.

## Configure `[server.terminal]`

```toml
[server.terminal]
shell = "/opt/homebrew/bin/fish"
```

`shell` is the program a new Pane starts. When it is empty, Unix uses `$SHELL`, and Windows looks for `pwsh.exe`, `powershell.exe` and `%ComSpec%` in that order. A change made in the interface applies from the next new Pane. A hand edit requires a Server restart.

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
| `appearance` | `system`, `light` or `dark` | `system` |
| `notifications` | Send a system notification when an Agent finishes or needs you | `true` |
| `keep_awake` | Keep the screen from sleeping | `false` |
| `fps_monitor` | Show the frame rate | `false` |
| `editor` | The last "Open in" target used | None |

## Configure `[client.terminal]`

```toml
[client.terminal]
font_family = "JetBrains Mono"
font_size = 13
color_scheme = "Dracula"
```

`font_size` ranges from 6 to 72. `color_scheme` is the name of a built-in iTerm2 color scheme. Leave it empty to use the default palette.

## Configure `[client.updates]`

```toml
[client.updates]
auto_check = true
channel = "stable"
```

`channel` is `stable` or `nightly`. When unset, it follows the build you installed. The automatic check runs once 5 seconds after launch and then every 5 hours. Condr only tells you about an update; it never installs one automatically.

## Save remote Devices in `[[client.servers]]`

```toml
[[client.servers]]
name = "build-box"
address = "ssh://rocky@build-box"

[[client.servers]]
name = "home"
address = "tcp://<device key>@192.168.1.20:2637"
```

This list saves remote Devices. `name` is the name shown in the sidebar and also the argument to `--device`. `address` is an `ssh://`, `tcp://` or `p2p://` address without the invite. The interface writes the list when you connect, edit or remove a Device; the command line only reads it.

## Add editors in `[[client.editors]]`

```toml
[[client.editors]]
name = "Helix"
command = ["hx"]
```

This adds the editor to "Open in". The path to open is appended as the last argument. Restart the window after editing by hand. The choice Condr remembers for each repository is written to `[[client.workspace_editors]]`; do not edit it by hand.

## Set environment variables

| Variable | Purpose |
| --- | --- |
| `CONDR_LOG` | Log filter in `tracing` EnvFilter syntax, for example `debug` or `condr_server=trace`. Default: `warn,condr_core=info,condr_server=info,condr_gui=info` |
| `CONDR_LOG_DIR` | Log directory |
| `CONDR_CONFIG_DIR` | Config directory |
| `CONDR_SOCKET_PATH` | Socket path of the local Server |
| `CONDR_DEVICE` | Default value of `--device` |
| `CONDR_VERSION` | Version the install script installs: `nightly` or `v0.1.0` |
| `CONDR_INSTALL_DIR` | Where the install script and `server install` put the command |
| `CONDR_INSTALL_ARGS` | Arguments the Windows install script passes to `server install` |

The Server inherits the environment of the process that starts it. So `CONDR_LOG=debug condr server restart` works, but after changing `.zshrc` you must restart the Server for the change to take effect.

For the variables set automatically in a Pane, see [Agent automation](/docs/using/automation/).

## Check the Server's other files

| File | Location | Purpose | If deleted |
| --- | --- | --- | --- |
| `device-key` | Config directory | This Device's key | A new key is generated, the authorized list is cleared, and every pairing must be redone |
| `authorized-clients` | Config directory | Paired Devices, one per line | Every paired Device is refused on its next connection |
| `pending-invite` | Config directory | The currently valid invite | That invite stops working |
| `condr-server-<id>.snapshot` | State directory | Session structure | Delete it while the Server is stopped, and the next start is empty. Worktrees stay on disk |
| `condr-gui.state` | State directory | Window position, sidebar and the Tab shown in each Workspace | The window opens at the default size |
| `condr-server-<id>.<date>.log` | Log directory | Server log, rolled daily and kept for 7 days | No effect |
| `condr-gui.<date>.log` | Log directory | Window log | No effect |

Key and invite files must have permissions 0600. If other users can read them, Condr refuses to use them.
