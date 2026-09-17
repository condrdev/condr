---
title: Getting started
description: Install Condr and start your first workspace.
---

## Install

:::caution
Condr is in early development. A new [Nightly](https://github.com/condrdev/condr/releases/tag/nightly) build is published every day `main` changes. Client and Server must use the same build.
:::

### Desktop

Download the installer for your platform from [Releases](https://github.com/condrdev/condr/releases).

| Platform             | Package  |
| -------------------- | -------- |
| Linux x86_64 / arm64 | AppImage |
| macOS x86_64 / arm64 | `.dmg`   |
| Windows x86_64       | `.exe`   |

### Headless server

The install script downloads the headless build (CLI/Server only) for the current machine, verifies it against `SHA256SUMS`, and runs `condr server install`.

| Platform             | Install command                                 |
| -------------------- | ----------------------------------------------- |
| Linux x86_64 / arm64 | `curl -fsSL https://condr.dev/install.sh \| sh` |
| macOS x86_64 / arm64 | `curl -fsSL https://condr.dev/install.sh \| sh` |
| Windows x86_64       | `irm https://condr.dev/install.ps1 \| iex`     |

Options:

```sh
CONDR_VERSION=v0.1.0 curl -fsSL https://condr.dev/install.sh | sh   # a versioned release (default: nightly)
curl -fsSL https://condr.dev/install.sh | sh -s -- --start           # also start the Server
```

```powershell
$env:CONDR_VERSION = 'v0.1.0'; irm https://condr.dev/install.ps1 | iex
$env:CONDR_INSTALL_ARGS = '--start'; irm https://condr.dev/install.ps1 | iex
```

The binary goes to `~/.local/opt/condr` (`%LOCALAPPDATA%\Programs\Condr` on Windows) and is added to your PATH. Override with `CONDR_INSTALL_DIR`. Open a new terminal, then run `condr --help`.

## Start a workspace

Run the server, connect a client, and open a session for the agent CLI you want to use.

The server owns sessions and terminals. Closing the GUI does not stop them.
