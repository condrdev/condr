---
title: Install
description: Install the Condr desktop app or the Headless Server, and learn how to start it, manage versions, upgrade smoothly and uninstall.
---

Condr ships in two deployment forms:

| Form | Best for | Components |
| :--- | :--- | :--- |
| **Desktop app** (recommended) | Everyday local development and interaction | GUI + bundled Server |
| **Headless Server** | Remote servers, cloud hosts, GUI-less containers and dev machines | `condr` CLI and Server only |

:::caution
Condr 0.1 is currently in Public Preview. Features and configuration options may keep changing in later releases.
:::

---

## Desktop app

The desktop app bundles the Server and starts it automatically when the app opens, so there is no background service to configure separately. To connect to this device remotely, see [Remote connections](/docs/using/remote/).

Get the package for your platform from the [download center](/download/):

| Platform | Architecture | Package |
| :--- | :--- | :--- |
| macOS | x86_64 / Apple Silicon | `.dmg` |
| Windows | x86_64 | `.exe` |
| Linux | x86_64 / arm64 | `.AppImage` |

### Platform notes

**Windows**
The installer is not code-signed yet. If SmartScreen blocks it, click **More info → Run anyway**.

**Linux**
After downloading, make the AppImage executable and launch it:

```sh
chmod +x condr-*-linux-*.AppImage
./condr-*-linux-*.AppImage
```

> **Tip**: if your environment lacks FUSE support, add this flag to extract and run it directly:
> ```sh
> ./condr-*-linux-*.AppImage --appimage-extract-and-run
> ```

---

## Headless Server

In environments without a graphical interface, only the `condr` CLI needs to be installed.

### 1. One-line install

**Linux / macOS**:
```sh
curl -fsSL https://condr.dev/install.sh | sh
```

**Windows (PowerShell)**:
```powershell
irm https://condr.dev/install.ps1 | iex
```

The install script places the binary in the directory below and adds it to your `PATH` automatically:
- **Linux / macOS**: `~/.local/opt/condr`
- **Windows**: `%LOCALAPPDATA%\Programs\Condr`

Reopen your terminal and verify the installation:
```sh
condr --version
```

### 2. Start the service

```sh
condr server start
```

The Server runs as a daemon; closing the current terminal does not affect it.

*Note: no system startup service is registered yet, so after a host reboot you need to run this command again by hand.*

### 3. Advanced install and environment options

Environment variables let you switch release channels, pin a version or force a reinstall:

**Linux / macOS**:
```sh
# Switch to the daily Nightly build
CONDR_VERSION=nightly curl -fsSL https://condr.dev/install.sh | sh

# Install a specific version
CONDR_VERSION=v0.1.0 curl -fsSL https://condr.dev/install.sh | sh

# Force-reinstall the current version
curl -fsSL https://condr.dev/install.sh | sh -s -- --force
```

**Windows (PowerShell)**:
```powershell
# Switch to the daily Nightly build
$env:CONDR_VERSION = 'nightly'; irm https://condr.dev/install.ps1 | iex

# Install a specific version
$env:CONDR_VERSION = 'v0.1.0'; irm https://condr.dev/install.ps1 | iex

# Force-reinstall the current version
$env:CONDR_INSTALL_ARGS = '--force'; irm https://condr.dev/install.ps1 | iex
```

---

## Release channels

By default, Condr checks its channel for updates in the background every 5 hours. You can see the version status in the client under **Settings › About › Updates**.

| Channel | What it is | Who it is for |
| :--- | :--- | :--- |
| **Stable** | Tested, standard stable releases (default) | Most production and everyday use |
| **Nightly** | Built automatically from the `main` branch every day, with the latest features | Early adopters, feature testing and bug verification |

---

## Upgrading

> **Compatibility note**: the client and Server versions need to match. If the two sides differ in version but the protocol is compatible, the sidebar shows a yellow warning icon; if the protocol is incompatible, the connection is refused.

### Upgrading the desktop app

Download the new version and install it over the old one. The old Server is handled as follows:

- **Windows**: the installer stops the old Server automatically, and the new Server is started when you launch the new client.
- **macOS / Linux**: the old Server keeps running after installation. The new client prompts you to restart the service when it launches; you can also do it later by clicking **Restart Condr** under **Settings › Device › General**.

### Upgrading the Headless Server

Run the matching install script again; it detects the current install and updates it in place.

If it finds a Server running, the script asks whether to restart it now:
- **Restart now**: the new version takes effect immediately.
- **Restart later**: the current process keeps running, and the switch completes once you restart it by hand.

> **About the impact of a restart**: restarting the Server interrupts every running Pane process. Afterwards, Condr restores the Workspaces, Tabs and Pane layout from the snapshot. Agents with Hooks configured resume their sessions automatically; see [Workspaces, Tabs and Panes](/docs/using/workspaces/).

---

## Uninstall

Uninstalling removes only the application itself and its executables. **It does not delete your configuration, runtime state or local logs.** To remove the remaining data completely, see [Configuration and settings](/docs/reference/configuration/#where-the-files-are) and clean up the directories by hand.

### Step 1: remove the main application

| Install type | How to uninstall |
| :--- | :--- |
| **Windows desktop app** | Uninstall it under **Settings › Apps › Installed apps**. The installer stops the background service and cleans up the environment variables automatically. |
| **macOS desktop app** | Move `Condr.app` to the Trash, then run the cleanup command below. |
| **Linux desktop app** | Delete the `.AppImage` file, then run the cleanup command below. |
| **Headless Server** | Run the cleanup command directly. |

### Step 2: remove the CLI and background service

In a regular system terminal, **not a terminal inside Condr**, run:

```sh
condr server uninstall
```

After confirmation, this command stops the running Server process and deletes the `condr` binary together with its `PATH` entry.
*In scripted environments, add `--yes` to skip the interactive confirmation: `condr server uninstall --yes`*
