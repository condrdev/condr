---
title: Agent 驱动 Condr
description: 让一个 Agent 开 Pane、读输出、分派工作并等待结果。
---

用下面的命令让一个 Agent 指挥其他 Agent，或用脚本控制 Condr。参数和 JSON 字段见 [CLI 参考](/zh-cn/docs/cli/)。

## 了解自动化如何工作

每个 Pane 的 shell 都有 `condr` 命令和这些环境变量：

| 变量 | 含义 |
| --- | --- |
| `CONDR_ENV=1` | 这个 shell 在 Condr 的 Pane 中运行 |
| `CONDR_PANE_ID` | 这个 Pane 的编号 |
| `CONDR_SOCKET_PATH` | 连接 Server 的路径 |
| `CONDR_BIN_PATH` | PATH 被重置时使用的 `condr` 绝对路径 |

在 Pane 中运行的 `condr` 命令默认作用于该 Pane 所在的 Workspace 和 Tab。例如，`condr pane split --direction right` 不用编号就能分割当前 Pane。

除 `pane read` 输出文本外，每条命令都输出 JSON。失败时，命令把 `{"error":{"code","message"}}` 写到 stderr，并以 1 退出。Agent 能读取这些输出，脚本也能处理。

Pane 中的 Agent 可以用这些命令开 Pane、启动 Agent、发送提示、等待完成并读取输出。你能在侧栏查看整个过程。

## 让 Agent 使用 Condr Skill

Condr 内置一份说明所有命令及注意事项的 Skill。打印它：

```sh
condr --skill
```

保存输出，并放到 Agent 读取 Skill 或自定义指令的位置。Claude Code 使用项目中的 `.claude/skills/condr/SKILL.md`。每次升级 Condr 后重新生成，因为 Skill 会随版本变化。

Agent 在 Pane 中运行时，`CONDR_ENV=1` 会告诉它可以使用 `condr` 命令。Agent 的命令沙箱必须允许访问 `CONDR_SOCKET_PATH` 指定的 socket，否则命令无法连接 Server。

## 执行常见操作

**腾出空间。** `pane split` 在当前 Pane 旁边打开 shell，`tab create` 打开 Tab，`workspace create` 打开另一个项目。三条命令都会返回新对象的编号，供后续命令使用。

**启动 Agent。** `agent start` 在空闲 shell 中启动 Agent，为它命名并等待就绪：

```sh
condr agent start reviewer --kind claude --pane 12
```

后续命令使用这个名字代替 Pane 编号。Agent 自己的参数放在 `--` 后面。

**发送提示。** `agent prompt` 向空闲 Agent 发送文本。加上 `--wait` 后，命令会等它进入 Idle 或 Blocked 才返回：

```sh
condr agent prompt reviewer "审查 src/auth.rs 的改动，列出问题" --wait
```

**只等待。** `agent wait` 等 Agent 进入某种状态。它返回状态，不返回输出。

**读取输出。** `pane read` 打印终端最后几十行，包含滚动历史。等待完成后用它读取结果。

**直接操作终端。** `pane run` 粘贴命令并回车，`pane send-text` 输入但不回车，`pane send-keys` 按下 `enter`、`esc` 或 `ctrl+c` 等按键。

Agent 的状态来自 hook。没有 hook 时，Agent 会一直是 Unknown，`agent start` 会等到超时，`--wait` 也永不返回。先按 [Agent](/zh-cn/docs/agents/) 页安装 hook。Codex 等在第一次提示前不报告状态的 Agent，会在 Condr 认出进程后让 `agent start` 返回。第一次 `agent prompt` 在 Unknown 状态也会接受。

Agent 等待批准时，`agent wait` 返回 `blocked`，`blocked_on` 会说明它在等待什么。让编排 Agent 把这个信息转告你，不要替你批准。

## 可直接粘贴的提示

把下面的提示发给主 Agent。运行前替换任务和路径。

**并行调查后汇总：**

```text
在这个 Workspace 里用 condr 开三个 Pane，各启动一个 claude。
一个追踪请求的处理路径，一个检查测试覆盖，一个找相关的历史回归。
都不要改文件。三个都完成后，读它们的输出，合并成一份报告给我。
```

**一个写、一个审：**

```text
用 condr 在旁边开一个 Pane 启动 codex，名字叫 reviewer。
我每让你完成一个改动，就把 diff 发给 reviewer 审查，等它完成后把意见汇总给我。
```

**把活派到另一台机器：**

```text
用 condr --device build-box 在那台机器的 ~/code/app 里开 Workspace，
启动 claude，让它跑完整测试套件并修复失败。等它完成后把结果读回来。
```

## 跨 Device 执行命令

Condr 保存的远程 Device 也能从命令行使用：

```sh
condr device list
condr --device build-box workspace list
condr workspace list --all-devices
```

把 `--device` 放在命令组前，后面写侧栏显示的 Device 名。用 `CONDR_DEVICE` 设置默认值。每次调用都会单独建连，所以结果中的编号属于那台 Device，后续命令要带同一个 `--device`。那台 Device 上不会使用 `CONDR_PANE_ID` 等默认值，请给出明确目标。

`server` 和 `agent hooks` 只作用于本机，不接受 `--device`。
