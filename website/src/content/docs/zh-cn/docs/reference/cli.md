---
title: CLI 参考
description: condr 命令行工具的语法格式、核心工作流、子命令参考及自动化集成规范。
---

`condr` 是一个面向多端与 AI Agent 协作的高性能终端复用控制工具。它既可在交互式终端中操作，也能作为无头脚本驱动后端会话。

运行 `condr <命令组> <子命令> --help` 可查看各命令的动态帮助信息。

---

## 命令行格式与语法规范

```text
condr [--device <名称>] <命令组> <子命令> [参数...]
condr [选项]
```

### 符号约定

* `<参数>`：必需参数。
* `[参数]`：可选参数（在 Condr Pane 内通常继承当前上下文）。
* `a|b`：互斥选项。
* `--`：其后的参数原样传递给下游进程（如 Shell 或 Agent）。

### 全局选项

| 选项 | 说明 |
| --- | --- |
| `--device <名称>` | 指定目标远程设备（名称需与侧栏配置一致），支持置于命令组前或末尾。可通过环境变量 `CONDR_DEVICE` 设置默认值。*注意：`server` 组与 `agent hooks` 不支持该选项。* |
| `--skill` | 打印内置 Agent Skill 描述文本（用于直接注入 AI 上下文），不连接 Server 进程。 |
| `-V, --version` | 查看版本信息。 |
| `-h, --help` | 显示使用说明。未传递子命令时返回帮助信息（退出码为 `2`）。 |

---

## 上下文与执行环境

`condr` 在不同的运行环境下具有智能的上下文继承策略：

```text
[当前环境]
 ├── Condr Pane 内部 ────► 读取环境变量 CONDR_SOCKET_PATH / CONDR_PANE_ID
 │                         默认省略 Workspace、Tab、Pane 参数
 │
 ├── 外部终端 (Bash/Zsh) ─► 直连本机默认 Server Socket
 │                         无当前上下文，需显式传入 ID
 │
 └── 附加 --device ──────► 连接远程设备 Server
                           本地上下文失效，路径与 ID 必须显式传入
```

> **提示**：如果 Server 尚未运行，绝大多数命令不会隐式启动服务，而是返回标准错误 `server_not_running`（退出码 `1`）。

---

## 常用工作流

### 切分 Pane 并执行任务

在当前 Tab 中向右切分出一个新 Pane，并在其中执行构建脚本：

```bash
# 切分 Pane 并获取新建的 Pane ID
NEW_PANE=$(condr pane split --direction right | jq -r '.pane.pane_id')

# 发送执行指令并回车
condr pane run "$NEW_PANE" "cargo build --release"
```

### 跨机远程操作

在名为 `mac-mini` 的远程设备上新建工作区并查看终端内容：

```bash
# 远程创建工作区
condr --device mac-mini workspace create --cwd "/home/dev/project" --label "Backend"

# 读取远程指定 Pane 的最新 50 行终端输出
condr --device mac-mini pane read 3 --lines 50
```

### Agent 自动化接入

启动一个自主 Agent 并等待其执行完成：

```bash
# 在指定 Pane 启动 Claude Agent
condr agent start claude-agent --kind claude --pane 1

# 发送任务提示并等待其进入就绪/等待确认状态（超时 120 秒）
condr agent prompt claude-agent "优化当前目录下的构建配置" --wait --timeout 120000
```

---

## 命令参考手册

### `server`

管理本机服务生命周期、监听端口及设备配对。本组命令只作用于本机，不支持 `--device`。

