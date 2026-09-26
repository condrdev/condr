---
title: CLI 参考
description: 查找 condr 的子命令、参数、JSON 输出和退出码。
---

先在这里找到命令，再运行 `condr <组> <命令> --help` 查看完整参数。帮助输出才是权威说明。

## 使用命令格式

```text
condr [--device <名字>] <组> <命令> [参数]
```

- **`--device <名字>`** 让命令作用于保存的远程 Device，名字与侧栏一致。用 `CONDR_DEVICE` 设置默认值。 `server` 和 `agent hooks` 不接受它。
- **`--skill`** 打印内置 Agent Skill，不连接 Server。
- **在 Pane 中运行时**，可省略的 Pane、Tab 和 Workspace 参数默认使用当前 Pane。
- **输出**：`workspace`、`tab`、`pane`、`agent` 和 `device` 输出 JSON。`pane read` 输出文本。`server` 输出人类可读文本。
- **退出码**：0 表示成功，1 表示失败，并把 `{"error":{"code","message"}}` 写到 stderr，2 表示用法错误。这些命令不会启动 Server。Server 未运行时，错误码是 `server_not_running`。
- **常见错误码**：`server_not_running`、`protocol_mismatch`、`not_authorized`、`device_not_found`、`workspace_not_found`、`pane_not_found`、`agent_unavailable`、`agent_not_ready`、`local_only`。

## 用 `condr server` 管理 Server

| 命令 | 作用 |
| --- | --- |
| `start` | 后台启动 Server，已运行时不做任何事。`--listen <IP:端口>` 和 `--p2p` 会写入配置并立即生效。 |
| `restart` | 停止再启动，结束所有 Pane 中的程序，并按快照恢复结构。不能在 Condr Pane 中运行。 |
| `stop` | 请求 Server 关闭。即使没有运行也返回 0。 |
| `run` | 在前台运行。`--listen` 和 `--p2p` 只对本次运行生效，不写入配置。 |
| `status` | 显示运行状态、运行时长、Workspace/Tab/Pane/Agent 数量、连接客户端数量和最近错误。`--json` 输出一个对象。 |
| `install` | 把当前可执行文件复制到用户目录并加入 PATH。`--start` 在安装后启动，`--restart` 让新版本接管。 |
| `uninstall` | 停止 Server，删除安装的 `condr` 和 PATH 项，保留配置、数据和日志。 |
| `invite` | 打印一次性配对链接，有效期 10 分钟。先打开 TCP 监听或 Peer-to-peer。 |
| `clients` | 列出已配对 Device：名称、最近连接时间和指纹。 |
| `revoke <指纹>` | 撤销 Device 并断开连接。也可使用指纹的唯一前缀，区分大小写。 |
| `bridge` | 把 stdin/stdout 连接到本机 Server socket。SSH 连接会在远端运行它，你无需手动调用。 |

`status --json` 的字段：`running`、`endpoint`、`fingerprint`、`version`、`protocol`、`listen`、`p2p`、`connected_devices`、`uptime_secs`、`workspaces`、`tabs`、`panes`、`agents`、`clients`、`recent_errors`。

## 用 `condr workspace` 管理 Workspace

| 命令 | 作用 |
| --- | --- |
| `list` | 列出所有 Workspace。`--all-devices` 加上所有保存的 Device，每个条目带 `device` 字段，无法连接的列在 `unreachable` 下。 |
| `create` | 打开 Workspace。`--cwd` 默认当前目录，`--label` 默认目录名，`--focus` 让所有窗口显示它。 |
| `get <id>` | 显示一个 Workspace。worktree 会带 `worktree` 对象。 |
| `focus <id>` | 让所有已连接窗口显示它。 |
| `rename <id> <名字>` | 重命名。 |
| `close <id>` | 关闭 Workspace 并停止所有终端，不删除文件。 |

## 用 `condr tab` 管理 Tab

| 命令 | 作用 |
| --- | --- |
| `list` | 列出 Tab。`--workspace <id>` 限定一个 Workspace。 |
| `create` | 新建 Tab，默认在当前 Pane 所在 Workspace 中，shell 从当前 Pane 的目录启动。`--label` 命名，`--focus` 切换过去。 |
| `get <id>` | 显示一个 Tab。 |
| `focus <id>` | 让所有窗口显示它。 |
| `rename <id> <名字>` | 重命名。 |
| `close <id>` | 关闭 Tab 并停止终端。关闭最后一个 Tab 时 Workspace 也关闭。 |

## 用 `condr pane` 管理 Pane

省略 `<id>` 的命令默认作用于当前 Pane。

| 命令 | 作用 |
| --- | --- |
| `list` | 列出 Pane 及 Agent 状态和 `blocked_on`。`--workspace <id>` 限定范围。 |
| `get <id>` | 显示一个 Pane。 |
| `current` | 显示当前 Pane。 |
| `layout [id]` | 描述 Pane 所在 Tab：分割树、每个 Pane 的边界和邻居。 |
| `split --direction right\|down [id]` | 分割，新 shell 在同一目录。`--focus` 让新 Pane 获得焦点。 |
| `focus [id]` | 聚焦 Pane 并让所有窗口显示它的 Tab。`--direction` 改为聚焦该方向的邻居。 |
| `resize --direction <方向> [id]` | 移动一条边。`--amount` 是分割比例，默认 0.05。 |
| `swap --direction <方向> [id]` | 与邻居交换。 |
| `move --to <id> --side <方向> [id]` | 在同一 Tab 中拆下 Pane，接到另一个 Pane 的一侧。 |
| `zoom [id]` | 放大到整个 Tab 或还原。`--on` 和 `--off` 设置方向。 |
| `close <id>` | 关闭 Pane 并停止终端。关闭最后一个 Pane 时 Tab 也关闭。 |
| `read <id>` | 打印最后几行文本，包含滚动历史。`--lines` 默认 80。 |
| `send-text <id> <文本>` | 原样输入文本，不回车。 |
| `send-keys <id> <键>...` | 按键：`enter`、`esc`、`tab`、`up`、`f5`、`ctrl+c`、`alt+shift+x`、`a`。 |
| `run <id> <命令>` | 粘贴命令并回车。 |

## 用 `condr agent` 管理 Agent

Agent 目标可以是 `start` 指定的名称或 Pane 编号。名称必须匹配 `[a-z][a-z0-9_-]{0,31}`，并持续到 Agent 退出或 Server 重启。

| 命令 | 作用 |
| --- | --- |
| `available` | 列出 Server PATH 上找到的 Agent。 |
| `list` | 列出已识别 Agent：名称、Pane、状态、`blocked_on` 和原生会话号。 |
| `start <名字> --kind <种类> --pane <id> [-- 参数]` | 在空闲 shell 中启动 Agent 并等待就绪。`--timeout` 单位为毫秒，默认 30000。 |
| `prompt <目标> <文本>` | 发送提示。`--wait` 等到 Idle 或 Blocked，`--until` 修改等待状态，`--timeout` 单位为毫秒。 |
| `wait <目标>` | 等待状态，不发送输入。默认等待 Idle 或 Blocked，`--timeout` 默认 120000。 |
| `hooks install\|uninstall\|status <种类>` | 安装、移除或检查 hook，只作用于本机。 |

`--kind` 的取值：`claude`、`codex`、`opencode`、`pi`、`omp`、`antigravity`、`grok`、`cursor`、`copilot`、`kimi`。

## 用 `condr device` 列出 Device

| 命令 | 作用 |
| --- | --- |
| `list` | 列出保存的远程 Device：名称、地址、当前是否能响应及 Workspace 数。 |
