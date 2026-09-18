---
title: Getting started
description: Install Condr, open your first workspace, and connect a remote device.
---

Condr has two parts:

- **Server**: the `condr` command. It runs all your agents and keeps working in the background.
- **Client**: the `condr-gui` app. It's the window you see, and it can connect to more than one Server.

## Install

:::caution
Condr is in early development. A new [Nightly](https://github.com/condrdev/condr/releases/tag/nightly) build is published every day. The Client and the Server must be the same build.
:::

### Desktop App

Install it on the computer you work at. It includes the Server, so local use needs nothing else.

Download the installer for your platform from [Releases](https://github.com/condrdev/condr/releases):

| Platform             | Package  |
| -------------------- | -------- |
| Linux x86_64 / arm64 | AppImage |
| macOS x86_64 / arm64 | `.dmg`   |
| Windows x86_64       | `.exe`   |

### Headless Server

Install it on the device where you want to run agents remotely. It includes only the `condr` command, with no graphical interface.

On that device, run the command for its platform:

| Platform             | Install command                                 |
| -------------------- | ----------------------------------------------- |
| Linux x86_64 / arm64 | `curl -fsSL https://condr.dev/install.sh \| sh` |
| macOS x86_64 / arm64 | `curl -fsSL https://condr.dev/install.sh \| sh` |
| Windows x86_64       | `irm https://condr.dev/install.ps1 \| iex`      |

The script downloads the headless build, verifies it against `SHA256SUMS`, and runs `condr server install`. The binary goes to `~/.local/opt/condr` (`%LOCALAPPDATA%\Programs\Condr` on Windows) and is added to your PATH. To check, open a new terminal and run `condr --help`.

To install a specific version, or to start the Server right after installing:

```sh
CONDR_VERSION=v0.1.0 curl -fsSL https://condr.dev/install.sh | sh   # a versioned release (default: nightly)
curl -fsSL https://condr.dev/install.sh | sh -s -- --start           # also start the Server
```

```powershell
$env:CONDR_VERSION = 'v0.1.0'; irm https://condr.dev/install.ps1 | iex
$env:CONDR_INSTALL_ARGS = '--start'; irm https://condr.dev/install.ps1 | iex
```

To install somewhere else, set `CONDR_INSTALL_DIR`.

## Open Your First Workspace

Open Condr. It starts the local Server if none is running, and reconnects to the existing one on later launches.

Add a workspace, then open a pane and run the agent CLI you want. Closing the window doesn't stop the Server or your agents. Open Condr again to pick up where you left off.

## Connect to a Remote Device

In the sidebar, click **Connect Remote Device**, then enter the link to the remote device. The link is in SSH or TCP format.

### SSH

```text
ssh://user@build-box
```

You can connect as soon as `condr` is installed on the remote device. Condr uses your existing OpenSSH configuration and ssh-agent, so no other setup is needed.

### TCP

```text
tcp://<server key>.<invite>@<host>:<port>
```

TCP listening is off by default. The remote device generates the pairing link. On that device:

1. Start the Server and have it listen on the network. If the Server is already running, use `restart` instead of `start`. The listen address is saved to the configuration file, so you don't need to pass it again:

   ```sh
   condr server start --listen 0.0.0.0:2637
   ```

2. Run `condr server invite`. The command prints the pairing link.
3. Enter the pairing link in Condr within 10 minutes. If it expires, repeat step 2.

Both sides authenticate each other with static keys (`Noise_IKpsk2`), and the connection is encrypted.
