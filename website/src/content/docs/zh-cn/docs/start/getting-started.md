---
title: 快速上手
description: 安装 Condr，在统一工作区中运行并管理多 Agent 实例。
---

Condr 是一款用于集中管理与调度 Agent 命令行工具（如 Claude Code、Codex 等）的工作台。无论 Agent 运行于本地还是远程服务器，均可通过 Condr 实现统一监控与协同交互。

Condr 提供**桌面客户端**与 **Headless Server** 两种运行形态。

:::caution
Condr 0.1 目前处于公开预览（Public Preview）阶段，功能与配置项可能在后续版本中持续调整。
:::

## 桌面端（推荐）

前往[下载页](/download/)获取对应平台的安装包，解压/安装后直接启动。

桌面端已内置 Server 组件并默认随应用启动，无需额外配置：

1. **新建工作区**：点击 **Open Project** 选择本地项目目录，Condr 将自动创建 Workspace 并唤起终端。
2. **安装 Agent 集成**：进入 **Settings › Device › Agent integrations**，在需要支持的 Agent 项后点击 **Install**，以便 Condr 捕获并监听其运行状态。
3. **运行 Agent**：在内置终端中直接执行 Agent 命令（例如 `claude`）。

## Headless Server

适用于云主机、无 GUI 开发机或远程服务器环境，仅需安装 `condr` 核心 CLI：

**Linux / macOS：**
```sh
curl -fsSL https://condr.dev/install.sh | sh
```

**Windows：**
```powershell
irm https://condr.dev/install.ps1 | iex
```

安装完成后，请**重开终端窗口**以加载最新的环境变量，随后启动服务：

```sh
condr server start
```

:::note
- Server 会常驻后台运行，关闭终端不会中断进程。
- 如需连接此机器，请参阅[远程连接](/zh-cn/docs/using/remote/)。
- 当前预览版本尚未配置系统自启服务，机器重启后需手动执行上述启动命令。
:::

## 深入探索

- [核心概念](/zh-cn/docs/start/concepts/)：了解 Device、Server、Workspace、Pane 与 Agent 的架构模型。
- [Workspace、Tab 与 Pane](/zh-cn/docs/using/workspaces/)：配置多窗口分屏，结合 Git Worktree 为各 Agent 分配独立分支工作环境。
- [Agent 集成与状态](/zh-cn/docs/using/agents/)：查看兼容 Agent 列表、生命周期 Hook 及状态监听定义。
- [远程连接](/zh-cn/docs/using/remote/)：通过 SSH、TCP 或 P2P 方式接入远程设备。
- [Agent 自动化与协同](/zh-cn/docs/using/automation/)：通过 `condr` CLI 实现 Agent 间的协同调度。
- [CLI 参考](/zh-cn/docs/reference/cli/)：查看完整的指令与参数手册。
- [故障排查](/zh-cn/docs/help/troubleshooting/)：常见连接中断、状态脱轨及诊断排错方案。
