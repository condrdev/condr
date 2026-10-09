---
title: Configuration and settings
description: The full guide to configuring Condr, covering Settings and config.toml, every configuration key, environment variables and storage paths on each platform.
---

Condr's configuration is layered:
* **Client settings**: control the front end's appearance and interaction, and apply to the current machine only.
* **Server settings**: control terminal instances, worktrees and remote connection policy, and apply to the host Device that runs the Server.
* **Machine-wide settings**: apply to the whole machine and are read by every Condr process on it, such as the proxy.

All settings are stored in `config.toml`, which you can modify in the graphical interface or edit directly.

---

## Settings window

* **Shortcut**: `Cmd + ,` on macOS, `Ctrl + ,` on Windows and Linux.
* **Auto-save**: changes are written to `config.toml` immediately. A text field writes when it loses focus, when you press Enter or when you click **Save**.

The Settings window configures one Device at a time. Use the picker at the top to switch Devices (defaults to the local machine), which also displays connection type and status. The local machine shows all pages; remote Devices only show the pages marked **Device** below; disconnected Devices show no pages until connected.

| Page | Applies to | Description |
| :--- | :--- | :--- |
| **General** | Device | **Status**: version, uptime, session metrics (Workspace and Pane counts) and recent errors. **Terminal**: default shell for new Panes (defaults to the system environment if left blank) |
| **Appearance** | This machine | Theme (follow system, light or dark), terminal fonts and colors, syntax highlighting and font sizes for Preview and Diff |
| **Notifications** | This machine | System notification toggle and test notification button |
| **Power** | This machine | Keep display awake (mirrors the coffee cup icon in the sidebar footer) |
| **Shortcuts** | This machine | Keyboard shortcuts reference (read-only) |
| **Network** | Device | Proxy settings for Peer-to-peer connections and update checks: **System**, **None** or **Manual** with a custom URL |
| **Remote access** | Device | TCP listener toggle and address, Peer-to-peer toggle, and the **Restart Condr** button. Changes take effect after a restart; until then the button reads **Restart to apply** |
| **Paired devices** | Device | Click **Generate invite** to create a one-time pairing link (both P2P and TCP formats). Lists paired Devices with online ones first, with a **Revoke** button on each row to remove pairings |
| **Agent integrations** | Device | View hook status for each Agent, with one-click options to install, update or remove them |
| **Developer** | This machine | Frame rate display toggle and quick-access buttons for application, config, data, state and log directories |
| **Licenses** | This machine | Open-source dependencies and license details |
| **About** | This machine | Version details, update channel selection, manual update check and automatic check toggle |

> **Access restriction**: when connected over TCP or P2P, settings that affect how other Devices connect to that Device (the proxy under `Network` and all options under `Remote access`, including **Restart Condr**) are disabled to prevent accidental disconnection. Hover over any disabled option to view details. Change them on the Device itself or over SSH. All other settings remain editable, including **Generate invite** and **Revoke** (for any Device except the one currently in use).

---

## The `config.toml` file

