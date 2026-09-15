# Condr

<p align="center">
  <img src="packaging/icons/condr.svg" alt="Condr" width="64" />
</p>

<p align="center">
  <b>让你的 Agent 持续运行</b>
  <br />
  一个窗口，管理所有的 Agent。本地、远程都一样，随时离开，随时继续。
</p>

<p align="center">
  <a href="README.md">English</a> · 简体中文
</p>

<p align="center">
  <a href="https://github.com/condrdev/condr/releases"><img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-666666?labelColor=333333" alt="Linux, macOS and Windows" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-666666?labelColor=333333" alt="Apache 2.0 license" /></a>
</p>

<p align="center">
  <img src="docs/images/hero.png" alt="Condr 同时连接本地和远程 Server，每个 Workspace 里并排运行 Claude Code 和 Codex" />
</p>

Condr 是一个轻量的原生 GUI，你可以在一个窗口中同时运行 Claude Code，Codex 等 CLI agent，也可以运行其他终端程序。

- **始终运行** —— `condr` Server 会持续运行。关闭窗口或断开连接后，agent 仍会继续工作。
- **远程访问** —— 在一个窗口管理本地和远程设备。通过 SSH 连接，或使用密钥通过 TCP 配对远程设备。
- **Agent 感知** —— 在侧边栏查看每个 agent 的状态。完成工作时，Condr 会通知你。
- **Agent 驱动** —— Agent 也可以使用 Condr：创建 Pane、读取输出、分配任务，还能与其他 Agent 沟通。
- **原生终端** —— Condr 使用 `alacritty` 内核，为每个窗格提供真实终端。任何 CLI agent 或终端程序都能运行。
- **Fast** —— 全程使用 Rust 构建。没有 Electron。小巧、响应快，即使 agent 忙碌时也能保持流畅。

## 安装

> [!WARNING]
> Condr 目前处于早期开发阶段，`main` 有变化的每一天都会发布新的 [Nightly](https://github.com/condrdev/condr/releases/tag/nightly) 构建。Client 和 Server 必须使用同一构建版本。

请前往 [Releases](https://github.com/condrdev/condr/releases) 下载安装包

### Desktop

| Platform             | 安装包   |
| -------------------- | -------- |
| Linux x86_64 / arm64 | AppImage |
| macOS x86_64 / arm64 | `.dmg`   |
| Windows x86_64       | `.exe`   |

### Headless server

| Platform             | 安装命令                                        |
| -------------------- | ----------------------------------------------- |
| Linux x86_64 / arm64 | `curl -fsSL https://condr.dev/install.sh \| sh` |
| macOS x86_64 / arm64 | `curl -fsSL https://condr.dev/install.sh \| sh` |
| Windows x86_64       | `irm https://condr.dev/install.ps1 \| iex`      |

你可以在侧栏中点击 **Connect Remote Device** 添加远程连接：

```text
ssh://user@build-box                         # 复用你的 OpenSSH 配置和 ssh-agent
tcp://<server key>.<invite>@<host>:<port>    # 由 `condr server invite` 生成
```

- 远端安装 `condr` 后，SSH 会直接转发远端私有 socket，无需额外配置。
- TCP 使用静态密钥验证双方身份（Noise_IKpsk2）。invite 有效期为 10 分钟。

**注意：** TCP 端点默认禁用，需要传入 `--listen` 参数启动，否则 Server 不会监听网络。

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

## License

[Apache License 2.0](LICENSE)
