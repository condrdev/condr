# Condr

<p align="center">
  <img src="assets/brand/condr.svg" alt="Condr" width="64" />
</p>

<p align="center">
  <b>让 Agent 永不挂断</b>
  <br />
  一个窗口管理本地与远程所有 Agent。随时断开，随时接续。
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

Condr 是一个专为多 Agent 协同与终端管理打造的轻量应用。在一个窗口里同时运行 Claude Code、Codex 和任何命令行程序，无论是本地还是远程服务器。

Condr 由两部分组成：

- **Server**：常驻后台的服务端。负责托管所有终端进程与 Agent 状态，哪怕关闭窗口或网络中断，后台任务也不会挂断。
- **Client**：高性能原生桌面应用。作为可随时接入的视图窗口，支持在一处同时连接并管理多台 Server。

## 特性

- **保持运行** — 合上笔记本或断开连接时， Agent 和长任务不再中断，随时重新连入接续上下文。
- **远程管理** — 无需切窗口，在一个界面总览本地与远程所有 Agent。内置 SSH、TCP 和 P2P 免配置穿透直连。
- **Agent 状态** — 侧栏显示每个 Agent 正在做什么。Agent 完成任务时，Condr 会向你发送通知。
- **Agent 驱动** — Agent 也能反向控制 Condr，自动创建窗格、跨窗格任务分发、与其他 Agent 沟通。
- **真实终端** — 基于 `alacritty` 内核的真实终端，完整保留原生 Shell 生态，还支持跨设备的截图一键粘贴。
- **极致流畅** — 纯 Rust 全栈打造，不使用 Electron。体积小，响应快，Agent 忙碌时界面不卡顿。

## 安装

根据你的使用场景选择安装组件：
- **日常开发电脑**：安装 **桌面应用**（已内置 Server，开箱即用）。
- **远程服务器 / 云主机**：安装 **Headless Server**（仅包含命令行工具，无图形界面）。

> [!WARNING]
> Condr 0.1 为公开预览版，功能与协议可能随版本更新调整。请保持客户端（Client）与服务端（Server）版本一致。如需体验最新特性，可下载每天自动构建的 [Nightly](https://github.com/condrdev/condr/releases/tag/nightly) 版本。

### 桌面应用

安装在你使用的电脑上（已内置 Server，无需单独配置）。

从[最新版本](https://github.com/condrdev/condr/releases/latest)下载对应平台的安装包：

| 平台                 | 安装包   |
| -------------------- | -------- |
| Linux x86_64 / arm64 | AppImage |
| macOS x86_64 / arm64 | `.dmg`   |
| Windows x86_64       | `.exe`   |

### Headless Server

安装在你想远程运行 Agent 的设备上（仅包含 `condr` 命令行工具，无图形界面）。

在终端中运行安装命令：

| 平台                 | 安装命令                                        |
| -------------------- | ----------------------------------------------- |
| Linux x86_64 / arm64 | `curl -fsSL https://condr.dev/install.sh \| sh` |
| macOS x86_64 / arm64 | `curl -fsSL https://condr.dev/install.sh \| sh` |
| Windows x86_64       | `irm https://condr.dev/install.ps1 \| iex`      |

## 快速上手

30 秒体验 Condr 的核心工作流：

1. **创建工作区**：打开 Condr，在侧边栏点击 **New Workspace** 选择你的项目文件夹。
2. **启动 Agent**：在终端窗格中运行你常用的命令行 Agent（如 `claude` 或 `codex`）。
3. **开启 Agent 状态监听**：在设置中安装 Agent 集成，侧边栏即可实时显示 Agent 的思考、完成与提问授权状态。

**更多使用指南**：访问 [Condr 官方文档站](https://condr.dev/zh-cn/docs/start/getting-started/) 了解远程连接、Agent 自动化协同及快捷键等完整特性。

## 连接远程设备

Condr 支持在单个窗口中跨机器集中管理本地与远程的所有 Agent 任务。将计算任务部署在远程服务器上，可以让 Agent 彻底脱离本地工作站的限制，实现无视重启与断网的不间断运行。

- **SSH**：适用于已配置 OpenSSH 访问权限的远程服务器或云主机，完全复用现有配置。
- **P2P**：适用于双方均处于 NAT 或防火墙之后、无公网 IP 的网络环境，自动打洞直连或加密中继转发。
- **TCP**：适用于具有固定 IP 或可直接访问的局域网服务器，通信全程采用 `Noise_IKpsk2` 端到端加密。

具体设置步骤见官方文档站 [远程连接](https://condr.dev/zh-cn/docs/using/remote/)。

## 开发

```bash
git clone https://github.com/condrdev/condr
cd condr
cargo build
cargo run -p condr-gui

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --features condr-gui/test-support -- -D warnings
cargo test --workspace --features condr-gui/test-support
```

## 许可证

[Apache License 2.0](LICENSE)
