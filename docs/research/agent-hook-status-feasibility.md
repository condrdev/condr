# Agent Hook 状态检测可行性

日期: 2026-09-06(同日补充相似项目调研)。本文是可行性调研,不是 ADR 或已批准的实施方案。
官方在线文档核对于上述日期;未实测各 CLI。本地 `../herdr` 只读,未执行 pull。

## 结论

**建议:** 可以自行实现 hook 集成,并让官方事件成为状态的主要来源。但“所有支持的原生交互式 CLI,只靠 hooks,完整且即时区分 idle/working/blocked”目前不能承诺。
不同 agent 的事件完整性差异很大;收窄支持范围、允许 `unknown`、接受部分状态延迟,才能删除屏幕检测而不伪造确定性。
进程存活检查能处理崩溃和退出,不能补齐运行中取消、审批通过或其他 hook 要求继续执行时的语义缺口。

## Condr 当前链路

本次读取 Condr commit: `c12b35880064a2926938d717d35fc388ebbf9b4c`。当前识别 23 种 agent,其中 21 种内嵌屏幕 manifest;`agent/mod.rs` 和 `agent/manifest.rs` 明确记录移植自 Herdr,不只是复制了 TOML。[Agent 模块](../../crates/condr-core/src/agent/mod.rs), [Manifest 引擎](../../crates/condr-core/src/agent/manifest.rs)

链路为进程探测识别身份,`TerminalAgentProbe::poll` 读取活动屏底部文本和 OSC,`AgentDetector` 分类并迟滞,Server 的 `apply_agent_refresh` 发布 `AgentChanged`。结果同时驱动 GUI 与 `agent start/prompt/wait`;状态误报会影响命令就绪判断。GUI 的 `Done` 是未查看完成状态的呈现,不是上游 hook 名称。[Probe](../../crates/condr-core/src/terminal/runtime.rs), [发布](../../crates/condr-server/src/server/terminal_monitor.rs), [编排](../../crates/condr-server/src/server/agents.rs)

已有 `PaneEnvironment` 将 `CONDR_PANE_ID`、`CONDR_SOCKET_PATH`、`CONDR_BIN_PATH` 传入 shell。候选接入只需新增一个读取 hook JSON 的 `condr` 子命令,经现有版本化协议上报本机 Server,复用状态发布与重连快照。远程 GUI 仍由 Server 转发状态。当前 agent 是在 Pane 的 shell 中启动,agent 退出时 shell/PTY 可以继续存活,所以不能仅用 PTY EOF 替代 agent 存活检查。[Pane 环境](../../crates/condr-core/src/terminal/runtime.rs), [启动与存活检查](../../crates/condr-core/src/terminal/launch.rs), [协议](../../crates/condr-core/src/protocol.rs)

## 官方能力

下表中的状态映射是 Condr 的候选解释,不是这些产品保证提供的统一状态协议。

| Agent | 工作与空闲事件 | 等待用户事件 | 关键边界 |
| --- | --- | --- | --- |
| Claude Code | `UserPromptSubmit`, `Stop`, `StopFailure` | `PermissionRequest`, `Notification`, `Elicitation` | 用户中断不触发 `Stop`;一般工具审批没有配对的完成事件 |
| Gemini CLI | `BeforeAgent`, `AfterAgent` | `Notification` 的 `ToolPermission` | 没有通用中断/审批完成 hook;`AfterAgent` 可要求重试 |
| OpenCode | plugin `session.status`, `session.idle`, `session.error` | `permission.asked/replied`, `question.asked/replied/rejected` | 事件更完整,仍需绑定当前根 session、处理并发请求和退出 |
| Codex | `UserPromptSubmit`, `Stop`, `Interrupt` | `PermissionRequest`,部分工具前后事件 | 无通用审批完成 hook;`Stop` 可要求继续;hook 必须受信任 |

### Claude Code

