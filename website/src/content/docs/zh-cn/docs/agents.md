---
title: Agent
description: 查看支持的 Agent，安装 hook，并读懂每种状态。
---

安装 Agent 的 hook，让侧栏显示状态和等待内容。编排命令见 [Agent 驱动 Condr](/zh-cn/docs/automation/)，命令选项见 [CLI 参考](/zh-cn/docs/cli/)。

## 准备 Agent

Condr 不提供模型或账号。先在运行 Server 的机器上安装 Agent CLI 并登录。

确认 Server 能找到哪些 Agent：

```sh
condr agent available
```

命令列出 Server 的 PATH 上找到的 Agent。启动 Server 的程序决定这个 PATH，Server 不会再次读取 shell 配置。因此，只写在 `.zshrc` 中的别名或路径对 Server 不可见。

## 查看支持的 Agent

| Agent | 命令 | 报告状态 | 说明等待内容 | `agent start` 返回时机 |
| --- | --- | --- | --- | --- |
| Claude Code | `claude` | 是 | 权限和问题 | 报告 Idle 后 |
| Codex | `codex` | 是 | 权限 | 认出进程后 |
| OpenCode | `opencode` | 是 | 权限和问题 | 报告 Idle 后 |
| Pi | `pi` | 是 | 问题 | 报告 Idle 后 |
| Oh My Pi | `omp` | 是 | 问题 | 报告 Idle 后 |
| Antigravity | `agy` | 是 | 否 | 认出进程后 |
| Grok Build | `grok` | 是 | 权限 | 报告 Idle 后 |
| Cursor CLI | `cursor-agent` | 是 | 否 | 认出进程后 |
| GitHub Copilot | `copilot` | 是 | 权限 | 认出进程后 |
| Kimi Code | `kimi` | 否 | 否 | 认出进程后 |

标为「认出进程后」的 Agent 在第一次提示前不报告状态，启动后会显示 Unknown。这是它们 hook 的行为，不是 Condr 的问题。

Condr 按进程名识别 Agent。通过 node、bun、python 或 shell 启动的 Agent 也能识别，符号链接会跟到真实目标。包装器隐藏名称时，例如 `mise exec -- claude`，Agent 自己的 hook 事件会报告身份。

## 安装 hook

hook 是 Agent 在开始、完成或等待时调用的一小段命令。Condr 只信这些报告。不装 hook，Agent 会一直是 Unknown。

在运行 Server 的机器上，为每个 Agent 安装一次：

```sh
condr agent hooks install claude
```

也可以打开 **Settings › Device › Agent integrations**，对选中的 Device 点 **Install**。远程 Device 也在那里安装，因为命令行的 `agent hooks` 不接受 `--device`。

命令只修改 Agent 自己的配置文件，并把 Condr 条目合并到已有条目旁边。文件损坏或超过 4 MiB 时，命令报错且不写入。

| Agent | 修改的文件 |
| --- | --- |
| Claude Code | `~/.claude/settings.json` |
| Codex | `~/.codex/hooks.json`，并运行 `codex features enable hooks` |
| OpenCode | `~/.config/opencode/condr-tui.js`，并加入 `tui.json` 的 `plugin` 列表 |
| Pi | `~/.pi/agent/extensions/condr-pi.ts` |
| Oh My Pi | `~/.omp/agent/extensions/condr-omp.ts` |
| Antigravity | `~/.gemini/antigravity-cli/plugins/condr/` |
| Grok Build | `~/.grok/hooks/condr.json` |
| Cursor CLI | `~/.cursor/hooks.json` |
| GitHub Copilot | `~/.copilot/hooks/condr.json` |
| Kimi Code | 不支持，见下文 |

Agent 自己的环境变量仍然生效。例如，`CLAUDE_CONFIG_DIR` 和 `CODEX_HOME` 会改变文件位置。

用 `status` 查看，用 `uninstall` 移除：

```sh
condr agent hooks status claude
condr agent hooks uninstall claude
```

hook 有四种状态：`installed`、`outdated`（升级 Condr 后重装）、`missing` 和 `unsupported`。

几个 Agent 需要额外一步：

- **Codex**：安装后在 Codex 内运行 `/hooks`，信任新 hook。
- **Antigravity**：在其设置中打开 Condr 插件。第一次调用工具前一直是 Unknown。
- **Pi** 需要 0.85.1 或更新版本。**Oh My Pi** 需要 18.1.17 或更新版本，每个 profile 安装一次。

hook 只在 Condr Pane 中生效。在其他终端运行 Agent 时，hook 不会工作。

## 读懂每种状态

| 状态 | 含义 | 侧栏标记 |
| --- | --- | --- |
| Unknown | 普通 shell，或 Agent 还没有报告 | 灰色信息图标 |
| Idle | 这一轮完成，等待下一条提示 | 空心圆 |
| Working | 正在工作 | 琥珀色 |
| Blocked | 需要你处理 | 红色 |
| Done | 你离开时已经完成 | 绿色 |

Done 只存在于窗口中。Agent 从 Working 或 Blocked 变为 Idle，且其 Pane 没有焦点时，侧栏会显示 Done，直到你点进该 Pane。

Workspace 行会汇总 Agent 状态，按响铃、Blocked、Done、Working 的顺序各显示一个计数。Unknown 和 Idle 不计数。

## 查看 Agent 在等什么

Agent 等待权限或回答时会变成 Blocked。能说明原因的 Agent 会在侧栏 Agent 行第二行显示工具名和命令第一行，例如：

```text
Bash: cargo test
```

这一行也可能显示问题文本，长度截断为 200 个字符。

你可以在三个位置看到：

- 侧栏 Agent 行的第二行。
- 系统通知正文。
- 侧栏顶部的 **Needs you** 列表。它汇总所有已连接 Device 上的 Blocked Agent，没有时消失。点击一行可跳到对应 Pane。

Cursor CLI 和 Antigravity 没有权限事件，因此等待时不会变成 Blocked。

## 接收通知

Pane 没有焦点时，以下两种变化会发送系统通知：

- Agent 完成：「Claude finished」。
- Agent 需要你：「Claude needs your input」。

通知正文包含等待内容、Workspace 名称和终端标题。点击通知会切换到该 Pane。Blocked 期间等待内容变化时，Condr 会替换通知，而不会叠加。

要关闭通知，打开 **Settings › Notifications** 并关闭 **Enable notifications**。这里还有发送测试通知的按钮。

## 了解 Agent 检测方式

Server 读取进程表，识别 Pane 中运行的 Agent。已安装的 hook 为每个事件运行 `condr agent-hook`，把事件写入 Pane 的终端。Server 在终端解析前截取事件，所以状态直接从 Agent 到 Server，不经过屏幕文字。

嵌套 Agent 不计入。一个 Agent 以子进程启动另一个 Agent 时，Condr 只跟踪外层 Agent。

## 了解限制

- **Kimi Code** 能被识别，但 hook 无法区分主任务和子任务的结束。Condr 不安装 hook，所以状态保持 Unknown。
- **OpenCode** 使用 TUI 插件，仍缺少真机测试。安装前删除旧的开发版 `plugins/condr.js`。
- **Grok Build** 在其他 Stop hook 阻塞时可能提前显示完成。
- **GitHub Copilot** 在 API 出错后可能保持 Working。
- Condr 无法识别的程序是普通终端，状态为 Unknown。这与识别到但未安装 hook 的 Agent 看起来一样。运行 `condr agent list` 区分它们，无法识别的程序不在列表中。
