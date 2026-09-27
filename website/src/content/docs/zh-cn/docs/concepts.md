---
title: 核心概念
description: 了解 Condr 怎样组织 Device、Workspace、Tab、Pane 和 Agent。
---

Condr 围绕终端组织工作，而不是围绕聊天。终端是基本单位，Agent 是运行在终端里的程序。

## 从 Device 到 Pane

Condr 的内容按下面的层次组织：

```text
macbook                 Device：你的电脑
├── my-app              Workspace：一个项目文件夹
│   ├── 1               Tab：一屏终端
│   │   ├── claude      Pane：运行 Agent 的终端
│   │   └── zsh         Pane：普通 shell
│   └── 2 Diff          Tab：查看一个文件的改动
└── my-app-login        Workspace：同一仓库的另一个 worktree
build-box               Device：远程服务器
└── api
```

侧栏列出 Device 和其中的 Workspace。Tab 显示在 Tab 栏里，Pane 显示在 Tab 中。

## Device 运行一个 Server

- Device 是一台运行 Condr 的机器，包括你自己的电脑。
- 每台 Device 运行一个 Server。Server 运行这台机器上的所有终端和 Agent，并记住 Workspace 的布局。
- 窗口和 `condr` 命令都是连接 Server 的 Client。关掉窗口只会断开连接，Server 和 Agent 会继续运行。
- 本机和远程 Device 使用同一套协议，所以用法相同。连接方式见[连接远程 Device](/zh-cn/docs/remote/)。

## Workspace 是一个项目文件夹

Workspace 由一个根目录和你在其中打开的 Tab 组成。

- 新终端默认在根目录启动。在 shell 里运行 `cd` 不会改变根目录。
- 关闭 Workspace 会结束它所有终端里的程序，但不会删除任何文件或 worktree。

## Tab 和 Pane 组织终端

Tab 是 Workspace 中的一屏，按顺序编号。Tab 有两种：

- **终端 Tab**：包含一个或多个 Pane。
- **查看 Tab**：**Diff** 显示一个文件的改动，**Preview** 显示文件内容。每个 Workspace 最多各有一个。

Pane 是一个终端。分割 Pane，就能在同一个 Tab 里并排运行多个程序。关闭 Pane 会结束它启动的所有程序。

## Agent 运行在 Pane 里

Agent 是 Condr 在 Pane 里认出的 Agent 命令行程序，例如 Claude Code 或 Codex。

- Pane 不运行 Agent 时，就是一个普通终端。
- Condr 不和模型对话。对话和工具调用由 Agent 自己处理，对话记录也由 Agent 保存。
- Agent 的状态只来自它自己的 hook。没有装 hook 的 Agent 显示 Unknown，Condr 不会从屏幕文字猜测状态。见 [Agent](/zh-cn/docs/agents/)。

## 用 Worktree 隔离每个 Agent

两个 Agent 改同一个目录时，可能互相覆盖。Worktree 为同一个仓库另开一个目录和分支，Condr 把它作为一个新的 Workspace 打开。

- Condr 创建的 worktree 放在仓库旁的 `<repo>.worktrees/<branch>`。
- 只有 Condr 创建的 worktree 才能在 Condr 里删除。删除时保留分支。目录里有未提交或未跟踪的文件时，Condr 会拒绝删除。

操作步骤见 [Workspace、Tab 与 Pane](/zh-cn/docs/workspaces/#给每个-agent-分配独立分支)。

## Server 重启后保留什么

Server 会保存一份快照，记录 Workspace、Tab 和 Pane 的结构，以及每个 Pane 里 Agent 对话的引用。

- Server 重启后，Condr 按快照恢复结构。装了 hook 的 Agent 会重新打开原来的对话。
- 快照不包含终端历史和正在运行的程序，重启会结束这些程序。

详见 [Workspace、Tab 与 Pane](/zh-cn/docs/workspaces/#server-重启后剩下什么)。