```bash
# 后台启动 Server，已在运行时不做任何事
# --listen 与 --p2p 会持久化写入配置文件
condr server start [--listen <IP:PORT>] [--p2p] [--snapshot <路径>]

# 重启 Server，基于最新快照恢复布局；禁止在 Condr Pane 内部调用
condr server restart [--listen <IP:PORT>] [--p2p] [--snapshot <路径>]

# 安全停止 Server；若服务未运行仍返回退出码 0
condr server stop

# 前台临时调试运行；配置项仅本次有效，不写入配置文件
condr server run [--listen <IP:PORT>] [--p2p] [--snapshot <路径>] [--endpoint <路径>]

# 查看运行状态、运行时长、会话计数及最近错误；未运行退出码为 1
condr server status [--json]

# 将当前二进制安装到用户执行路径并注册 PATH
condr server install [--start] [--restart] [--yes] [--json]

# 停止 Server 并清理二进制与 PATH 项（保留用户数据与日志）
condr server uninstall [--yes] [--json]

# 生成一次性配对链接（有效期 10 分钟），需先开启监听或 P2P
condr server invite

# 列出所有已授权的远程设备指纹与连接状态
condr server clients

# 吊销指定客户端权限并立即断开连接；支持唯一前缀匹配
condr server revoke <指纹或前缀>

# 将 Stdio 桥接至本地 Socket；通常由 SSH 自动化调用，无需手动执行
condr server bridge [--endpoint <路径>]
```

---

### `workspace`

```bash
# 列出所有工作区
# --all-devices 并行聚合所有远程设备的工作区，连不上的列在 unreachable 下
condr workspace list [--all-devices]

# 创建工作区；--cwd 默认继承当前目录（跨机调用 --device 时为必填项）
condr workspace create [--cwd <路径>] [--label <名称>] [--focus]

# 获取指定工作区元数据及 Tab、Pane 计数
condr workspace get <id>

# 将全部已连接的 GUI 窗口聚焦至该工作区
condr workspace focus <id>

# 重命名指定工作区
condr workspace rename <id> <新名称>

# 关闭工作区并终止其附属的所有终端进程（不影响磁盘文件）
condr workspace close <id>
```

---

### `tab`

```bash
# 列出 Tab，支持按 Workspace 过滤
condr tab list [--workspace <id>]

# 新建 Tab，Shell 默认继承当前 Pane 目录
# 在 Condr 外部或跨机调用时 --workspace 为必填项
condr tab create [--workspace <id>] [--label <名称>] [--focus]

# 查询、切换聚焦或重命名
condr tab get <id>
condr tab focus <id>
condr tab rename <id> <新名称>

# 关闭 Tab；若关闭的是最后一个 Tab，其父级 Workspace 也会自动关闭
condr tab close <id>
```

---

### `pane`

*注：命令中的 `[id]` 若省略，均默认指向当前上下文中的 Pane。*

#### 布局与管理

```bash
# 查看所有窗格及关联的 Agent 状态
condr pane list [--workspace <id>]

# 查看指定窗格或当前窗格的详情
condr pane get <id>
condr pane current

# 切分窗格并在同一目录启动默认 Shell
condr pane split --direction right|down [--focus] [id]

# 聚焦指定 Pane（并让所有窗口切换到其所在 Tab），或按相对方位切换
condr pane focus [--direction <left|right|up|down>] [id]

# 调整边缘，默认调整 0.05
condr pane resize --direction <left|right|up|down> [--amount <比例>] [id]

# 与指定方向的相邻窗格互换位置
condr pane swap --direction <left|right|up|down> [id]

# 重新锚定窗格相对拓扑
condr pane move --to <目标ID> --side <left|right|up|down> [id]

# 最大化当前 Pane 或恢复分屏视图
condr pane zoom [--on|--off] [id]

# 打印当前 Tab 的分割树 JSON（包含各窗格边界及邻居）
condr pane layout [id]

# 终止窗格内进程并关闭；若关闭的是最后一个 Pane，其所在 Tab 也会自动关闭
condr pane close <id>
```

#### 交互与 I/O 自动化

```bash
# 获取指定窗格的终端文本内容（含滚动缓存，默认最后 80 行）
condr pane read <id> [--lines <N>]

# 向终端注入原始文本（不附带回车符）
condr pane send-text <id> <文本>

# 发送控制键或组合键：enter、esc、tab、ctrl+c、alt+shift+x、f1~f20
condr pane send-keys <id> <按键...>

# 快捷指令：向终端粘贴命令文本并立即注入 enter 回车
condr pane run <id> <命令>
```

---

### `agent`

`condr` 原生支持识别与调度主流终端 AI Agent。`<目标>` 参数支持使用 `agent start` 时命名的自定义别名（`[a-z][a-z0-9_-]{0,31}`），亦可直接传入其所在的 Pane ID。

