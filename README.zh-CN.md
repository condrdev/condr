# Condr

<p align="center">
  <img src="assets/brand/condr.svg" alt="Condr" width="64" />
</p>

<p align="center">
  <b>让你的 Agent 一直运行</b>
  <br />
  一个窗口，管理所有 Agent。本机和远程一样用，随时离开，随时回来。
</p>

<p align="center">
  <a href="README.md">English</a> · 简体中文
</p>

<p align="center">
  <a href="https://github.com/condrdev/condr/releases"><img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-666666?labelColor=333333" alt="支持 Linux、macOS 和 Windows" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-666666?labelColor=333333" alt="Apache 2.0 许可证" /></a>
</p>

<p align="center">
  <img src="assets/screenshots/hero.png" alt="Condr 同时连接本地和远程 Server，每个 Workspace 里并排运行 Claude Code 和 Codex" />
</p>

Condr 是一个小巧的桌面应用。它在一个窗口里同时运行 Claude Code、Codex 和其他命令行 Agent，也能运行任何终端程序。

Condr 由两部分组成：

- **Server**：命令 `condr`。它运行所有 Agent，并在后台持续工作。
- **Client**：应用 `condr-gui`。它是你看到的窗口，可以同时连接多个 Server。

## 特性

- **一直运行。** 关闭窗口或断开连接后，Server 和 Agent 继续工作。
- **远程管理。** 在一个窗口里管理本机和远程设备。支持 SSH、TCP 和 Peer-to-peer 三种连接方式。
- **Agent 状态。** 侧栏显示每个 Agent 正在做什么。Agent 完成任务时，Condr 发送通知。
- **Agent 驱动。** Agent 也能操作 Condr：创建窗格、读取输出、分配任务、与其他 Agent 沟通。
- **真实终端。** 每个窗格都是基于 `alacritty` 内核的真实终端，能运行任何命令行程序。
- **快。** 全部用 Rust 编写，不使用 Electron。体积小，响应快，Agent 忙碌时界面不卡顿。

## 安装

> [!WARNING]
> Condr 0.1 是公开预览版，版本之间仍可能有变化。Client 和 Server 请一起更新；Server 版本不同的设备会在侧栏标出，有新版本时 Condr 也会提示。想用最新改动，可以装每天发布的 [Nightly](https://github.com/condrdev/condr/releases/tag/nightly) 版本。

### 桌面应用

安装在你使用的电脑上。它已包含 Server，本机使用不需要再安装其他组件。

从[最新版本](https://github.com/condrdev/condr/releases/latest)下载对应平台的安装包：

| 平台                 | 安装包   |
| -------------------- | -------- |
| Linux x86_64 / arm64 | AppImage |
| macOS x86_64 / arm64 | `.dmg`   |
| Windows x86_64       | `.exe`   |

macOS 应用没有经过 Apple 公证（尚未完成 Apple 开发者账号注册），首次启动会提示无法验证。点 **完成**，打开 **系统设置 › 隐私与安全性**，滚到底部点 **仍要打开**。或者在终端里清除一次下载标记：

```sh
xattr -cr /Applications/Condr.app
```

### Headless Server

安装在你想远程运行 Agent 的设备上。它只包含 `condr` 命令，没有图形界面。

在该设备上运行对应命令：

| 平台                 | 安装命令                                        |
| -------------------- | ----------------------------------------------- |
| Linux x86_64 / arm64 | `curl -fsSL https://condr.dev/install.sh \| sh` |
| macOS x86_64 / arm64 | `curl -fsSL https://condr.dev/install.sh \| sh` |
| Windows x86_64       | `irm https://condr.dev/install.ps1 \| iex`      |

## 连接远程设备

在侧栏点击 **Connect Remote Device**，然后输入远程设备的链接。支持三种链接：

- **SSH**（`ssh://`）：已有 SSH 登录时使用，沿用你现有的 OpenSSH 配置。
- **TCP**（`tcp://`）：远程设备有固定地址时使用，一次性邀请配对，`Noise_IKpsk2` 加密。
- **Peer-to-peer**（`p2p://`）：两台机器都在 NAT 后面时使用，端到端加密，能直连就直连，不能直连时经中继转发。

具体设置步骤见文档站的 [Connect to a Remote Device](https://condr.dev/docs/getting-started/#connect-to-a-remote-device)。

## 开发

```bash
git clone https://github.com/condrdev/condr
cd condr
cargo build
cargo run -p condr-gui                   # 需要显示器（Windows / macOS / Linux 桌面）

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --features condr-gui/test-support -- -D warnings
cargo test --workspace --features condr-gui/test-support
```

## 许可证

[Apache License 2.0](LICENSE)
