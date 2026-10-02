---
title: 核心概念
description: 掌握 Condr 的实体分层模型（Device、Server、Workspace、Tab、Pane、Agent）与核心设计哲学。
---

Condr 是一个**以终端为中心、面向多 Agent 协同**的统一工作台。与把大模型做在 Webview 聊天框中的方案不同，Condr 将各厂商原生的 Agent CLI 直接运行在真实的终端环境里，在保持原生体验的同时提供跨设备、多分支的编排与状态监控。

---

## 核心设计原则

- **终端优先，原生运行**：不重做对话 UI，不接管对话循环。Claude Code、Codex 等 Agent 作为原生子进程运行在 PTY 中，完整保留上下文压缩、本地配置与工具生态。
- **运行时与视图解耦**：服务端（`condr-server`）托管终端进程、Agent 状态与布局快照，是唯一真源；客户端窗口只是视图投影。关闭窗口不影响后台任务，多端同时接入互不干扰。
- **确定性状态感知**：拒绝脆弱的屏幕字符正则抓取。仅通过为 Agent 挂载官方生命周期 Hook，借助带内控制序列（OSC 777）主动上报精准状态与等待原因。

---

## 实体层级模型

Condr 的资源与界面结构按严谨的树状层级组织：

```text
Device（物理机/虚拟机，持有唯一 Ed25519 密钥）
└── Server（单机唯一的常驻后台服务，掌管运行时）
    └── Session（服务端维护的工作区拓扑与分屏布局快照）
        ├── Workspace 1（主工作区：绑定根目录，如 ~/code/my-app）
        │   ├── Tab 1（终端页：承载分屏 Pane，运行 Agent 或普通 Shell）
        │   ├── Tab 2 Diff（代码审查：查看单文件相对 HEAD 的差异）
        │   └── Tab 3 Preview（源码预览：语法高亮浏览文件）
        │
        └── Workspace 2（托管工作树：基于 Git Worktree 的隔离分支环境）
            └── Tab 1（终端页）
                └── Pane 1（运行另一任务的 Agent）
```

| 实体 | 职责与生命周期 |
| :--- | :--- |
| **Device / Server** | 运行 Condr 的机器抽象。每台设备持有唯一的加密身份（Device Key），关闭客户端不影响 Server 运行。 |
| **Session** | 服务端维护的布局拓扑。布局变动实时写入快照，Server 重启后自动恢复结构与 Agent 会话上下文。 |
| **Workspace** | 项目任务单元，锚定唯一的绝对路径**根目录**。支持基于 Git Worktree 派生出隔离的 **Managed Worktree** 独立工作区。 |
| **Tab** | Workspace 内的页面。分为**终端 Tab**（承载分屏窗格，支持快捷键 `1~9` 切换）与**查看 Tab**（Diff / Preview 只读审查页）。 |
| **Pane / Terminal** | 布局叶子节点与虚拟终端引擎（基于 Rust `alacritty_terminal`）。支持水平/垂直切分、缩放与进程树安全级联退出。 |
| **Agent** | 运行在 Pane 内受识别的命令行程序。状态实时同至侧边栏，支持阻断原因提取。 |

---

## 关键机制解析

### 1. 会话快照与重启恢复
请区分 **Condr Session**（服务端维护的窗口/分屏拓扑结构）与 **Agent Conversation**（Agent 自身在磁盘存储的对话上下文）：
- **快照记录**：Server 实时保存工作区树形结构、窗格拆分比例及正在运行的 Agent 原生会话 ID。
- **平滑接续**：重启 Server 后，Condr 自动重建分屏并恢复工作目录；对支持 Hook 的 Agent 自动执行 resume 指令重连上下文。*(注：不保存终端历史输出屏幕字符与进程内存状态)*。

### 2. Managed Worktree（隔离分支）
多 Agent 并发修改同一项目时极易产生代码与 Git 冲突：
- Condr 允许为仓库创建 **Managed Worktree**，在独立的 `<repo>.worktrees/<branch>` 目录检出新分支并作为独立 Workspace 打开。
- 移除工作区时强制校验 Clean 状态，仅安全清理独立工作树目录，保留 Git 分支与提交。

### 3. Client 独立视图与多端协作
- **视图独立**：每个连入的 Client 独立记录自己正在查看的 Workspace 与 Tab。多设备接入同一 Server 时，切换视角互不干扰。
- **主从保护**：同一终端 Pane 在同一时刻仅允许一个主控端输入按键；并发接入的第二台设备自动进入 **Viewing only**（只读观察）状态，防止输入冲突。

### 4. Agent 状态指示灯

| 状态 | 含义 | 侧边栏图标 |
| :--- | :--- | :--- |
| **Unknown** | 普通终端 Shell，或未配置 Hook 的 Agent | 灰色图标 |
| **Idle** | 当前轮次执行完毕，等待你的下一条指令 | 空心圆环 |
| **Working** | 正在生成、思考或执行本地工具命令 | 琥珀色实心 |
| **Blocked** | 遇到阻断（如等待用户授权 Bash 执行、等待回答），附带明确原因 | 红色警示 |
| **Done** | 切到其他窗口期间，后台任务已完成 | 绿色勾选 |

---

## 快速查阅

- [工作区与分屏操作](/zh-cn/docs/workspaces/)：分屏切分、快捷键及 Managed Worktree 使用。
- [Agent 适配与 Hook](/zh-cn/docs/agents/)：受支持的 Agent 列表与状态上报配置。
- [代码审查与改动对比](/zh-cn/docs/changes/)：使用 Diff Tab 和 Preview Tab 审查文件。
- [远程连接指南](/zh-cn/docs/remote/)：配置 SSH、TCP 局域网配对或 P2P 穿透直连。