The GUI client, the background Server and the `condr` CLI share the `config.toml` in the config directory (see [Storage directories and files](#storage-directories-and-files)). Every field is optional and falls back to its built-in default when absent.

* **Writing**: Condr takes a file lock when writing the config to avoid concurrent writes, and keeps your comments and layout intact.
* **When changes apply**:
  * Changes made in the GUI apply immediately, except for network options on the `Remote access` page and the Peer-to-peer proxy.
  * After editing `[client]` or its sub-tables manually, restart the GUI window.
  * After editing `[server]` or its sub-tables manually, run `condr server restart` in a terminal.
  * After editing `[network]` manually, perform both steps.
* **Error fallback**: if the file has a syntax or parse error, Condr reports it in the window and runs on the full set of defaults. Until the error is fixed, changes made in the UI cannot be saved.

---

## Client keys

### `[client]` basic preferences

```toml
[client]
appearance = "dark"
notifications = true
keep_awake = false
fps_monitor = false
```

| Key | Type / values | Default | Description |
| --- | --- | --- | --- |
| `appearance` | `system` / `light` / `dark` | `system` | Interface color mode. An invalid value falls back to `system` |
| `notifications` | bool | `true` | System notifications when an Agent finishes a task or needs your confirmation |
| `keep_awake` | bool | `false` | Keep the screen on, and stop the system from sleeping when idle |
| `fps_monitor` | bool | `false` | Overlay the live frame rate and resource usage on the interface |
| `editor` | string | Empty | The editor last used through **Open in**, also the global default (written and maintained by Condr) |

### `[client.terminal]` terminal display

```toml
[client.terminal]
font_family = "JetBrains Mono"
font_size = 13
color_scheme = "Dracula"
```

| Key | Type / values | Default | Description |
| --- | --- | --- | --- |
| `font_family` | string | The interface theme's monospace font | The font terminals render with. Empty uses the interface's built-in default font |
| `font_size` | 6 ~ 72 | The interface's base monospace size | In pixels. Fractions are rounded, and out-of-range values are clamped to the nearest bound |
| `color_scheme` | string | Empty | A built-in iTerm2 scheme name, matching the **Appearance** dropdown exactly. Empty or unrecognized names use the default palette |

### `[client.code]` code views (Preview / Diff)

```toml
[client.code]
theme = "Dracula"
font_size = 14
```

| Key | Type / values | Default | Description |
| --- | --- | --- | --- |
| `theme` | string | Empty | The syntax highlight theme, matching an **Appearance › Syntax** option. When empty or unrecognized, a light interface uses GitHub Light and a dark one uses GitHub Dark |
| `font_size` | 6 ~ 72 | The interface's base monospace size | The font size of code views, with the same bounds and rounding as the terminal |

### `[client.updates]` software updates

```toml
[client.updates]
auto_check = true
channel = "stable"
```

| Key | Type / values | Default | Description |
| --- | --- | --- | --- |
| `auto_check` | bool | `true` | Check for updates automatically (first 5 seconds after the client starts, then every 5 hours). Condr only notifies you and never installs silently |
| `channel` | `stable` / `nightly` | The installed build's channel | Which release stream to follow. See [release channels](/docs/start/install/#release-channels) for how they differ |

### `[[client.servers]]` remote Device list

```toml
[[client.servers]]
name = "build-box"
address = "ssh://rocky@build-box"

[[client.servers]]
name = "home"
address = "tcp://<device key>@192.168.1.20:2637"
```

An array of tables manages the remote Devices:

* `name`: the Device's name in the sidebar, which `condr --device <name>` also uses to route a command.
* `address`: the connection URI, starting with `ssh://`, `tcp://` or `p2p://` (without the one-time invite secret). See [Remote connections](/docs/using/remote/) for each protocol's format.

> The GUI maintains and persists this list; the CLI only reads it. When one entry's URI fails to parse, Condr skips it and reports an error, and the other Devices connect as usual.

### `[[client.editors]]` external editors

```toml
[[client.editors]]
name = "Helix"
command = ["hx"]
```

Registers an external program in the **Open in** submenu of the context menu:

* `name`: the label shown in the menu.
* `command`: the program and its leading arguments. When Condr launches it, the target file or directory path is passed as the last argument.

Restart the client after editing this table by hand. Condr records each project's choice in `[[client.workspace_editors]]` (runtime storage; editing it by hand is not recommended). For more examples, see [Shortcuts and preferences](/docs/using/preferences/#add-an-external-editor-open-in).

---

## Server keys

The Server loads its configuration only on a cold start. After editing it by hand, run `condr server restart` to reload it.

### `[server]` network listening

```toml
[server]
listen = "0.0.0.0:2637"
```

| Key | Type / format | Default | Description |
| --- | --- | --- | --- |
| `listen` | `IP:PORT` | Empty | An extra TCP listen address (a plain IP address; host names are not resolved). When unset, the Server does not listen on TCP and accepts only local socket connections. `condr server start --listen` writes this key back |

### `[server]` worktree location

```toml
[server]
worktree_root = "~/worktrees"
```

| Key | Type / format | Default | Description |
| --- | --- | --- | --- |
| `worktree_root` | path | Empty | The root directory that holds every Git worktree |

How `worktree_root` is written decides where worktrees go on disk:

| Mode | Rule | Layout |
| --- | --- | --- |
| **Unset** | Next to each repository | `<repo>.worktrees/<branch>` |
| **Absolute path / starts with `~`** | Gathered under the given root | `<root>/<repo>/<branch>` |
| **Relative path** | Inside each repository's root | `<repo>/<root>/<branch>` |

*Note: characters in the branch name other than letters, digits, `.`, `_` and `-` become `-`. With the example above, branch `feat/login` of repository `my-app` maps to `~/worktrees/my-app/feat-login`.*

### `[server.p2p]` peer-to-peer connections

```toml
[server.p2p]
enabled = true
```

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `enabled` | bool | `false` | Enable P2P connections through NATs. `condr server start --p2p` sets it to `true` |

### `[server.terminal]` terminal environment

```toml
[server.terminal]
shell = "/opt/homebrew/bin/fish"
```

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `shell` | path | Empty | The executable new Panes start. When empty, Unix-like systems read `$SHELL`, and Windows tries `pwsh.exe`, `powershell.exe` and `%ComSpec%` in that order |

A shell changed under **Settings › General** applies right away: the next new Pane uses it, with no Server restart.

---

## Machine-wide keys

Tables without a `client` or `server` prefix apply to all Condr processes on the machine. In the GUI, they are managed through that machine's Server just like Server settings.

### `[network.proxy]` proxy

```toml
[network.proxy]
mode = "manual"
url = "http://user:password@proxy:8080"
```

| Key | Type / values | Default | Description |
| --- | --- | --- | --- |
| `mode` | `system` / `none` / `manual` | `system` | `system` checks `HTTPS_PROXY` (or `ALL_PROXY`) and `NO_PROXY`, then falls back to Windows or macOS system proxy settings. `none` disables proxies entirely. `manual` uses the `url` value |
| `url` | `http://` or `https://` URL | Empty | The proxy URL used in `manual` mode, including `user:password@` if authentication is required. Retained when switching to other modes. An empty `url` in `manual` mode disables proxying |

Proxy settings apply to the Server's connection to the Condr relay for Peer-to-peer networking and to update checks in the window. Peer-to-peer applies changes when the Server restarts; update checks apply them immediately.

---

## Environment variables

| Variable | Description |
| --- | --- |
| `CONDR_LOG` | Log filter in `tracing` EnvFilter syntax (such as `debug` or `condr_server=trace`). Default: `warn,condr_core=info,condr_server=info,condr_gui=info` |
| `CONDR_LOG_DIR` | A custom log directory |
| `CONDR_CONFIG_DIR` | A custom config directory |
| `CONDR_DATA_DIR` | A custom data directory |
| `CONDR_SOCKET_PATH` | A custom IPC path for the local Server (a Unix domain socket or a Windows named pipe) |
| `CONDR_DEVICE` | The default target for `--device` |
| `CONDR_VERSION` | The version the install script fetches (such as `nightly` or `v0.1.0`) |
| `CONDR_INSTALL_DIR` | Where the install script and `condr server install` put the binary |
| `CONDR_INSTALL_ARGS` | Extra arguments the Windows install script passes to `condr server install` (such as `--force`) |

> **Inherited environment**: the Server inherits the full environment of the process that starts it, and never reads shell profiles itself. For example, `CONDR_LOG=debug condr server restart` in a terminal restarts the Server with debug logging. After changing `.zshrc` or `.bashrc`, run `condr server restart` in a terminal that has loaded the new profile; **Restart Condr** in Settings keeps the old Server's environment.

For the variables set in every Pane, see [Agent automation](/docs/using/automation/).

---

## Storage directories and files

### Directories by platform

| Category | Linux | macOS | Windows |
| --- | --- | --- | --- |
| **Config** (`config.toml`) | `~/.config/condr` | `~/Library/Application Support/condr` | `%APPDATA%\condr` |
| **Data** (Device key pair, paired list) | `~/.local/share/condr` | `~/Library/Application Support/condr` | `%LOCALAPPDATA%\condr` |
| **State** (Session snapshot) | `~/.local/state/condr` | `~/Library/Application Support/condr` | `%LOCALAPPDATA%\condr` |
| **Logs** | `~/.local/state/condr` | `~/Library/Logs/condr` | `%LOCALAPPDATA%\condr` |
| **Runtime** (socket / temporary files) | `$XDG_RUNTIME_DIR/condr` | `$TMPDIR/condr` | `%LOCALAPPDATA%\condr\runtime` |
| **Binary** (the `condr` program) | `~/.local/opt/condr` | `~/.local/opt/condr` | `%LOCALAPPDATA%\Programs\Condr` |

* **XDG**: on Linux, `XDG_CONFIG_HOME`, `XDG_DATA_HOME` and `XDG_STATE_HOME` are honored. Without `XDG_RUNTIME_DIR`, runtime files fall back to `~/.local/share/condr/runtime`.
* **Quick access**: **Settings › Developer › Locations** has a button that opens each directory.

### Core files

| File | Category | Purpose | If removed |
| --- | --- | --- | --- |
| `config.toml` | Config | Every window and Server setting | All settings return to their defaults, and the saved remote Device list is emptied |
| `device-key` | Data | This Device's asymmetric key | The next start generates a new key pair and clears the paired list; existing peers must pair again |
| `authorized-clients` | Data | The pairing allowlist (one Device per line) | Paired Devices are refused on their next connection and must pair again |
| `pending-invite` | Data | The one-time pairing invite currently in effect | That invite link stops working at once |
| `condr-server-<id>.snapshot` | State | The Session structure snapshot (Workspace, Tab and Pane layout) | *Stop the Server first*. The next cold start discards every session context (worktrees checked out on disk are untouched) |
| `condr-gui.state` | State | Window geometry, sidebar collapse state and the Tab each Workspace has open | The window resets to its default centered size and layout |
| `condr-server-<id>.<date>.log` | Logs | The Server's running log (rotated daily, the latest 7 kept) | No direct effect |
| `condr-gui.<date>.log` | Logs | The front end's log (rotated the same way) | No direct effect |
| `condr-server-<id>.stderr` | Logs | Output from a background Server crash or an uncaught panic, with its stack | No direct effect |

> **Security and permissions**: the key and the invite are sensitive authentication files. On Linux and macOS their permissions must be `0600` (owner read and write); any group or other read permission makes Condr refuse to load them. On Windows, Condr sets their ACL itself and restricts them to the current user.

:::note[Directory migration]
Releases up to 0.1.6 stored `device-key`, `authorized-clients` and `pending-invite` in the config directory. The first start of a newer release moves them to the data directory automatically, and the key and every pairing carry over intact.
:::

---

## Further reading

* [Shortcuts and preferences](/docs/using/preferences/): key bindings, external editors and the default terminal shell
* [Remote connections](/docs/using/remote/): end-to-end secure setup of direct TCP and P2P connections
* [CLI reference](/docs/reference/cli/): command syntax and managing the Server's background lifecycle
* [Troubleshooting](/docs/help/troubleshooting/): common disconnects, permission denials and reading crash logs