**事实:** 官方提供 `SessionStart/SessionEnd`、提交/正常停止/API 失败事件,工具调用前后事件以及权限请求。MCP 用户输入有 `Elicitation/ElicitationResult` 配对。`Stop` 明确不覆盖用户中断;取消正在执行的工具也不保证 `PostToolUseFailure`。因此不能把该事件的可选 `is_interrupt` 字段当成通用取消通知。[Hooks reference](https://code.claude.com/docs/en/hooks)

**推断:** 普通 `PermissionRequest` 在决定前触发,其他 hook 可以直接允许;它不一定代表用户正在等待。批准一个耗时工具后,`PostToolUse` 要到工具完成才到达,不能提供即时 `blocked -> working`。`ElicitationResult` 仅解决 MCP 输入配对,不是普通权限决议事件。[PermissionRequest](https://code.claude.com/docs/en/hooks#permissionrequest), [PostToolUse](https://code.claude.com/docs/en/hooks#posttooluse)

**事实:** `Stop` 自身允许其他 hook 阻止停止并要求继续;`SessionStart` 的来源包括 compact,不应无条件映射成 idle。`SessionEnd` 有超时,不能承担进程退出的唯一检测。用户、项目、插件和受管配置可提供或限制 hooks。[Stop](https://code.claude.com/docs/en/hooks#stop), [Hook locations](https://code.claude.com/docs/en/hooks#hook-locations)

**接入:** 原生 `claude` 可用 `--settings` 加载本次启动的 JSON,或加载含 `hooks/hooks.json` 的插件;不需要接管对话循环。应保留现有设置和 hooks,并检测集成是否实际生效。[CLI reference](https://code.claude.com/docs/en/cli-reference), [Hooks reference](https://code.claude.com/docs/en/hooks#hook-locations)

### Gemini CLI

**事实:** 官方定义 `BeforeAgent/AfterAgent`、`BeforeTool/AfterTool`、模型事件、`SessionStart/SessionEnd`、`Notification`、`PreCompress`。`Notification` 当前列出的类别为 `ToolPermission`;事件枚举未提供通用 `Interrupt`、`PermissionResolved` 或独立回合失败事件。[Hooks reference](https://geminicli.com/docs/hooks/reference/), [官方 hook 类型](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/hooks/types.ts)

**事实:** `AfterAgent` 在最终响应后触发,其他 hook 可拒绝响应并强制重试;文档没有承诺它覆盖所有取消/异常路径。`SessionEnd` 是 best effort,CLI 不等待其完成。`AfterTool` 是工具执行后的通知。[Agent hooks](https://geminicli.com/docs/hooks/reference/#agent-hooks), [Lifecycle hooks](https://geminicli.com/docs/hooks/reference/#lifecycle--system-hooks)

**推断:** 可以覆盖正常回合开始/结束和权限提示,但不能仅凭已列出的 hook 保证取消收尾及审批后的即时恢复。要声称完整支持,需要针对锁定的 CLI 版本检查实际调用位置并运行交互式测试。

**接入:** 在 `.gemini/settings.json`、`~/.gemini/settings.json` 或 extension 中配置 command hooks,接收 stdin JSON,返回合法 JSON。沿用原生 CLI,无需另造聊天界面;配置合并、hook 禁用和项目信任会影响事件可用性。[Hooks 配置](https://geminicli.com/docs/hooks/), [Hook schema](https://geminicli.com/docs/hooks/reference/#configuration-schema)

### OpenCode

**事实:** 本地 JS/TS plugin 可以订阅 `session.status`、`session.idle`、`session.error`、`permission.asked/replied`。官方 question 实现还发布 `question.asked/replied/rejected`,并提供未完成请求列表;状态服务提供当前 session 状态查询。[Plugins](https://opencode.ai/docs/plugins/), [Question 源码](https://github.com/anomalyco/opencode/blob/dev/packages/opencode/src/question/index.ts), [Session status 源码](https://github.com/anomalyco/opencode/blob/dev/packages/opencode/src/session/status.ts)

**建议:** 它更适合以官方事件作为权威来源。用当前根 session 的状态加未完成请求集合计算 blocked;处理一个回复时,不能忽略另一个仍未回答的请求。`session.created/deleted` 描述会话数据生命周期,不能等同于 CLI 进程启动/退出;异常、取消、重连后的恢复仍应验收,不能仅看事件名字保证正确。

**接入:** 原生 CLI 启动时自动加载 `.opencode/plugins/` 或 `~/.config/opencode/plugins/` 中的文件;配置还支持 `OPENCODE_CONFIG` 与 `OPENCODE_CONFIG_CONTENT`。可用本地 plugin 桥接 Condr,不必发布 npm 包。在线源码链接指向移动的 `dev`,实施时应锁定版本。[Plugins](https://opencode.ai/docs/plugins/), [Config](https://opencode.ai/docs/config/)

### Codex

**事实:** 当前官方文档提供 `SessionStart/SessionEnd`、`UserPromptSubmit`、`PreToolUse/PostToolUse`、`PermissionRequest`、`Stop`、`Interrupt` 等。`Interrupt` 覆盖主线程活动回合中断。`PermissionRequest` 在审批前触发,其他 hook 可直接批准而不显示提示;`PostToolUse` 在工具产出结果后触发。事件列表没有通用审批完成事件。[OpenAI 官方 Hooks 文档](https://learn.chatgpt.com/docs/hooks)

**推断:** 可覆盖正常提交、中断和部分等待,但不能用工具完成事件即时判断长任务审批后的恢复。`Stop` 可被其他 hook 要求继续,`SessionStart` 的 compact 来源可发生于回合中,都不能无条件标成 idle。工具 hooks 也不覆盖全部工具路径,用户提问/权限等待需按具体工具核对。

**接入:** 使用活动配置层的 `hooks.json`、`config.toml` 内联 hooks 或 plugin;非受管 hook 定义必须先被用户信任,配置成功不等于生效。Condr 的 handler 应只报告状态,不输出审批决策或改变对话。本机仅核对到 `codex-cli 0.153.4`,未验证该二进制包含在线文档的全部事件;实施前需锁定最低版本并实测。[配置与信任](https://learn.chatgpt.com/docs/hooks#review-and-trust-hooks)

## Herdr 当前参考实现

本地参考 commit: `5158adab10b6dcfea9370782043392f80fa0643c`,由 `git -C ../herdr rev-parse HEAD` 取得。该 commit 与 Condr 标注的移植来源一致;此处额外核对其 hook 集成与状态仲裁。

**事实:** 当前 Herdr 也没有对全部 agent 使用纯 hooks。

- `full_lifecycle_hook_authority` 只列出 pi、omp、mastracode、opencode、kilo、kimi;这些来源存活时覆盖屏幕状态。[detect/mod.rs](https://github.com/herdrdev/herdr/blob/5158adab10b6dcfea9370782043392f80fa0643c/src/detect/mod.rs#L316)
- 有效状态仲裁集中在 `terminal/state.rs`,包含 hook 来源/序号/session 绑定,进程退出时清理权威来源,其余来源保留屏幕兜底和可见 blocker 仲裁。[terminal/state.rs](https://github.com/herdrdev/herdr/blob/5158adab10b6dcfea9370782043392f80fa0643c/src/terminal/state.rs#L5)
- 当前 Claude 安装器清理旧的状态 hooks,只注册 `SessionStart` 上报会话标识;脚本忽略其他事件及子 agent。[claude_settings.rs](https://github.com/herdrdev/herdr/blob/5158adab10b6dcfea9370782043392f80fa0643c/src/integration/claude_settings.rs#L22), [Claude 集成](https://github.com/herdrdev/herdr/blob/5158adab10b6dcfea9370782043392f80fa0643c/src/integration/assets/claude/herdr-agent-state.sh)
- OpenCode 集成根据 session、permission、question 事件上报状态,过滤子 session 对根身份的覆盖。[OpenCode 集成](https://github.com/herdrdev/herdr/blob/5158adab10b6dcfea9370782043392f80fa0643c/src/integration/assets/opencode/herdr-agent-state.js)

**许可核对范围:** 阅读了上述 Rust 文件和两个集成资产的文件头及许可证标记,未发现独立许可声明;仓库 `Cargo.toml` 声明 Apache-2.0,根 `LICENSE` 为 Apache-2.0。仓库内其他 vendor/packaging 目录有各自许可证,不能将顶层许可泛化到它们。本文仅总结行为,没有移植上述源码;未来若移植须重新核对具体文件和 notice。[Cargo.toml](https://github.com/herdrdev/herdr/blob/5158adab10b6dcfea9370782043392f80fa0643c/Cargo.toml), [LICENSE](https://github.com/herdrdev/herdr/blob/5158adab10b6dcfea9370782043392f80fa0643c/LICENSE)

## 相似项目方案

2026-09-06 补充。以下仓库均为浅克隆到本机核对,commit 为核对时的 HEAD;未运行任何一个。

| 项目 | 状态来源 | Hook 到宿主的传输 | 无 hook 时 | 许可证 |
| --- | --- | --- | --- | --- |
| [tty7](https://github.com/l0ng-ai/tty7) `e98586b` | 纯 hooks,无屏幕规则 | hook 子命令把 OSC 777 写回控制 tty,Server 的 VT 解析器就地消费 | 显式 `no-agent`;`free` 看前台进程是否退出 | Apache-2.0 |
| [herdr](https://github.com/herdrdev/herdr) `5158ada` | 屏幕 manifest 为主,6 种 agent 的 hook 为权威 | CLI + socket | 屏幕兜底 | Apache-2.0 |
| [agent-deck](https://github.com/asheshgoplani/agent-deck) `5235e16` | hooks 状态文件 + tmux `capture-pane` 正则,双源仲裁 | hook 写 `<id>.json`,文件 watcher | 屏幕兜底 | MIT |
| [tmux-agent-status](https://github.com/samleeney/tmux-agent-status) `a323f10` | 纯 hooks(Claude/Codex/Devin) | 状态文件 `~/.cache/…` | 不支持,自定义 agent 自己写状态文件 | MIT |
| [claude-squad](https://github.com/smtg-ai/claude-squad) `ce1ffb4` | 纯屏幕:内容 diff + 固定提示字符串 | 无 | — | AGPL-3.0,禁止参考代码 |
| [paseo](https://github.com/getpaseo/paseo) `ecec332` | 不可比:自己拥有对话循环(Claude Agent SDK、Codex app-server、ACP、OpenCode server),状态来自结构化流 | 进程内 | — | 见本地 checkout |

### tty7

与 Condr 最接近:GPUI + `alacritty_terminal` + 常驻 server/client、Apache-2.0、19 种 agent 进程识别,其中 12 种有 hook。它证明了“纯 hooks、不做屏幕检测”的产品可以成立,但也明确接受了状态缺口。[README](https://github.com/l0ng-ai/tty7/blob/e98586bdbd6d0adeffb86cef0028c6d6df3a0994/README.md), [Status 文档](https://github.com/l0ng-ai/tty7/blob/e98586bdbd6d0adeffb86cef0028c6d6df3a0994/docs/agents/status.mdx)

**事实,传输:** 安装的 hook 命令是 `tty7 agent-hook <agent> <event>`。它读 stdin JSON(上限 64 KiB),抽取 `session_id/message/cwd/prompt`,拼成 `ESC ] 777 ; notify ; tty7://cli-agent ; {json} BEL`,直接写到控制终端:Unix 先 `/dev/tty`,失败则用 `ps` 沿父进程链找 tty 设备;Windows 用 `AttachConsole(祖先 pid)` 后写 `CONOUT$`。Server 端 PTY 读线程的 OSC tokenizer 在 fan-out 前识别该前缀并转成 `AgentEvent`。没有 socket,没有 pane id 环境变量,事件天然绑定到写入的那个 PTY,SSH 远程 pane 也同样生效;hook 用 `TTY7` 环境变量判定自己不在 tty7 内时直接退出。[agent_hooks.rs](https://github.com/l0ng-ai/tty7/blob/e98586bdbd6d0adeffb86cef0028c6d6df3a0994/crates/tty7-core/src/core/agent_hooks.rs), [pane.rs OSC 消费](https://github.com/l0ng-ai/tty7/blob/e98586bdbd6d0adeffb86cef0028c6d6df3a0994/crates/tty7-core/src/daemon/pane.rs)

**事实,状态机:** 四态 `idle/working/waiting/done`,是 level 不是事件。`SessionStart→idle`、`PromptSubmit→working`、`PermissionRequest/QuestionAsked→waiting`、`Notification` 只在 `working` 时转 `waiting`、`ToolComplete` 只在 `waiting` 时转回 `working`、`Stop→done`、`SessionEnd→idle`。前台进程不再是 agent 时清空整个状态。未安装 hook 的已识别 agent 发出普通 OSC 9/777 通知时标 `waiting`。`tty7 wait` 另加 `no-agent/free/exit`,并用 `--changed` 忽略等待开始前已处于的状态;changelog 记录曾把无人上报的 pane 报成 `idle` 导致 `wait --until idle` 对正在编译的 shell 立即返回。[cli_agent.rs](https://github.com/l0ng-ai/tty7/blob/e98586bdbd6d0adeffb86cef0028c6d6df3a0994/crates/tty7-core/src/core/cli_agent.rs), [orchestration 文档](https://github.com/l0ng-ai/tty7/blob/e98586bdbd6d0adeffb86cef0028c6d6df3a0994/docs/agents/orchestration.mdx)

**事实,各 agent 事件表:** Claude 注册 `SessionStart/UserPromptSubmit/Notification/PostToolUse/Stop/SessionEnd`,没有处理用户中断。Codex 只注册 `SessionStart/UserPromptSubmit/Stop`,安装时执行 `codex features enable hooks`。Gemini 用 `BeforeAgent/AfterAgent/AfterTool`,`Notification` 只有含 `ToolPermission` 才算 `permission-request`;Copilot/Droid/Grok 同样在 hook 内 sniff `permission_prompt`/`elicitation_dialog`。Qwen、Kimi 有一等 `PermissionRequest`;Kimi 把 `Interrupt` 和 `StopFailure` 都映射为 `stop`,注释说明否则 Esc 会让 pane 永久 `working`。OpenCode 用生成的 JS plugin:`session.status busy/idle`、`session.idle`、`permission.ask`、`permission.replied→prompt-submit`、`tool.execute.before→prompt-submit`,并记录子 session id 过滤 subagent 事件。Pi/OMP 用 extension。[事件表](https://github.com/l0ng-ai/tty7/blob/e98586bdbd6d0adeffb86cef0028c6d6df3a0994/crates/tty7-core/src/core/agent_hooks.rs#L685)

**事实,安装:** Settings 页一键安装。JSON 配置(Claude/Codex/Gemini/Droid/Qwen)按 event 合并并用 `agent-hook <slug>` 标记识别自有条目,保留用户 hooks;Kimi 走 `config.toml` 的 `[[hooks]]` 合并;其余 agent 写 tty7 独占文件。有 installed/outdated/missing 三态,`tty7 doctor` 报告 hook 状态,远程机器通过 Host 抽象安装。

**推断:** tty7 对 Claude 的中断、审批后长工具恢复等缺口与本文 [官方能力](#官方能力) 分析一致,它选择接受而不是补屏幕检测。`ToolComplete` 只用于 `waiting→working`,不推进其他状态,避免把工具完成误当回合结束。

### agent-deck

**事实:** Go + tmux 的混合方案。Claude hook 注册 `SessionStart/UserPromptSubmit/Stop/PermissionRequest/Notification(matcher permission_prompt|elicitation_dialog)/SessionEnd`,写入状态文件,由 watcher 读取(限制 64 KiB、`O_NOFOLLOW`,防沙箱内符号链接)。hook 状态有按 agent 区分的“新鲜窗口”:窗口内 hook 为快路径,过期后回落到 `capture-pane` 正则(spinner 字符 + 约 90 个 Claude “thinking”词表 + `esc to interrupt`),再叠加 flicker 检测与防抖。Codex 另有“completion evidence 消费”机制处理稀疏事件。[detector.go](https://github.com/asheshgoplani/agent-deck/blob/5235e160096bd35f5f0484ef8c2c63b9034fa67d/internal/tmux/detector.go), [claude_hooks.go](https://github.com/asheshgoplani/agent-deck/blob/5235e160096bd35f5f0484ef8c2c63b9034fa67d/internal/session/claude_hooks.go), [instance.go](https://github.com/asheshgoplani/agent-deck/blob/5235e160096bd35f5f0484ef8c2c63b9034fa67d/internal/session/instance.go)

**推断:** 时间窗口式双源仲裁把复杂度推到了很高的水平(`instance.go` 超过 6000 行,大量按 issue 编号命名的回归测试)。若 Condr 保留屏幕兜底,更宜采用 herdr 式“按 agent 声明 hook 权威”,而不是按时间新鲜度切换。

### tmux-agent-status 与 claude-squad

**事实:** tmux-agent-status 是纯 hooks 的 bash 实现:Claude `UserPromptSubmit/PreToolUse→working`、`Stop→done`、`Notification→wait`;`Stop` 载荷里 `background_tasks` 有 `running` 项时保持 `working`,避免后台任务未完就显示完成。Codex 用 `SessionStart/UserPromptSubmit/PreToolUse/Stop`,README 提醒需 `/hooks` 信任。claude-squad 只做屏幕:每 tick 比较 pane 内容是否变化判定 Running/Ready,用固定字符串(如 "No, and tell Claude what to do differently")判定提示。[better-hook.sh](https://github.com/samleeney/tmux-agent-status/blob/a323f10eedabc499fc1c8d4e1c73a564c6e3ae70/hooks/better-hook.sh), [claude-squad tmux.go](https://github.com/smtg-ai/claude-squad/blob/ce1ffb4392b01f38e2c4599c7c84d2a93973b138/session/tmux/tmux.go)

### 对 Condr 的启示

1. **纯 hooks 有可运行的同类先例。** tty7 用与 Condr 相同的技术栈发布了 12 种 agent 的 hook 状态,同时明确接受 Claude 中断等缺口,用 `no-agent` 而不是伪造 `idle`。这与本文“允许 `unknown`、收窄支持范围”的建议一致。
2. **传输可选 in-band OSC。** hook 写回控制 tty 让事件与该 PTY 的输出严格同序,不需要 pane id 或 socket 路径,远程 pane 自动生效,tty7 还借此把回合锚定到 scrollback 行。代价是 hook 需定位控制 tty(Windows 走 `AttachConsole`)、单条 OSC 有 8 KiB 上限;tty7 不剥离该序列,依赖 VT 忽略未知 OSC 777,Condr 若采用需确认 `alacritty_terminal` 行为一致。Condr 已有 `CONDR_SOCKET_PATH`,IPC 同样可行;两者都能满足“Server 持有状态”。
3. **Stop 载荷值得多用。** `background_tasks` 判定后台任务、`Notification` 按 `permission_prompt|elicitation_dialog` matcher 过滤、Kimi 的 `Interrupt/StopFailure→stop`,都是低成本补齐缺口的做法。
4. **状态是 level,等待命令要有 `changed`/stale 语义。** tty7 和 agent-deck 都独立踩到“刚 send 就 wait,读到上一回合状态”的问题;Condr 的 `agent wait` 应从一开始区分。
5. **不要按时间窗口混合两种来源。** agent-deck 的实现规模是反例;要么如 tty7 单源,要么如 herdr 按 agent 固定权威。

## 对 Condr 的候选路径

**建议,尚未决定:** 将“是否继续包含 Herdr 实现”和“是否保留屏幕检测”分开决策。独立编写一个小型 hook 适配层可以减少对 Herdr 实现的维护关系,但不会自动消除上游事件缺口。

开源项目复用上游源码本身是正常的工程选择。Apache-2.0 允许符合条件的修改和再分发,要求保留适用的许可证、归属说明和修改标记;这不代替对具体文件的核对。决定替换实现更有用的理由是减少屏幕规则维护、获得明确事件语义。[Apache-2.0 第 2、4 条](https://www.apache.org/licenses/LICENSE-2.0)

1. 先选择明确支持的 CLI/版本,按事件合同实现状态;不承诺所有 agent 自动获得完整检测。
2. Hooks 向运行 agent 的同机 Condr Server 上报,由 Server 持有状态并通过现有协议通知 GUI;GUI 关闭不影响集成。
3. 绑定 pane、启动实例、根 session,拒绝已退出实例的迟到事件;子 agent 的完成不能直接变成根 agent 完成。上报需验证事件与身份、限制输入大小并设置短超时,不能让监测阻塞 agent。取消/失败也不应被无条件当成成功完成通知。
4. 保留进程存活检查处理退出;已知 hook 未安装、禁用或状态无法确定时显式 `unknown`。静默丢失事件无法仅靠后续沉默可靠发现,不得用“最近没有 hook”推定 idle;即使进程仍存活,也可能一直没有新的有效状态。
5. 若要求持续准确的完整三态,在事件缺口处仍需官方状态查询/事件流或范围有限的自有屏幕补充;需要这些补充就不再是纯 hooks。

**验证门槛:** 一次正常回合不足以确认可用。至少实测: 启动/恢复/compact、只思考无工具、权限同意后长工具、拒绝/撤销、多个并发待答、生成中取消、工具中取消、API 错误、Stop hook 要求继续、子 agent、直接退出/强制终止、旧事件迟到、hook 禁用和丢失。跨 Windows/Linux 验证 command 调用和本地 IPC。

**取舍:** 删除屏幕检测可降低对 CLI 显示文本和 manifests 的维护,代价是安装/版本支持边界及部分状态不可知。如果产品接受该边界,这是可行方向;如果要求与当前广泛检测能力等价,纯 hooks 暂无充分证据。
