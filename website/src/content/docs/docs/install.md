---
title: Install
description: Install the desktop app or headless Server, then upgrade or uninstall Condr.
---

Choose the package for each machine, install it, and keep Condr updated or remove it later.

:::caution
Condr 0.1 is a public preview, and things may still change between versions. Update the window and the Server together.
:::

## Choose what to install

| This machine | Install | Includes |
| --- | --- | --- |
| The computer you work at | Desktop app | The window and the Server |
| A machine that only runs agents, with no window | Headless Server | Only the `condr` command |

The desktop app includes the Server, so local use needs no headless Server. To reach this computer from another one, treat it as a remote Device. See [Remote devices](/docs/remote/).

## Install the desktop app

Download the installer for your platform from the [download page](/download/):

| Platform | Package |
| --- | --- |
| Linux x86_64 / arm64 | AppImage |
| macOS x86_64 / arm64 | `.dmg` |
| Windows x86_64 | `.exe` |

The Windows installer goes to `%LOCALAPPDATA%\Programs\Condr`, needs no administrator rights, and requires Windows 10 1809 or later. On first launch, the macOS app and the Linux AppImage copy the `condr` command to `~/.local/opt/condr`. The window uses that copy to start the Server.

### macOS says the app can't be verified

The app isn't notarized by Apple yet, so the first launch says it can't be verified. Open it in either of these ways:

- Click **Done**, open **System Settings › Privacy & Security**, scroll down, and click **Open Anyway**.
- Or clear the download flag once in a terminal:

  ```sh
  xattr -cr /Applications/Condr.app
  ```

### Windows shows SmartScreen

The preview installer isn't signed. When SmartScreen blocks it, click **More info**, then **Run anyway**.

### The Linux AppImage won't open

AppImage needs FUSE. Install your distribution's `libfuse2` package, or run it after unpacking with `--appimage-extract`.

## Install the headless Server

On the machine that will run agents, run the command for its platform:

| Platform | Install command |
| --- | --- |
| Linux x86_64 / arm64 | `curl -fsSL https://condr.dev/install.sh \| sh` |
| macOS x86_64 / arm64 | `curl -fsSL https://condr.dev/install.sh \| sh` |
| Windows x86_64 | `irm https://condr.dev/install.ps1 \| iex` |

The script downloads the headless build, checks it against `SHA256SUMS`, runs `condr server install`, and adds `condr` to your PATH. It installs the command at `~/.local/opt/condr`, or at `%LOCALAPPDATA%\Programs\Condr` on Windows.

The script adds PATH in `~/.zprofile` on macOS or `~/.profile` on Linux. On Windows, it updates the user's environment variables. To verify the install, open a new terminal and run:

```sh
condr --version
```

Non-interactive SSH shells usually don't read those files. If another machine reports `condr: not found` over SSH, name the path in the link. See [Remote devices](/docs/remote/#connect-over-ssh).

### Choose install script options

Running the script again does nothing when the latest version is already installed. Use these commands to install a version, start the Server after installation, or reinstall:

```sh
CONDR_VERSION=nightly curl -fsSL https://condr.dev/install.sh | sh   # the nightly (default: the latest release)
CONDR_VERSION=v0.1.0 curl -fsSL https://condr.dev/install.sh | sh    # one versioned release
curl -fsSL https://condr.dev/install.sh | sh -s -- --start            # also start the Server
curl -fsSL https://condr.dev/install.sh | sh -s -- --force            # reinstall even when up to date
```

```powershell
$env:CONDR_VERSION = 'nightly'; irm https://condr.dev/install.ps1 | iex
$env:CONDR_VERSION = 'v0.1.0'; irm https://condr.dev/install.ps1 | iex
$env:CONDR_INSTALL_ARGS = '--start'; irm https://condr.dev/install.ps1 | iex
$env:CONDR_INSTALL_ARGS = '--force'; irm https://condr.dev/install.ps1 | iex
```

To install somewhere else, set `CONDR_INSTALL_DIR`.

## Choose Release or Nightly

| Channel | What it is | Who it's for |
| --- | --- | --- |
| Release | A tagged, versioned release | Most people |
| Nightly | Built from `main` once a day | People who want the latest changes and accept occasional breakage |

The window checks the channel you installed 5 seconds after launch, then every 5 hours. When a newer build is available, **Settings › About** shows a notice and the settings icon in the sidebar gets a dot. Condr only tells you. It never installs anything. Change the channel in **Settings › About**.

## Upgrade both parts

Update the window and the Server together. A compatible protocol lets different versions connect and shows a yellow triangle in the sidebar. An incompatible protocol refuses the connection.

- **Desktop app**: download the new installer and install over the old one. After the window restarts, the local Server is still the old version. Run `condr server restart`, or click **Restart Condr** in **Settings › Device › Daemon**, so the new version takes over.
- **Headless Server**: run the install script again. If the Server is running, the script asks whether to restart it. If you do not restart it, connected windows show a version mismatch until you do.

Restarting the Server ends every program in every Pane, then restores the structure from the snapshot. Agents with hooks installed resume their sessions on their own. See [Workspaces, Tabs, and Panes](/docs/workspaces/#what-survives-a-server-restart).

## Uninstall Condr

Headless Server:

```sh
condr server uninstall
```

This stops the Server and removes the installed `condr`, its symlink, and its PATH entry. It keeps the config, state, and log directories and tells you where they are. To remove those directories, see [Configuration and settings](/docs/configuration/#where-the-files-are).

Desktop app: on macOS, drag `Condr.app` to the Trash. On Windows, uninstall from **Apps & features**. On Linux, delete the AppImage. To remove the `condr` command copied to `~/.local/opt/condr`, run the `uninstall` command above.
