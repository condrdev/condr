---
title: 快速上手
description: 安装 Condr，打开第一个 Workspace，连接远程设备。
---

Condr 由两部分组成：

- **Server**：命令 `condr`。它运行所有 Agent，并在后台持续工作。
- **Client**：应用 `condr-gui`。它是你看到的窗口，可以同时连接多个 Server。

## 安装

:::caution
Condr 0.1 是公开预览版，版本之间仍可能有变化。Client 和 Server 请一起更新；Server 版本不同的设备会被标出，有新版本时 Condr 也会提示。想用最新改动，可以装每天发布的 [Nightly](https://github.com/condrdev/condr/releases/tag/nightly) 版本。
:::

### 桌面应用

安装在你使用的电脑上。它已包含 Server，本机使用不需要再安装其他组件。

从[下载页](/download/)下载对应平台的安装包：

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

脚本下载 headless 版本，用 `SHA256SUMS` 校验后运行 `condr server install`。程序装到 `~/.local/opt/condr`（Windows 上是 `%LOCALAPPDATA%\Programs\Condr`），并加入 PATH。更新时如果 Server 正在运行，脚本会问是否重启 Server 让新版本接管；重启之前，应用会显示该设备版本不一致。要确认安装成功，打开一个新终端运行 `condr --help`。

再次运行时，如果已经装了最新版本，脚本不下载直接退出。要安装指定版本、装完立即启动 Server，或者强制重装：

```sh
CONDR_VERSION=nightly curl -fsSL https://condr.dev/install.sh | sh   # nightly（默认是最新 release）
CONDR_VERSION=v0.1.0 curl -fsSL https://condr.dev/install.sh | sh    # 某个版本
curl -fsSL https://condr.dev/install.sh | sh -s -- --start            # 同时启动 Server
curl -fsSL https://condr.dev/install.sh | sh -s -- --force            # 已是最新也重装
```

```powershell
$env:CONDR_VERSION = 'nightly'; irm https://condr.dev/install.ps1 | iex
$env:CONDR_VERSION = 'v0.1.0'; irm https://condr.dev/install.ps1 | iex
$env:CONDR_INSTALL_ARGS = '--start'; irm https://condr.dev/install.ps1 | iex
$env:CONDR_INSTALL_ARGS = '--force'; irm https://condr.dev/install.ps1 | iex
```

要装到其他位置，设置 `CONDR_INSTALL_DIR`。

## 打开第一个 Workspace

打开 Condr。如果本机没有运行中的 Server，它会启动一个；之后再打开时，它会重新连上已有的 Server。

添加一个 Workspace，打开一个窗格，运行你想用的 Agent 命令行工具。关闭窗口不会停止 Server 和 Agent，再次打开 Condr 就能从上次离开的地方继续。

## 连接远程设备

在侧栏点击 **Connect Remote Device**，然后输入远程设备的链接。链接有 SSH、TCP 和 Peer-to-peer 三种格式：已有 SSH 登录时用 SSH，远程设备有固定地址时用 TCP，两台机器都没有固定地址时用 Peer-to-peer。

### SSH

```text
ssh://user@build-box
```

远程设备装好 `condr` 就能连接。Condr 沿用你现有的 OpenSSH 配置和 ssh-agent，不需要其他设置。

### TCP

```text
tcp://<server key>.<invite>@<host>:<port>
```

TCP 监听默认关闭。配对链接由远程设备生成。在该设备上：

1. 启动 Server 并让它在网络上监听。如果 Server 已在运行，把 `start` 换成 `restart`。监听地址会保存到配置文件，之后不用再传：

   ```sh
   condr server start --listen 0.0.0.0:2637
   ```

2. 运行 `condr server invite`，命令会打印配对链接。
3. 在 10 分钟内把配对链接输入 Condr。过期了就重复第 2 步。

双方用静态密钥互相认证（`Noise_IKpsk2`），连接全程加密。

### Peer-to-peer

```text
p2p://<server key>.<invite>
```

适合两台都在 NAT 后面的机器，比如家里的台式机和公司的笔记本。不需要固定 IP，也不用装 VPN。Peer-to-peer 默认关闭。在远程设备上：

1. 启动 Server 并开启 Peer-to-peer。如果 Server 已在运行，把 `start` 换成 `restart`。这个设置会保存到配置文件：

   ```sh
   condr server start --p2p
   ```

2. 运行 `condr server invite`，命令会打印配对链接。
3. 在 10 分钟内把配对链接输入 Condr。过期了就重复第 2 步。

两台设备能直连时直连，不能直连时经 Condr 的中继转发。两台设备之间的连接用它们的密钥端到端加密，中继看不到你发送的内容。中继是 Condr 唯一的托管服务，从不使用 Peer-to-peer 的设备永远不会连接它。中继能看到什么、看不到什么，见 [SECURITY.md](https://github.com/condrdev/condr/blob/main/SECURITY.md)。
