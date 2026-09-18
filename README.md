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
- **Remote access.** Manage local and remote devices in one window, over SSH or TCP.
- **Agent status.** The sidebar shows what each agent is doing. Condr notifies you when an agent finishes.
- **Agent driven.** Agents can use Condr too: create panes, read output, assign tasks, and talk to other agents.
- **Real terminals.** Every pane is a real terminal built on the `alacritty` core. Run any CLI agent or terminal program.
- **Fast.** Written entirely in Rust, with no Electron. Small and responsive, even while agents are busy.

## Install

> [!WARNING]
> Condr is in early development. A new [Nightly](https://github.com/condrdev/condr/releases/tag/nightly) build is published every day. The Client and the Server must be the same build.

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

   ```bash
   condr server start --listen 0.0.0.0:2637
   ```

2. Run `condr server invite`. The command prints the pairing link.
3. Enter the pairing link in Condr within 10 minutes. If it expires, repeat step 2.

Both sides authenticate each other with static keys (`Noise_IKpsk2`), and the connection is encrypted.

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
