---
title: 快速上手
description: 安装 Condr，开始在一个窗口里运行你的 Agent。
---

Condr 在你的机器上运行 Claude Code、Codex 等 Agent 命令行工具。无论 Agent 在本机还是远程设备上，你都能随时随地操控它们。

Condr 目前提供桌面应用和 Headless 两种安装方式。

:::caution
Condr 0.1 是公开预览版，版本之间仍可能有变化。
:::

## 桌面应用（推荐）

从[下载页](/download/)下载安装包，打开即可。

桌面应用自带 Server 并自动启动，无须单独安装。第一次打开后：

1. 点 **Open Project**，选择项目文件夹。Condr 会打开一个 Workspace 并提供 shell。
2. 安装 Agent 集成：打开 **Settings › Device › Agent integrations**，在你用的 Agent 旁边点 **Install**。这样 Condr 才能追踪 Agent 的状态。
3. 在 shell 里像平时一样运行 Agent，例如 `claude`。

## Headless Server

如果你需要在服务器、开发机和其他不需要窗口的机器中使用，只需要安装 `condr` 命令：

Linux 或 macOS：
```sh
curl -fsSL https://condr.dev/install.sh | sh
```

Windows：
```powershell
irm https://condr.dev/install.ps1 | iex
```

安装完成后，请打开一个新的终端让新加入的 PATH 生效，然后启动 Server：

```sh
condr server start
```

Server 在后台运行，关掉终端也不会停止。目前它不会开机自启，所以机器重启后需要手动启动。

要从你的电脑连接这台机器，见[连接远程 Device](/zh-cn/docs/remote/)。用 SSH 连接时，Condr 会自动启动 Server，你可以跳过上面的步骤。

## 接下来

- [核心概念](/zh-cn/docs/concepts/)：Device、Server、Workspace、Pane 和 Agent 分别是什么。
- [Workspace、Tab 与 Pane](/zh-cn/docs/workspaces/)：分割 Pane，用 Worktree 给每个 Agent 一个独立分支。
- [Agent](/zh-cn/docs/agents/)：支持的 Agent、hook，以及每种状态的含义。
- [连接远程 Device](/zh-cn/docs/remote/)：用 SSH、TCP 或 Peer-to-peer 连接其他机器。
- [Agent 驱动 Condr](/zh-cn/docs/automation/)：让一个 Agent 用 `condr` 命令指挥其他 Agent。
- [CLI 参考](/zh-cn/docs/cli/)：每个 `condr` 命令。
- [故障排查](/zh-cn/docs/troubleshooting/)：出问题时先查什么。
