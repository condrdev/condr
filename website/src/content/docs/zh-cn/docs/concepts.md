---
title: 核心概念
description: 认识 Condr 的术语，以及决定它工作方式的三个想法。
---

先认识这些词，再看其他 Condr 文档。你会知道侧栏中的每一项代表什么。

## 认识这些词

- **Device**：运行 Condr 的机器。侧栏为每台 Device 列出一项，包括你自己的机器。
- **Server**：Device 上的 `condr` 进程。下面的内容都由它负责。
- **Workspace**：一个项目文件夹，以及你在其中打开的终端。
- **Tab**：Workspace 中的一屏。一个 Tab 包含一个或多个 Pane。
- **Pane**：一个终端。分割 Tab 后会得到更多 Pane。
- **Agent**：Pane 中由 Condr 识别的程序，例如 Claude Code 或 Codex。Condr 在侧栏显示它的状态。

## 让 Server 管理运行时

Server 是 Workspace、终端和 Agent 状态的唯一来源。客户端连接 Server，并显示它报告的内容。本机和远程 Client 使用同一协议，因此通过网络连接时行为相同。

## 让 Agent 保持原生

Condr 将每个 Agent 自己的 CLI 作为子进程运行。对话和工具调用仍由 Agent 处理。Condr 提供终端，并显示 Agent 的活动。

## 把窗口当作 Client

关闭窗口只会断开连接。Server、它的终端和你的 Agent 会继续运行。重新打开窗口即可重新连接。
