---
title: 快速上手
description: 十分钟内装好 Condr，跑起第一个 Agent，再连上一台远程 Device。
---

按下面的步骤装好 Condr，跑起一个 Agent，再连上第二台 Device。

Condr 由两部分组成：

- **Server**：命令 `condr`。它运行你的 Agent，关掉窗口后也继续工作。
- **窗口**：应用 `condr-gui`。它是你使用的界面。一个窗口可以同时显示多台 Device。

:::caution
Condr 0.1 是公开预览版，版本之间仍可能有变化。窗口和 Server 请一起更新。
:::

## 1. 安装桌面应用

从[下载页](/download/)下载对应平台的安装包，装在你工作用的电脑上。安装包已包含 Server。

| 平台 | 安装包 |
| --- | --- |
| Linux x86_64 / arm64 | AppImage |
| macOS x86_64 / arm64 | `.dmg` |
| Windows x86_64 | `.exe` |

macOS 可能提示应用无法验证，Windows 可能弹出 SmartScreen。预览版还没有签名。打开应用、使用安装脚本选项和安装 Headless Server 的步骤，见[安装](/zh-cn/docs/install/)。

## 2. 跑起第一个 Agent

1. 安装你要用的 Agent 命令行工具并登录，例如 Claude Code 或 Codex。Condr 不提供模型和账号。
2. 打开 Condr。它会启动本机 Server。
3. 点 **Open Project**，选一个项目文件夹，让 Condr 打开一个带 shell 的 Workspace。
4. 让 Condr 读取 Agent 的状态。在这个 shell 里运行一次：

   ```sh
   condr agent hooks install claude
   ```

   把 `claude` 换成你的 Agent。每个 Agent 装一次 hook。
5. 在同一个 shell 里像平时一样运行 Agent，例如 `claude`。侧栏显示它的状态：工作中是琥珀色，等你批准是红色。
6. 给它一个任务，然后关掉窗口。Server 和 Agent 会继续运行。
7. 再打开 Condr。Workspace 会回到原来的位置。Agent 如果已经完成，侧栏会标成绿色。

现在你已经会用 Condr 了。要同时跑几个 Agent，在 Pane 菜单里选 **Split Right**。也可以右键 Workspace，选 **Create Worktree**，给每个 Agent 一个独立分支。见 [Workspace、Tab 与 Pane](/zh-cn/docs/workspaces/)。

## 3. 连接远程 Device

让 Agent 在另一台机器上运行，并在同一个窗口里查看：

1. 在那台机器上安装 Headless Server：

   ```sh
   curl -fsSL https://condr.dev/install.sh | sh
   ```

   Windows 使用 `irm https://condr.dev/install.ps1 | iex`。
2. 在你的电脑上打开 Condr，点侧栏底部的 **Connect Remote Device**。选择 **SSH address**，输入你平时 SSH 登录用的地址：

   ```text
   user@build-box
   ```

3. 这台机器会出现在侧栏。点 **New Workspace**，之后用法和本机 Device 一样。

SSH 使用你现有的 OpenSSH 配置和密钥。没有 SSH 的机器可以用 TCP 或 Peer-to-peer 连接。见[连接远程设备](/zh-cn/docs/remote/)。

## 接下来

- [Agent](/zh-cn/docs/agents/)：了解状态含义和支持的 Agent。
- [Agent 驱动 Condr](/zh-cn/docs/automation/)：让一个 Agent 指挥其他 Agent。
- [键盘快捷键](/zh-cn/docs/keybindings/)。
- [故障排查](/zh-cn/docs/troubleshooting/)：出问题时先做哪些检查。
