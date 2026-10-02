---
title: 远程连接
description: 了解如何配置 Headless Server，并通过 SSH、TCP 及 P2P 三种方式安全连接远程设备，保持 Agent 持续稳定运行。
---

Condr 允许你在单个桌面窗口中跨机器管理多个 Agent 任务。通过将计算任务运行在远程设备上，你的 Agent 可以不受本地工作站重启或断网的影响，实现不间断高效工作。

## 常驻后台机制

Condr 采用客户端与服务端分离（Client/Server）的架构设计：

- **服务端（Server）**：即 `condr` 命令行程序，运行在目标设备上，负责管理终端进程并让 Agent 在后台持续运行。
- **客户端（Client）**：即 `condr-gui` 桌面应用，用于连接一个或多个服务端并提供图形界面交互。

在常驻后台模式下：

- 关闭桌面客户端窗口不会终止远程服务与正在运行的 Agent 任务。
- 网络短暂中断时，Server 将保持当前状态并在后台继续执行任务。
- 重新打开客户端并建立连接后，系统会自动同步最新的终端输出与 Agent 状态。

## 前置准备：安装 Headless Server

在连接远程设备前，需要在该设备上安装无图形界面的 Headless Server（即 `condr` 命令工具）。

如果那台设备已经安装了桌面应用，就不用再装：桌面应用自带 Server，打开应用时会自动启动，关闭窗口后 Server 也继续运行。只有没有图形界面的机器才需要按下面的命令安装。

### 安装命令

请根据远程设备的操作系统，在终端中执行对应的安装命令：

| 操作系统 | 安装命令 |
| :--- | :--- |
| **Linux**（x86_64 / arm64） | `curl -fsSL https://condr.dev/install.sh \| sh` |
| **macOS**（Apple Silicon / Intel） | `curl -fsSL https://condr.dev/install.sh \| sh` |
| **Windows**（x86_64） | `irm https://condr.dev/install.ps1 \| iex` |

安装完成后，可以在远程终端运行 `condr --version` 确认安装是否成功。

## 连接远程设备的方式

Condr 支持三种连接协议，以适应不同的网络拓扑环境。在客户端侧栏中点击 **Connect Remote Device** 即可开始连接。

### SSH 连接

适用于已配置 OpenSSH 访问权限的远程服务器或云主机。

- **适用场景**：已有 SSH 登录权限且配置了密钥对的服务器。
- **连接步骤**：
  1. 在客户端点击 **Connect Remote Device** → **SSH**。
  2. 输入以 `ssh://` 开头的地址，例如：
     - `ssh://user@192.168.1.100`
     - `ssh://my-cloud-server`（会自动读取本地 `~/.ssh/config` 中的主机别名）
  3. 确认连接。Condr 将利用本地 OpenSSH 配置建立加密通道，并自动启动或复用远程 `condr` 服务。

### P2P 连接

适用于双方均处于 NAT 或防火墙之后、无固定公网 IP 的网络环境。

- **适用场景**：跨网段的办公电脑、家庭开发机或复杂网络环境。
- **完整流程**：
  1. **启动 P2P 模式**：在远程设备上以 Peer-to-peer 模式启动服务，这个设置会写入配置。Server 已在运行时，把 `start` 换成 `restart`：

     ```sh
     condr server start --p2p
     ```

  2. **获取 P2P 链接**：运行 `condr server invite`，它会输出以 `p2p://` 开头、无需填写地址的链接。链接 10 分钟内有效，且只能使用一次。
  3. **客户端连接**：在客户端点击 **Connect Remote Device** → **Peer-to-peer**，粘贴 `p2p://` 链接。
  4. **建立连接**：Condr 将优先尝试点对点直连；若网络受限无法直连，则会自动通过加密中继节点转发数据。

### TCP 连接

适用于具有固定 IP 地址或可直接访问的内部局域网服务器。通信全程采用 `Noise_IKpsk2` 端到端加密算法保护。

- **适用场景**：拥有静态 IP 或在同一局域网下的设备，且不依赖 SSH 服务的环境。
- **完整流程**：
  1. **开启监听**：Server 默认只监听本机。在远程设备上带监听地址启动它，这个地址会写入配置，之后启动无需再传。Server 已在运行时，把 `start` 换成 `restart`：

     ```sh
     condr server start --listen 0.0.0.0:2637
     ```

  2. **生成配对邀请**：在远程设备上运行以下命令，生成一次性配对链接：

     ```sh
     condr server invite
     ```

  3. **复制配对链接**：控制台将输出格式为 `tcp://<device key>.<invite>@<host>:<port>` 的链接，把其中的 `<host>` 换成这台设备的 IP。链接 10 分钟内有效，且只能使用一次。
  4. **客户端连接**：在 Condr 客户端点击 **Connect Remote Device** → **TCP**，粘贴该链接并确认。
  5. **完成密钥握手**：客户端与服务端通过预共享密钥建立点对点安全信道。配对完成后，Condr 只保存 Device key 和地址，不保存 invite。

## 常见问题排查

### SSH 提示 condr: not found

**问题现象**：通过 SSH 连接时，客户端提示错误信息 `bash: condr: not found` 或 `command not found`。

**原因分析**：安装脚本把 `condr` 装在 `~/.local/opt/condr`，并在 `~/.local/bin` 建立软链接，再把这个目录的 PATH 配置写进 `~/.profile` 或 `~/.zprofile`。非交互式 SSH 连接通常不会加载这两个文件，导致系统找不到 `condr` 可执行程序。

**解决方法**：

**方法一：在连接地址中指定路径（推荐）**。不改动远程设备，直接在 SSH 地址里告诉 Condr 可执行文件在哪：

```text
ssh://user@host?bin=/home/user/.local/bin/condr
```

**方法二：配置环境变量至 Shell 初始化文件头部**：

1. 登录远程设备，确认 `condr` 的安装路径：

   ```sh
   which condr
   ```

2. 编辑远程设备上的 `~/.bashrc` 或 `~/.zshenv` 文件。
3. 将 PATH 变量设置放置在文件最顶部（必须位于非交互式 Shell 提前返回代码之前）：

   ```sh
   export PATH="$HOME/.local/bin:$PATH"
   ```

**方法三：创建系统级软链接**。在远程设备上将 `condr` 软链接至系统标准可执行路径中：

```sh
sudo ln -s ~/.local/bin/condr /usr/local/bin/condr
```

### TCP 连接超时或无法握手

如果使用 TCP 连接时一直处于连接超时状态，请按以下步骤排查：

1. **检查端口开放**：确保远程设备的防火墙及云服务商安全组已放行对应的 TCP 监听端口。
2. **校验 invite 时效**：`condr server invite` 生成的配对链接 10 分钟后过期，使用一次后也会失效。若已过期，请重新生成并在客户端输入新的链接。
3. **检查后台服务**：在远程设备上运行 `condr server status`，确认服务端进程处于运行状态，且 Listen 一行显示了监听地址。
