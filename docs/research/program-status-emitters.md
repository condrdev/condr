# 哪些 Agent CLI 会发 OSC 7501

调查日期：2026-10-19。依据公开 changelog、PR、issue，以及对本机 Claude Code 2.1.303、Codex 0.163.0 二进制和下载的 Grok Build 1.0.62、Antigravity CLI 1.4.2、cursor-agent 2026.10.16 做 `strings` 核对。规范是 [Program Status Protocol](https://www.superlogical.com/rex/docs/build/program-status) rev 0.4（2026-10-10）；它建议的"无回复哨兵"是紧跟探测发 `CSI c`，DA1 先回来就当不支持。Condr 的处理见 ADR 0044。

| CLI | 发不发 / 起始版本 | 是否先探测 `OSC 7501;?` | state / kind / 其他键 | 子记录 `id=` | hooks 是否仍触发 |
| --- | --- | --- | --- | --- | --- |
| Claude Code | 发，2.1.295（2026-10-08） | 探测，与 XTVERSION、`CSI ?u` 同批，DA1 为哨兵，2 s 超时；`CLAUDE_CODE_DISABLE_TERMINAL_TITLE`、`CLAUDE_CODE_SESSION_KIND=bg` 时关闭；无强制开启变量 | idle/working/done/blocked/error + clear；`app=claude-code`；kind permission/question/auth；progress、title、msg | 有：每个 subagent / 后台任务一条，`id=<task id>`，最多 32 条，消失时 `state=clear:id=` | 是（#103022 实测 working 比 UserPromptSubmit 晚 6 ms，done 比 Stop 晚 25 ms） |
| Codex CLI | 发，0.163.0（2026-10-15，PR #52725） | 不探测，stdout 是 tty 就发，tmux 内也发；无开关 | 0.163：idle/working/blocked + clear，`app=codex`，无 kind；0.164.0-alpha.1（PR #54114）起加 done/error 与 kind permission/question/auth；done/error 保留到用户按键或鼠标点击；无 msg/title/progress | 无 | 是（两 PR 只动 `codex-rs/tui/`） |
| Pi | 发，1.1.0（2026-10-07，#10607） | 探测，与 kitty 键盘查询同发，DA1 之前收到回复才上报；`PI_PROGRAM_STATUS=1|0` 强制 | idle/working/blocked/done/error + clear；`app=pi`；kind permission/question/auth；msg（会话名 / 对话框标题 / 错误首行）；无 id/title/progress | 无 | 是 |
| Oh My Pi | 发，18.8.7（2026-10-09，#15026） | 不探测，`stdout.isTTY` 且非 headless 就发；设置 `terminal.programStatus` 可关 | idle/working/blocked/done/error + clear；`app=omp`；kind 只有 permission/question；msg（审批提示 / 问题 / 错误）；启动时不发 idle | 无 | 是 |
| OpenCode | **不发**（至 v1.19.3）。#55172 于 2026-10-13 合入，#55300 次日回退 | — | 计划：按会话分记录 working / blocked:kind=permission|question，根记录汇总；无 done/error | 计划有 | — |
| Grok Build | 发，1.0.52（2026-10-11） | 探测，仅在它本来就发版本查询的终端（tmux 内不发）；`[ui.notifications] program_status = auto|on|off`；也认 `REX_SESSION`/`REX_BLOCK` | working/blocked/done/error + clear；`app=grok-build`；二进制里有 `kind=permission` 和 `title=`；`state=clear:id=` 存在 | 有层级 id，具体哪些记录未确认 | 未确认 |
| Cursor CLI | **不发**（2026.10.16） | — | — | — | — |
| GitHub Copilot CLI | **不发**（至 v1.0.100；#5112 开着） | — | — | — | — |
| Antigravity CLI (`agy`) | 发，1.4.0（2026-10-16） | 探测（Bubble Tea 的 `OSC 7501;? BEL`）；`AGY_CLI_OSC7501=on|off`，`AGY_CLI_OSC7501_MESSAGES=brief` | working / waiting / done / failed；编码器支持全部键，实际发哪些及 `app=` 值未确认（闭源，二进制里有 `antigravity` 字面量） | 链接了 `ClearProgramStatusID`，用法未确认 | 未确认 |
| Kimi CLI | **不发**（1.52.0） | — | — | — | — |

Gemini CLI 也不发（gemini-cli#29710 开着）。

## 对 Condr 的影响

- `app` 的值都能经 `AgentKind::parse_label` 解析：`claude-code` 是 Claude 的 alias，`codex`、`pi`、`omp` 是 id，`grok-build` 是 Grok 的 alias。Antigravity 若发 `app=antigravity`，同样命中其 id；发别的就要加 alias。
- Claude Code 的 `done` 在一轮结束时发出，`idle` 只在中断（Esc）或启动时发，所以 Done 会一直挂到下一轮；Codex 0.164 的 done/error 则在用户按键后变回 idle。Condr 的 Client 本地"看过"位负责把 Done 显示成 Idle。
- Codex 0.163 一轮结束发的是 `idle` 而非 `done`，Condr 会显示 Idle；GUI 对"Working 之后紧接 Idle"仍发"finished"通知。升到 0.164 后自然变成 Done。
- Claude Code 的 `blocked` 文本（`msg`）形如 `approve <tool>: <description>`、`MCP input: <server>`；`dialog open` 这种没有 kind。认证失败报 `blocked:kind=auth`。
- Windows Server 上 conhost 自己回答 DA1，Claude Code 和 Pi 的探测会判定不支持，仍由 hooks 报状态；Codex、OMP 不受影响。
- 协议不带会话 ID，resume 仍只能靠 hooks 的 SessionStart。

来源：[Claude Code CHANGELOG](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)、[claude-code#101114](https://github.com/anthropics/claude-code/issues/101114)、[claude-code#103022](https://github.com/anthropics/claude-code/issues/103022)、[codex#52725](https://github.com/openai/codex/pull/52725)、[codex#54114](https://github.com/openai/codex/pull/54114)、[pi-mono#10607](https://github.com/badlogic/pi-mono/issues/10607)、[Pi terminal setup](https://pi.dev/docs/latest/terminal-setup#program-status)、[oh-my-pi#15026](https://github.com/can1357/oh-my-pi/issues/15026)、[opencode#55172](https://github.com/anomalyco/opencode/pull/55172)、[opencode#55300](https://github.com/anomalyco/opencode/pull/55300)、[x.ai changelog](https://x.ai/build/changelog)、[antigravity-cli releases](https://github.com/google-antigravity/antigravity-cli/releases)、[copilot-cli#5112](https://github.com/github/copilot-cli/issues/5112)。
