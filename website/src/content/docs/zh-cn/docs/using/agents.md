---
title: Agent 集成
description: 在 Condr 中使用并查看命令行 Agent 的运行状态。
---

在多 Agent 并发协同开发时，了解各个命令行 Agent（如 Claude Code、Codex 等）是否在后台完成任务或正在等待授权是保持高效流程的关键。Condr 提供了直观的状态监控与通知机制。

## 准备与检查 Agent

在 Condr 中运行 Agent 前，需要确认 Condr 服务端能够识别对应的命令行工具。

- **检查可用 Agent**：在终端中运行 `condr agent available` 命令， Condr 会列出当前在环境变量 `PATH` 中找到的所有支持的 Agent 工具。
- **PATH 继承机制**：Condr 服务端在启动时继承系统 `PATH`，但不会读取 Shell 配置文件（如 `.zshrc`）中的 Alias 或自定义路径。
- **排查找不到命令**：请将 Agent 安装至系统全局 `PATH`（如 `/usr/local/bin`），或在终端环境中执行 `condr server restart` 重启服务端。

## 安装 Hook 监听精准状态

传统的终端管理工具依赖屏幕文本正则匹配，容易因输出变动导致误判。Condr 采用原生的 Hook 机制，由 Agent 主动上报，实现 100% 准确的状态感知。

- **一键安装 Hook**：执行命令 `condr agent hooks install <agent>`（例如 `condr agent hooks install claude`）。
- **图形化管理界面**：前往 **Settings › Device › Agent integrations** 直接安装。
- **查看与卸载**：使用 `condr agent hooks status <agent>` 查看状态；如需移除，执行 `condr agent hooks uninstall <agent>`。

## Agent 状态指示

安装 Hook 后，侧边栏会自动显示各个窗格（Pane）中 Agent 的实时运行状态：

| 状态标识 | 状态说明 |
| :--- | :--- |
| **Working** | 正在执行任务（推理思考、生成文本或调用工具）。 |
| **Blocked** | 阻塞状态，等待用户输入或授权（在 Needs you 中列出）。 |
| **Done** | 后台执行完毕。当焦点切回该 Pane 时自动重置为 Idle。 |
| **Idle** | 空闲状态，当前轮次完成，等待新指令。 |
| **Unknown** | 原生 Shell 会话，或未接入 Condr Hook 的 Agent。 |
| **Bell** | 终端发出响铃提醒（Bell 优先级高于常规生命周期状态）。 |

> **提示**：当处于后台的 Agent 完成任务或触发阻塞时，Condr 会推送系统原生通知，点击直接定位至相应窗格。

## 常见 Agent 配置须知

- **Codex**：完成 Hook 安装后，需在 Codex 内部运行 `/hooks` 命令并确认信任。
- **Antigravity**：安装后需在其设置中打开 Condr 插件。
- **Pi / Oh My Pi**：若使用了多个 Profile，每个 Profile 均需独立安装一次 Hook。
