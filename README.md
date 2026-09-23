# Condr

<p align="center">
  <img src="assets/brand/condr.svg" alt="Condr" width="64" />
</p>

<p align="center">
  <b>Keep your agents running</b>
  <br />
  One window for all your agents. Local or remote, step away and pick up where you left off.
</p>

<p align="center">
  English · <a href="README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <a href="https://github.com/condrdev/condr/releases"><img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-666666?labelColor=333333" alt="Linux, macOS, and Windows" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-666666?labelColor=333333" alt="Apache 2.0 license" /></a>
</p>

<p align="center">
  <img src="assets/screenshots/hero.png" alt="Condr connected to local and remote Servers, with Claude Code and Codex running side by side in each Workspace" />
</p>

Condr is a small desktop app. It runs Claude Code, Codex, and other CLI agents in one window, along with any other terminal program.

Condr has two parts:

- **Server**: the `condr` command. It runs all your agents and keeps working in the background.
- **Client**: the `condr-gui` app. It's the window you see, and it can connect to more than one Server.

## Features

- **Always on.** Close the window or disconnect. The Server and your agents keep working.
- **Remote access.** Manage local and remote devices in one window, over SSH, TCP or Peer-to-peer.
- **Agent status.** The sidebar shows what each agent is doing. Condr notifies you when an agent finishes.
- **Agent driven.** Agents can use Condr too: create panes, read output, assign tasks, and talk to other agents.
- **Real terminals.** Every pane is a real terminal built on the `alacritty` core. Run any CLI agent or terminal program.
- **Fast.** Written entirely in Rust, with no Electron. Small and responsive, even while agents are busy.

## Install

> [!WARNING]
> Condr 0.1 is a public preview, and things may still change between versions. Update the Client and the Server together; Condr marks a Device whose Server is another build, and tells you when a new version is out. For the latest changes, a [Nightly](https://github.com/condrdev/condr/releases/tag/nightly) build is published every day.

### Desktop App

Install it on the computer you work at. It includes the Server, so local use needs nothing else.

Download the installer for your platform from the [latest release](https://github.com/condrdev/condr/releases/latest):

| Platform             | Package  |
| -------------------- | -------- |
| Linux x86_64 / arm64 | AppImage |
| macOS x86_64 / arm64 | `.dmg`   |
| Windows x86_64       | `.exe`   |

The macOS app is not notarized (the Apple Developer account registration is not done yet), so the first launch says the app cannot be verified. Click **Done**, open **System Settings › Privacy & Security**, scroll down and click **Open Anyway**. Or clear the download flag once in a terminal:

```sh
xattr -cr /Applications/Condr.app
```

### Headless Server

Install it on the device where you want to run agents remotely. It includes only the `condr` command, with no graphical interface.

On that device, run the command for its platform:

| Platform             | Install command                                 |
| -------------------- | ----------------------------------------------- |
| Linux x86_64 / arm64 | `curl -fsSL https://condr.dev/install.sh \| sh` |
| macOS x86_64 / arm64 | `curl -fsSL https://condr.dev/install.sh \| sh` |
| Windows x86_64       | `irm https://condr.dev/install.ps1 \| iex`      |

## Connect to a Remote Device

In the sidebar, click **Connect Remote Device** and enter the link to the remote device. Three kinds of link are supported:

- **SSH** (`ssh://`): when you already have SSH access. Uses your existing OpenSSH configuration.
- **TCP** (`tcp://`): when the remote device has a fixed address. Paired with a one-time invite and encrypted with `Noise_IKpsk2`.
- **Peer-to-peer** (`p2p://`): when both machines are behind NAT. End-to-end encrypted, direct when possible, relayed when not.

See [Connect to a Remote Device](https://condr.dev/docs/getting-started/#connect-to-a-remote-device) in the docs for the setup steps.

## Development

```bash
git clone https://github.com/condrdev/condr
cd condr
cargo build
cargo run -p condr-gui                   # needs a display (Windows / macOS / Linux desktop)

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --features condr-gui/test-support -- -D warnings
cargo test --workspace --features condr-gui/test-support
```

## License

[Apache License 2.0](LICENSE)
