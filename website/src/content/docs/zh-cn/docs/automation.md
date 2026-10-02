---
title: Agent 自动化与协同
description: 安装 Skill、配置环境变量，并利用 condr CLI 实现多 Agent 任务分发、自动化控制与跨设备调度。
---

在 Condr 中，Agent 不仅能回答问题和修改代码，还能通过内置的 `condr` 命令行工具反向操作终端界面，实现多 Agent 跨窗格协同或自动化任务编排。

## 工作原理

当 Agent 在 Condr 的终端窗格（Pane）中运行时，系统会自动注入以下环境变量，以便 Agent 和脚本识别 Condr 环境并直接调用 `condr` 命令：

- `CONDR_ENV=1`：标识当前运行于 Condr 窗格中。
- `CONDR_PANE_ID`：当前窗格的编号。
- `CONDR_SOCKET_PATH`：连接 Condr 服务端的 Socket 通讯路径。
- `CONDR_BIN_PATH`：`condr` 可执行文件的绝对路径。

在窗格中直接运行 `condr` 命令（如 `condr pane split`）会默认作用于当前所在的 Workspace 和 Tab。除 `pane read` 输出纯文本外，其他命令默认输出 JSON 结构，方便 Agent 解析处理。

## 安装 Condr Skill

为了让 Agent（如 Claude Code 等）更好地理解如何使用 Condr 的控制指令，你需要为其安装 Condr 的 Skill：

### 一键安装

在终端中运行以下命令，即可一键全局安装 Condr Skill：

```sh
npx -y skills add condrdev/condr -g
```

### 导出 Skill 说明（备用方案）

如果所在的网络隔离或需要手动注入提示词，可以在终端中运行以下命令直接输出完整的 Skill 内容：

```sh
condr --skill
```

> **提示**：如果在沙箱环境中使用 Agent，请确保沙箱允许访问 `CONDR_SOCKET_PATH` 指定的 Socket 路径。

## 核心控制命令

### 准备空间与启动 Agent
- **新建分屏/标签页**：
  ```sh
  condr pane split --direction right
  condr tab create
  ```
- **启动指定 Agent 并命名**：在闲置窗格中启动 Agent，分配名字以便后续调用：
  ```sh
  condr agent start reviewer --kind claude --pane 12
  ```

### 发送提示与等待响应
- **发送提示词并等待完成**：加上 `--wait` 参数后，命令会阻塞直到 Agent 变为空闲状态（Idle）或等待授权状态（Blocked）：
  ```sh
  condr agent prompt reviewer "审查 src/auth.rs 的改动，列出潜在问题" --wait
  ```
- **仅等待状态**：
  ```sh
  condr agent wait reviewer
  ```

### 读取与驱动终端
- **读取指定窗格输出**：等待 Agent 完成任务后，读取该窗格的终端输出：
  ```sh
  condr pane read 12 --lines 50
  ```
- **直接发送按键与文本**：
  ```sh
  condr pane run 12 "npm test"        # 粘贴命令并按回车
  condr pane send-keys 12 enter       # 发送指定按键（如 enter, esc, ctrl+c）
  ```

> **注意**：使用 `agent start` 和 `--wait` 之前，必须先在目标 Agent 上安装 Hook（见 [Agent 集成与状态](/zh-cn/docs/agents/)）。未安装 Hook 的 Agent 无法精准上报状态，会导致命令超时或无法返回。

### 跨设备（Device）控制
在命令行中加上 `--device` 参数，可以跨机器控制已保存的远程设备：

```sh
# 查看保存的远程设备列表
condr device list

# 在名为 build-box 的远程设备上列出 Workspace
condr --device build-box workspace list

# 查看所有设备上的 Workspace
condr workspace list --all-devices
```

`--device` 放在命令组之前，后面跟侧栏里显示的设备名；设置 `CONDR_DEVICE` 环境变量后可以省略它。`server` 和 `agent hooks` 只作用于本机，不接受 `--device`。

## 典型协同场景与 Prompt 模版

你可以将以下提示词复制发送给你的主控 Agent，实现常见的自动化协同流程：

### 场景一：并行调查后汇总报告
```text
在这个 Workspace 里用 condr 打开三个 Pane，各启动一个 claude。
一个追踪请求的处理路径，一个检查测试覆盖，一个查找相关的历史回归。
三者均不要修改文件。待三个 Agent 均完成后，读取它们的输出，合并为一份完整的报告给我。
```

### 场景二：代码编写与实时审查（一写一审）
```text
用 condr 在旁边打开一个 Pane 并启动 codex，将其命名为 reviewer。
接下来每当我让你完成一项代码改动后，将代码 Diff 发给 reviewer 进行审查，待其完成后将审查意见汇总汇报给我。
```

### 场景三：派发任务至远程机器
```text
用 condr --device build-box 在该远程机器的 ~/code/app 目录下打开 Workspace，
启动 claude 并让其运行完整测试套件与修复失败项。待其完成后将最终结果读取返回。
```

### 场景四：自动化构建监控与响应
```text
用 condr 在左侧窗格运行 npm run dev，在右侧窗格通过 condr pane read 监控左侧日志。
一旦检测到编译报错，立即提取报错信息并开始修复代码。
```