#### 运行与协同

```bash
# 扫描并列出 Server PATH 中已安装且被支持的 Agent CLI 工具
condr agent available

# 查询所有活动 Agent 进程、运行状态、阻塞原因（blocked_on）及原生会话 ID
condr agent list

# 在空闲 Pane 中初始化并启动 Agent，等待其就绪；默认超时 30 秒（30000ms）
condr agent start <名称> --kind <种类> --pane <id> [--timeout <毫秒>] [-- 附加参数...]

# 下发任务提示；--wait 阻塞直至 Agent 返回目标状态（默认等到 idle 或 blocked）
# --until 与 --timeout 需配合 --wait 使用
condr agent prompt <目标> <内容> [--wait] [--until <状态>] [--timeout <毫秒>]

# 阻塞等待 Agent 状态流转（常用于异步轮询或长任务同步），不发送输入
# 默认超时 120 秒（120000ms）
condr agent wait <目标> [--until <状态>] [--timeout <毫秒>]
```

*支持的种类 (`--kind`)*：`claude`、`codex`、`opencode`、`pi`、`omp`、`antigravity`、`grok`、`cursor`、`copilot`、`kimi`。

#### Hook 集成

```bash
# 查看特定 Agent 的状态探测钩子：installed / outdated / missing / unsupported
condr agent hooks status <种类>

# 为指定 Agent 自动配置或清理环境集成钩子；只修改本机配置
condr agent hooks install <种类>
condr agent hooks uninstall <种类>
```

---

### `device`

```bash
# 列出 config.toml 中配置的所有远程设备，包含连通性（reachable）及远程工作区数量
condr device list
```

> 提示：设备的添加、重命名与注销建议在 GUI 侧栏中直观完成，命令行只读取该配置。

---

## 自动化与脚本规范

### 输出规范

1. **结构化数据（JSON）**：除 `pane read` 输出原始终端字符、`server` 组默认输出交互文本外，其余所有子命令**默认输出机器可读的标准 JSON**。`server status`、`install`、`uninstall` 附加 `--json` 时同样输出 JSON。
2. **错误输出（Stderr）**：发生业务逻辑或网络错误时，命令行以标准 JSON 输出至标准错误（Stderr）：

```json
{
  "error": {
    "code": "pane_not_found",
    "message": "pane 9 not found"
  }
}
```

*注意：`server` 守护进程类命令错误格式为交互式文本（以 `error: ` 开头）。*

### 退出码

| 退出码 | 类型 | 场景说明 |
| --- | --- | --- |
| `0` | **成功 (Success)** | 命令正常执行并返回。 |
| `1` | **执行错误 (Runtime Error)** | 业务逻辑失败、找不到资源、权限拒绝或网络不可达。Stderr 返回对应错误信息。 |
| `2` | **语法错误 (Usage Error)** | 缺失必填参数、提供了未知选项或标记解析失败。 |

### 常见错误码列表

在脚本编写中，建议通过解析 `error.code` 做针对性容错：

| 错误码 | 触发原因及处理建议 |
| --- | --- |
| `server_not_running` | Server 未启动。可通过 `condr server start` 先行拉起。 |
| `no_current_pane` | 运行于 Condr 外部或附加了 `--device`，且未显式指定 Pane ID。 |
| `cwd_required` | 使用 `--device` 跨机创建 Workspace 时未传入 `--cwd` 绝对路径。 |
| `workspace_not_found` / `tab_not_found` / `pane_not_found` | 指定的实体 ID 不存在或已关闭。 |
| `pane_busy` | 目标窗格已有 Agent 或待启动的 Agent，或其 Shell 尚未就绪。 |
| `agent_timeout` | 在 `agent start`、`agent prompt` 或 `agent wait` 设定的超时时间内未等到指定状态。 |
| `not_authorized` | 本机 Device key 不在目标设备的配对名单中，或已被吊销。 |
| `local_only` | 尝试通过 `--device` 远程执行 `agent hooks`，该命令只能在目标机器本地运行。 |
