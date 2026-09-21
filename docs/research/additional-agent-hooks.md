# 新增 Agent hooks：原生契约与测试条件

调查日期：2026-09-11。只读取原生仓库、官方文档与发布元数据；最初仅做源码核查；随后按用户授权将 Pi/OMP 安装至 `/tmp` 并执行版本/帮助检查，未修改用户配置或调用模型。Condr 状态含义以 ADR 0014 为准。本文的源码契约对应固定 commit，不能把主分支新事件默认为旧版 CLI 已支持。

## Pi、OMP、Grok、Kimi 的版本与安装

| CLI | 核查版本 / commit | 安装与 Linux arm64 |
| --- | --- | --- |
| Pi | `0.85.1` / `62129190d81067ec86ae5a5fc907c96bfe435a78` | `npm install -g @earendil-works/pi-coding-agent@0.85.1`，Node >=22.19.0；官方另有 `pi-linux-arm64.tar.gz`；命令 `pi` |
| Oh My Pi | `18.1.17` / `3b3a6dc9bbd85102ce19d0b1c11bf6870915f6ec` | `bun install -g @oh-my-pi/pi-coding-agent@18.1.17`，Bun >=1.3.14；官方有 `omp-linux-arm64`、musl arm64；命令 `omp` |
| Grok Build（xAI 官方） | 源码 `37949780c144e37df692e3d669051a21fec24f20`；官方 stable endpoint 返回 `1.0.25`，两者构建对应关系未验证 | 官方 `https://x.ai/cli/install.sh`；脚本支持 `linux-aarch64`；命令 `grok`。安装脚本仅下载审阅，未执行 |
| Kimi Code | `1.50.0` / `86f136422a0aae6b217ea49e7ea1d2e8a1defcd2` | `uv tool install --python 3.13 kimi-cli==1.50.0`，最低 Python >=3.12；官方有 `kimi-1.50.0-aarch64-unknown-linux-gnu.tar.gz`；命令 `kimi` |

发布证据：[Pi package](https://github.com/badlogic/pi-mono/blob/62129190d81067ec86ae5a5fc907c96bfe435a78/packages/coding-agent/package.json)、[Pi release](https://github.com/badlogic/pi-mono/releases/tag/v0.85.1)、[OMP package](https://github.com/can1357/oh-my-pi/blob/3b3a6dc9bbd85102ce19d0b1c11bf6870915f6ec/packages/coding-agent/package.json)、[OMP release](https://github.com/can1357/oh-my-pi/releases/tag/v18.1.17)、[Grok install](https://x.ai/cli/install.sh)、[Grok stable](https://x.ai/cli/stable)、[Kimi release](https://github.com/MoonshotAI/kimi-cli/releases/tag/1.50.0)。npm registry 同日确认 Pi/OMP latest 为上述版本；Pi 当前包名已是 `@earendil-works/pi-coding-agent`。

“Grok CLI”还可以指社区项目 `superagent-ai/grok-cli`（npm `@vibe-kit/grok-cli`），它与 xAI Grok Build 的配置和事件契约不同。本文按官方 Grok Build 调查；不能把社区项目的 `~/.grok/user-settings.json` 用到官方 CLI。

## Pi

### 接入契约

全局扩展自动加载 `${PI_CODING_AGENT_DIR:-~/.pi/agent}/extensions/*.ts`（也支持 JS、子目录入口）；工程扩展在 `.pi/extensions/`。模块 `export default function(pi) { pi.on(event, async (event, ctx) => ...) }`，可使用 Node `child_process` 运行 `condr agent-hook pi <event>`。测试可显式 `pi --no-extensions -e /absolute/condr.ts`；`--no-extensions` 禁止自动发现，但显式 `-e` 仍加载。见 [加载指南](https://github.com/badlogic/pi-mono/blob/62129190d81067ec86ae5a5fc907c96bfe435a78/packages/coding-agent/docs/extensions.md)、[配置目录](https://github.com/badlogic/pi-mono/blob/62129190d81067ec86ae5a5fc907c96bfe435a78/packages/coding-agent/src/config.ts)。

**所有事件都从 `ctx.sessionManager.getSessionId()` 读取当时的真实会话 ID，并检查 `ctx.mode === "tui"`。`hasUI` 在 TUI 和 RPC 均可能为 true，不是交互终端判据。** 不要用随机 ID、环境变量中的静态 ID，或首次启动缓存的 ID。见 [ExtensionContext](https://github.com/badlogic/pi-mono/blob/62129190d81067ec86ae5a5fc907c96bfe435a78/packages/coding-agent/src/core/extensions/types.ts#L309)。

| 原生事件 | 关键 payload | Condr 语义 |
| --- | --- | --- |
| `session_start` | `reason: startup / reload / new / resume / fork`，`previousSessionFile?` | 更新原生 ID；按 `ctx.isIdle()` 决定状态，reload 不应无条件清除 Working |
| `agent_start` | `type` | Working |
| `agent_end` | `messages` | 只是 agent loop 结束，可能马上 retry、compact、继续队列；不能直接作为最终 Idle |
| `agent_settled` | `type` | 本轮真正结束，不再自动 retry/compact/queued continuation，Idle |
| `ui_prompt_start` | `reason: ui_prompt`，`kind: select / confirm / input / editor / custom`，`title?` | 真实扩展 UI 等待，Blocked |
| `ui_prompt_end` | 同上 | 等待结束；用 `ctx.isIdle()` 恢复 Idle/Working，不能一律 Working |
| `tool_call` / `tool_result` | `toolName`、`toolCallId`、`input` / `content,details,isError` | 工具进度；不需要每工具重新猜测等待 |
| `session_before_compact` / `session_compact` / `session_compact_failed` | `reason: manual / threshold / overflow`，`willRetry`；失败另含 `aborted,errorMessage?` | 压缩不等于新会话，不能清空状态/ID |
| `session_shutdown` | `reason: quit / reload / new / resume / fork` | 不宜一律映射进程退出；reload/换会话也会触发 |

原生定义：[types.ts](https://github.com/badlogic/pi-mono/blob/62129190d81067ec86ae5a5fc907c96bfe435a78/packages/coding-agent/src/core/extensions/types.ts#L559)。`ui_prompt_*` 包裹的是扩展 `ctx.ui` 的阻塞方法，内嵌 UI 以 depth 合并，只在最外层出现/结束触发，不代表所有 Pi 内建菜单；异步以 microtask 投递。见 [runner.ts](https://github.com/badlogic/pi-mono/blob/62129190d81067ec86ae5a5fc907c96bfe435a78/packages/coding-agent/src/core/extensions/runner.ts#L436)。`agent_settled` 是原生最终 idle 边界：[agent-session.ts](https://github.com/badlogic/pi-mono/blob/62129190d81067ec86ae5a5fc907c96bfe435a78/packages/coding-agent/src/core/agent-session.ts#L625)。

Pi 核心没有单独的 permission hook；权限扩展若通过 `ctx.ui.confirm/select/custom` 等实现，其等待可由 `ui_prompt_*` 原生事件识别。错误/取消的最终结果都可以按 settled 结束 Working；Condr 目前没有独立失败态。子进程 `pi --print`/RPC 的扩展必须保持静默；任意第三方扩展在同一 TUI 中自行模拟其他 agent 的行为，不在核心事件保证范围内。

### 测试接口

自定义 `${agentDir}/models.json`：

```json
{"providers":{"condr-test":{"baseUrl":"https://example.invalid/v1","api":"openai-completions","apiKey":"$CONDR_TEST_API_KEY","authHeader":true,"models":[{"id":"model-id"}]}}}
```

`api` 可改 `openai-responses`。Pi 0.85.1 的环境变量引用必须带 `$`；裸大写字符串会被当作字面密钥，这与 OMP 不同。`pi --provider condr-test --model model-id` 用真实 PTY 交互；`-p`/`--mode rpc` 仅验证被过滤，不能代替 TUI 状态验收。会话恢复 `pi --session <id>`。来源：[models.md](https://github.com/badlogic/pi-mono/blob/62129190d81067ec86ae5a5fc907c96bfe435a78/packages/coding-agent/docs/models.md)、[CLI flags](https://github.com/badlogic/pi-mono/blob/62129190d81067ec86ae5a5fc907c96bfe435a78/packages/coding-agent/README.md#cli-reference)。

## Oh My Pi / OMP

### 接入契约

默认 `~/.omp/agent/extensions/condr.ts`，JS/TS default extension factory。**真正的 agent 目录覆盖变量是 `PI_CODING_AGENT_DIR`，不是 `OMP_CODING_AGENT_DIR`。** `PI_CONFIG_DIR` 控制 native 配置根名；命名 profile 使用 `~/.omp/profiles/<name>/agent`，会覆盖普通 agent-dir 选择，后续安装器不能宣称默认目录覆盖所有 profile。默认 profile 下可用 `omp config path` 确认目录。加载器从 native provider 的 `getAgentDir()` 自动发现。见 [dirs.ts](https://github.com/can1357/oh-my-pi/blob/3b3a6dc9bbd85102ce19d0b1c11bf6870915f6ec/packages/utils/src/dirs.ts#L420)、[native discovery](https://github.com/can1357/oh-my-pi/blob/3b3a6dc9bbd85102ce19d0b1c11bf6870915f6ec/packages/coding-agent/src/discovery/builtin.ts#L59)、[extension loader](https://github.com/can1357/oh-my-pi/blob/3b3a6dc9bbd85102ce19d0b1c11bf6870915f6ec/packages/coding-agent/src/extensibility/extensions/loader.ts#L568)。

和 Pi 一样，事件 handler 使用 `ctx.mode === "tui"` 与 `ctx.sessionManager.getSessionId()`。OMP 的 `hasUI` 类型注释称 RPC 无 UI，但实际 runner 只是检查 UI context 是否为 no-op；明确的 `mode` 更准确。子 agent 创建时 `hasUI: false`：[executor.ts](https://github.com/can1357/oh-my-pi/blob/3b3a6dc9bbd85102ce19d0b1c11bf6870915f6ec/packages/coding-agent/src/task/executor.ts#L3497)。

| 原生事件 | 关键 payload | Condr 语义 |
| --- | --- | --- |
| `session_start` | 无 source | 更新 ID 和当前状态 |
| `session_switch` | `reason: new / resume / fork`，`previousSessionFile?` | 更新 ID |
| `session_branch` | `previousSessionFile?` | 也可能改变 ID，必须订阅 |
| `agent_start` | 无额外字段 | Working |
| `agent_end` | `messages`，`willContinue?: boolean` | 仅 `!willContinue` 才 Idle；自动重试/继续保持 Working |
| `session_stop` | `session_id,turn_id,messages,stop_hook_active,signal` | 是可要求继续的前置 gate，不能当最终 Idle |
| `tool_approval_requested` | `sessionId,toolCallId,toolName,approvalMode,reason?` | Blocked |
| `tool_approval_resolved` | `sessionId,toolCallId,toolName,approved,reason?` | 恢复 Working；拒绝也已经结束等待 |
| `tool_execution_start` | `toolName,toolCallId,args` | 内建 `ask` 工具可标记等待回答；其他工具为执行进度 |
| `tool_result` / `tool_execution_end` | `toolName,toolCallId,isError` 等 | 工具/提问返回；多个同时待决请求需保留剩余等待 |
| `session.compacting` / `session_compact` | `sessionId` / `compactionEntry,fromExtension` | 保持当前 ID/状态，不当作新会话 |
| `auto_compaction_end` | `aborted,willRetry,errorMessage?,skipped?` | 压缩完成仍可能继续；不一律 Idle |
| `session_shutdown` | 无 source | 退出由 Condr 进程检测清理 |

定义：[shared-events.ts](https://github.com/can1357/oh-my-pi/blob/3b3a6dc9bbd85102ce19d0b1c11bf6870915f6ec/packages/coding-agent/src/extensibility/shared-events.ts)、[extension types](https://github.com/can1357/oh-my-pi/blob/3b3a6dc9bbd85102ce19d0b1c11bf6870915f6ec/packages/coding-agent/src/extensibility/extensions/types.ts#L919)。`tool_approval_requested` 在工具所需批准 gate 时触发；无 UI 路径也可能发 request 后立即 resolved(false)，所以 TUI 过滤不可省略。[wrapper.ts](https://github.com/can1357/oh-my-pi/blob/3b3a6dc9bbd85102ce19d0b1c11bf6870915f6ec/packages/coding-agent/src/extensibility/extensions/wrapper.ts#L271)。审批事件若有 `sessionId`，需与 `ctx.sessionManager.getSessionId()` 比较，防止共享 runner 里的子会话污染。OMP 自己的 Warp bridge 也订阅会话切换、审批及 `ask` 工具：[warp-events.ts](https://github.com/can1357/oh-my-pi/blob/3b3a6dc9bbd85102ce19d0b1c11bf6870915f6ec/packages/coding-agent/src/modes/warp-events.ts#L163)。

### 测试接口

`${agentDir}/models.yml` 的 `providers.<id>` 支持 `baseUrl,apiKey,api,models`；`api` 为 `openai-completions` 或 `openai-responses`。`apiKey` 可以指定环境变量名。模型定义应明确 `id,name,reasoning,input,cost,contextWindow,maxTokens`；按实际端点选择兼容参数。使用 `omp --provider condr-test --model model-id`，实际可用 flags 先执行对应版本 `--help`。来源：[models.md](https://github.com/can1357/oh-my-pi/blob/3b3a6dc9bbd85102ce19d0b1c11bf6870915f6ec/docs/models.md)。

## Grok Build（xAI 官方）

### 接入契约

独立 `${GROK_HOME:-~/.grok}/hooks/condr.json`，自动合并同目录所有 JSON，无需修改用户主配置。基本结构：

```json
{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"condr agent-hook grok session-start","timeout":5}]}]}}
```

原生配置支持 `SessionStart,UserPromptSubmit,PreToolUse,PostToolUse,PostToolUseFailure,PermissionDenied,Stop,StopFailure,StopCancelled,Notification,SubagentStart,SubagentStop,PreCompact,PostCompact,SessionEnd`；`SubagentEnd` 是别名。没有独立 `PermissionRequest` event，应订阅 `Notification` 并检查 `notificationType === "permission_prompt"`。`Stop`/`UserPromptSubmit` 不需要 matcher，其他事件可按结构化字段过滤。来源：[官方 hooks guide](https://github.com/xai-org/grok-build/blob/37949780c144e37df692e3d669051a21fec24f20/crates/codegen/xai-grok-pager/docs/user-guide/10-hooks.md)。

共同字段：`hookEventName`（snake_case event value）、`sessionId,cwd,workspaceRoot,timestamp`，可选 `transcriptPath,clientIdentifier,promptId,permissionMode`。新版同时提供 `session_id,hook_event_name` 等 snake aliases，其中 `hook_event_name` 的 value 是 PascalCase。**`notificationType`、`subagentType` 没有通用 snake alias**，解析器要明确支持 camelCase。环境同时提供 `GROK_HOOK_EVENT,GROK_SESSION_ID,GROK_WORKSPACE_ROOT`。来源：[event.rs](https://github.com/xai-org/grok-build/blob/37949780c144e37df692e3d669051a21fec24f20/crates/codegen/xai-grok-hooks/src/event.rs#L354)、[command runner](https://github.com/xai-org/grok-build/blob/37949780c144e37df692e3d669051a21fec24f20/crates/codegen/xai-grok-hooks/src/runner/command.rs#L221)。

- `SessionStart` 带 `source,modelId?,agentType?`；只在主会话启动触发。
- `UserPromptSubmit` → Working；`PreToolUse/PostToolUse/PostToolUseFailure` 带 `toolName,toolUseId,toolInput`，结果事件带 `toolResult` 或 `error`；用于退出等待态。
- `Notification` 的 `notificationType: permission_prompt` → Blocked。原生单测确认自动允许和 yolo 不发，而真实用户提示会发。没有配对的 permission_resolved hook，等待恢复要依靠后续工具/停止事件，因此批准后长工具可能直到结束才退出 Blocked。`idle_prompt` 是持续 idle 通知，不能拿任意通知等同回合完成；`task_complete` 可以是后台任务完成。
- `Stop` 是主 agent 正常完成；`StopFailure.error` 为 `rate_limit/authentication_failed/invalid_request/server_error/max_output_tokens/unknown`；`StopCancelled.reason` 为 `user_interrupt/permission_rejected/permission_cancelled/max_turns/no_progress/unknown`。Condr 可都结束 Working，但失败/取消不是独立 wire state。
- 前台子 agent 的 `UserPromptSubmit`、工具事件、`StopFailure`、`StopCancelled`、`SessionEnd` 含 `subagentType`，应过滤；`SubagentStart/Stop` 只描述子任务，不能清父状态。主 agent `Stop` 与子 agent `SubagentStop` 已分离。部分全局 permission manager 把子工具审批转成根会话通知，表示用户当前确实需要回答。
- 压缩用 `PreCompact/PostCompact.source`，无需新建会话。

审批证据：[updates.rs](https://github.com/xai-org/grok-build/blob/37949780c144e37df692e3d669051a21fec24f20/crates/codegen/xai-grok-shell/src/session/acp_session_impl/updates.rs#L929)、[原生审批测试](https://github.com/xai-org/grok-build/blob/37949780c144e37df692e3d669051a21fec24f20/crates/codegen/xai-grok-shell/src/session/acp_session_tests/permission_prompt_notification_tests.rs)。

**跨 CLI 误触发必须测试：Grok 默认还导入 `~/.claude/settings.json`、`~/.cursor/hooks.json`。** 若其中有 `condr agent-hook claude ...`，Grok 也会运行；可从原生 `GROK_HOOK_EVENT` 或 `hookEventName` 识别其来源，并在不匹配 agent adapter 时静默。不要通过禁用用户现有兼容来源解决 Condr 自己的身份误判。该行为在官方 hooks guide 的 Hook Locations 表中有明确记录。

### 测试接口

`grok --version` / `grok --help` / `grok models`，交互 `/hooks` 检查加载，`grok -p "Hello" -m <model>` 可做非交互辅助测试。自定义 `${GROK_HOME}/config.toml`：

```toml
[model.condr-test]
model = "model-id"
base_url = "https://example.invalid/v1"
env_key = "CONDR_TEST_API_KEY"
api_backend = "chat_completions"
context_window = 128000
```

`api_backend` 支持 `chat_completions`（默认）、`responses`、`messages`；`grok -m condr-test` 选择测试模型。不必使用 xAI 模型。来源：[custom models](https://github.com/xai-org/grok-build/blob/37949780c144e37df692e3d669051a21fec24f20/crates/codegen/xai-grok-pager/docs/user-guide/11-custom-models.md)。发布二进制是否已经包含此 commit 所有 hook 字段，仍需真实安装验收。

## Kimi Code

### 接入契约与限制

`${KIMI_SHARE_DIR:-~/.kimi}/config.toml` 的顶层 `[[hooks]]` 数组：

```toml
[[hooks]]
event = "SessionStart"
command = "condr agent-hook kimi session-start"
timeout = 5
```

每条有 `event,command,matcher?,timeout?`；不是 Claude 的多层 JSON schema。可用 `--config-file` 指定测试配置。共同 stdin JSON 为 `hook_event_name,session_id,cwd`。配置及 payload 源码：[config.py](https://github.com/MoonshotAI/kimi-cli/blob/86f136422a0aae6b217ea49e7ea1d2e8a1defcd2/src/kimi_cli/hooks/config.py)、[events.py](https://github.com/MoonshotAI/kimi-cli/blob/86f136422a0aae6b217ea49e7ea1d2e8a1defcd2/src/kimi_cli/hooks/events.py)、[share.py](https://github.com/MoonshotAI/kimi-cli/blob/86f136422a0aae6b217ea49e7ea1d2e8a1defcd2/src/kimi_cli/share.py)。

原生 13 种事件：`PreToolUse,PostToolUse,PostToolUseFailure,UserPromptSubmit,Stop,StopFailure,SessionStart,SessionEnd,SubagentStart,SubagentStop,PreCompact,PostCompact,Notification`。工具事件含 `tool_name,tool_input,tool_call_id`；Stop 带 `stop_hook_active`；StopFailure 带 `error_type,error_message`；SessionStart 带 `source: startup/resume`；子事件带 `agent_name,prompt/response`；压缩带 `trigger` 和 token count；Notification 带 `sink,notification_type,title,body,severity`。

**仅按上述事件直接接入还不能可靠识别主 agent 状态，主要有三个原生缺口：**

1. 前台子 agent 共享父 HookEngine，`run()` 同样发 `UserPromptSubmit/Stop`，toolset 同样发工具事件；子 runtime 复用父 `session.id`，这些 payload 没有 `agent_id` 或 root/subagent 字段。同进程内无法通过 Condr 的进程祖先过滤区分。只忽略 `SubagentStart/Stop` 无法解决。子 agent 错误/取消又不保证对应 `SubagentStop`，因此外部计数器也不能可靠修补。证据：[subagent runner](https://github.com/MoonshotAI/kimi-cli/blob/86f136422a0aae6b217ea49e7ea1d2e8a1defcd2/src/kimi_cli/subagents/runner.py#L245)、[共享 parent session](https://github.com/MoonshotAI/kimi-cli/blob/86f136422a0aae6b217ea49e7ea1d2e8a1defcd2/src/kimi_cli/soul/agent.py#L338)、[run hooks](https://github.com/MoonshotAI/kimi-cli/blob/86f136422a0aae6b217ea49e7ea1d2e8a1defcd2/src/kimi_cli/soul/kimisoul.py#L663)。
2. 没有审批出现/回复 hook。`Notification` 在根 agent 消费后台通知时触发，不表示审批；不能映射为 Blocked。`PreToolUse` 的 `AskUserQuestion` 可发现提问工具，但 print/afk 会自动跳过提问，仍不是“确实等待”事件。[通知触发](https://github.com/MoonshotAI/kimi-cli/blob/86f136422a0aae6b217ea49e7ea1d2e8a1defcd2/src/kimi_cli/soul/kimisoul.py#L1131)、[AskUserQuestion](https://github.com/MoonshotAI/kimi-cli/blob/86f136422a0aae6b217ea49e7ea1d2e8a1defcd2/src/kimi_cli/tools/ask_user/__init__.py#L73)。
3. 用户取消走 `asyncio.CancelledError`，只发 wire `TurnEnd`，不会走正常 Stop hook；Stop 被其他 hook 要求继续后，这个后续回合没有第二次最终 Stop。Condr 不能用按键或屏幕补猜。[run completion](https://github.com/MoonshotAI/kimi-cli/blob/86f136422a0aae6b217ea49e7ea1d2e8a1defcd2/src/kimi_cli/soul/kimisoul.py#L743)。

可靠的完整支持需要上游给 hook payload 增加原生角色/agent ID，或提供能区分根活动的公开扩展事件；现有 wire 协议虽然包含更多事件，但接管 wire UI 会偏离 Condr 嵌原生 TUI 的当前设计。不能把首批 Kimi 支持写成已完整覆盖。

### 测试接口

支持用户提供的 OpenAI 端点，provider type 必须选 `openai_legacy`（Chat Completions）或 `openai_responses`；两者读取 `OPENAI_BASE_URL,OPENAI_API_KEY`，不是 `KIMI_BASE_URL`（后者只覆盖 `type=kimi`）。例如：

```toml
default_model = "condr-test"
[providers.condr-test]
type = "openai_legacy"
base_url = "https://example.invalid/v1"
api_key = "placeholder-overridden-by-env"
[models.condr-test]
provider = "condr-test"
model = "model-id"
max_context_size = 128000
```

`kimi --config-file <file> --model condr-test` 进入交互，`--print` 自动启用 afk，不能用于审批验收。`--prompt` 可以保留交互显示，但完成后退出。`kimi --session <id>` 恢复会话。[provider implementation](https://github.com/MoonshotAI/kimi-cli/blob/86f136422a0aae6b217ea49e7ea1d2e8a1defcd2/src/kimi_cli/llm.py#L276)、[CLI options](https://github.com/MoonshotAI/kimi-cli/blob/86f136422a0aae6b217ea49e7ea1d2e8a1defcd2/docs/en/reference/kimi-command.md)。

## 四种 CLI 的最小验收

离线回归保留真实原生 payload fixtures，检查序列及进程/子会话过滤。扩展用假 ExtensionAPI 驱动真实生成模块，并把 `ctx.mode` 分别设 tui/rpc/print；会话改变时 getSessionId 返回新 ID，不能让事件只复用最初 ID。Pi 检查 agent_end 不先 Idle、agent_settled 才 Idle，UI prompt 出入；OMP 检查 willContinue、多个审批请求/回复与 session_switch/session_branch。Grok 检查 camelCase 字段、子agent过滤与它加载 Claude/Cursor hooks 时不串身份。Kimi fixture 必须体现父子共享 payload 的已知不可判定性，不能写一个不存在的 agent_id fixture 自证过滤有效。

安装后在独立配置根、临时目录、真实 PTY 跑启动、单轮、工具批准/拒绝、提问回答/取消、Ctrl+C、退出、resume/new、压缩及子 agent；模型接口提供前可先验证启动、配置加载与扩展自定义对话 UI，真实工具调用和结束事件仍需最少一次模型对话。Linux arm64 验证 Server 状态；Windows 另验 hook 进程到 ConPTY 路径。

## 文件许可证核查

Pi 上述 `packages/coding-agent/src/core/extensions/{types,runner}.ts`、`agent-session.ts` 与 config/docs 均无独立许可证头或子目录 LICENSE；根 [MIT LICENSE](https://github.com/badlogic/pi-mono/blob/62129190d81067ec86ae5a5fc907c96bfe435a78/LICENSE)。OMP 所列 extension/shared-events/dirs/discovery/model 文件同样无独立许可证头，根 [MIT LICENSE](https://github.com/can1357/oh-my-pi/blob/3b3a6dc9bbd85102ce19d0b1c11bf6870915f6ec/LICENSE)。

Kimi hooks/*.py、soul/kimisoul.py、subagents/runner.py、llm.py 没有不同许可证头；根 [Apache-2.0 LICENSE](https://github.com/MoonshotAI/kimi-cli/blob/86f136422a0aae6b217ea49e7ea1d2e8a1defcd2/LICENSE) 和 [NOTICE](https://github.com/MoonshotAI/kimi-cli/blob/86f136422a0aae6b217ea49e7ea1d2e8a1defcd2/NOTICE) 已核查，Codex 衍生项指 skill-creator 文档，不在此次适配范围。

Grok 的 hooks/event.rs、runner/command.rs 和 hook docs 没有不同文件许可，根 [Apache-2.0 LICENSE](https://github.com/xai-org/grok-build/blob/37949780c144e37df692e3d669051a21fec24f20/LICENSE)；另有 [THIRD-PARTY-NOTICES](https://github.com/xai-org/grok-build/blob/37949780c144e37df692e3d669051a21fec24f20/THIRD-PARTY-NOTICES) 和 third_party/NOTICE，不能据根许可无条件搬其他目录。Condr 应仅按公开事件契约独立实现小适配器，不复制 CLI 或 Superset 实现。


## 本机安装与启动/恢复补充

已在本机 Linux arm64 实际执行成功：

- `/tmp/condr-agent-clis/pi/node_modules/.bin/pi --version` → `0.85.1`，`--help` 成功。由 `npm install --prefix /tmp/condr-agent-clis/pi --no-audit --no-fund @earendil-works/pi-coding-agent@0.85.1` 安装。npm 提示 esbuild/protobufjs 等部分 install scripts 未被执行，当前版本/帮助正常；扩展加载将在实际 PTY 测试确认。
- `/tmp/condr-agent-clis/omp/omp --version` → `omp/18.1.17`，`--help` 成功。下载官方 `omp-linux-arm64`，SHA256 `58ab1b8f75d202cf3767e19901c834d2435898a189d5993be6d5cae6736716e1` 与 GitHub release asset digest 相同。
- 两次验证分别用 `PI_CODING_AGENT_DIR=/tmp/condr-agent-clis/pi/config` 与 `/tmp/condr-agent-clis/omp/config`。

| CLI | 精确恢复参数 | 首次上报时机 |
| --- | --- | --- |
| Pi | `pi --session <id>`；`--resume` 本身只是会话选择器 | 先启动 TUI，再 `bindExtensions(mode=tui)`，随后 `session_start`；首次 prompt 前可上报。reload 的 session_start 要读取当前 Idle/Working，而非总置 Idle |
| OMP | `omp --resume <id>`；`--session <id>` 同一 parser alias | `extensionRunner.initialize(...,uiContext,"tui")` 后立即 `session_start`；首次 prompt 前可上报 |
| Grok | `grok --resume <id>`；`--session-id` 创建新 UUID，不是恢复 | 主会话启动触发 SessionStart，子会话不发；发布二进制具体时机待验收 |
| Kimi | `kimi --session <id>` / `--resume <id>` | CLI 创建/恢复 KimiCLI 实例后运行 SessionStart，再进入 UI；会话ID不存在时此参数也可能创建新会话 |

启动证据：[Pi interactive binding](https://github.com/badlogic/pi-mono/blob/62129190d81067ec86ae5a5fc907c96bfe435a78/packages/coding-agent/src/modes/interactive/interactive-mode.ts#L905)、[Pi bindExtensions](https://github.com/badlogic/pi-mono/blob/62129190d81067ec86ae5a5fc907c96bfe435a78/packages/coding-agent/src/core/agent-session.ts#L2468)、[OMP controller](https://github.com/can1357/oh-my-pi/blob/3b3a6dc9bbd85102ce19d0b1c11bf6870915f6ec/packages/coding-agent/src/modes/controllers/extension-ui-controller.ts#L303)、[OMP resume flags](https://github.com/can1357/oh-my-pi/blob/3b3a6dc9bbd85102ce19d0b1c11bf6870915f6ec/docs/cli-reference.md)、[Grok session flags](https://github.com/xai-org/grok-build/blob/37949780c144e37df692e3d669051a21fec24f20/crates/codegen/xai-grok-pager/docs/user-guide/17-sessions.md)。

OMP 的 `ask` 最好用 `tool_execution_start` 而非更早的 `tool_call` 标记 Blocked：后者发生在调度/approval 前。实际 `AskTool.execute()` 用 UI `select/editor`，`tool_result`/`tool_execution_end` 用 `toolCallId` 清除；非法输入、空问题列表会直接返回而不显示 UI，需允许短暂请求后立即收尾。没有比工具执行更精确的公开 ask UI lifecycle event。见 [ask.ts](https://github.com/can1357/oh-my-pi/blob/3b3a6dc9bbd85102ce19d0b1c11bf6870915f6ec/packages/coding-agent/src/tools/ask.ts#L869)。

独立测试配置可以禁止跨 CLI 配置读取：Grok `config.toml` 写 `[compat.claude] hooks=false`、`[compat.cursor] hooks=false`；环境覆盖分别是 `GROK_CLAUDE_HOOKS_ENABLED=0`、`GROK_CURSOR_HOOKS_ENABLED=0`（[原生名称](https://github.com/xai-org/grok-build/blob/37949780c144e37df692e3d669051a21fec24f20/crates/codegen/xai-grok-tools/src/types/compat.rs#L130)）。OMP `config.yml` 的 `disabledProviders` 可以列 `claude,codex,gemini,github,opencode,cursor` 禁止这些来源；这是所有配置类别的开关，不仅 hooks。外部用户级来源默认不启用，项目级默认仍会启用（[settings.md](https://github.com/can1357/oh-my-pi/blob/3b3a6dc9bbd85102ce19d0b1c11bf6870915f6ec/docs/settings.md#provider-and-source-disabling)）。产品集成仍须能处理用户允许的跨 CLI hook，不宜替用户禁用其既有配置。

## Google Antigravity CLI

调查的是 Google 原生 `agy`，不是 IDE 的 `antigravity` 启动命令。2026-09-11 的官方 Linux arm64 更新 manifest 给出 **1.2.1**；下载后按 manifest SHA-512 校验，仅解包至 `/tmp/condr-agent-clis/agy/agy`，跳过会写 shell profile 的 `install` 子命令。`agy --help` 在本机正常运行；恢复参数是 `--conversation ID`，最近会话是 `--continue`。官方有 Windows、macOS、Linux 安装器。[安装文档](https://antigravity.google/docs/cli/install/)、[Linux arm64 manifest](https://antigravity-cli-auto-updater-974169037036.us-central1.run.app/manifests/linux_arm64.json)

建议以自有 plugin 安装：`~/.gemini/antigravity-cli/plugins/condr/plugin.json` 写 `{"name":"condr","description":"Condr agent status hooks"}`，相邻 `hooks.json` 放 named hook 配置。插件按目录自动发现；已被用户 disable 的插件不能只凭文件存在声称生效。CLI 文档也提到 settings.json 内 hooks，但未给出其准确结构；不采用猜测的 Gemini CLI schema。[CLI plugins](https://antigravity.google/docs/cli/plugins/)

Google 的统一 hooks 文档在 common payload 中明确区分 CLI 与桌面 artifact path，因此下列契约涵盖 CLI：

```json
{
  "condr": {
    "PreInvocation": [{"type":"command","command":"condr agent-hook antigravity prompt-submit","timeout":5}],
    "PreToolUse": [{"matcher":"*","hooks":[{"type":"command","command":"condr agent-hook antigravity tool-start","timeout":5}]}],
    "PostToolUse": [{"matcher":"*","hooks":[{"type":"command","command":"condr agent-hook antigravity tool-complete","timeout":5}]}],
    "Stop": [{"type":"command","command":"condr agent-hook antigravity stop","timeout":5}]
  }
}
```

事件 payload 使用 `conversationId`；工具名称在 `toolCall.name`，参数在 `toolCall.args`。PreInvocation 是每次模型调用开始，不是每条用户消息开始。`Stop.fullyIdle=false` 表示仍有 background commands/async tasks，不能标记完全 Idle；只有 `true` 可作为完成。`terminationReason` 有 model_stop/max_steps_exceeded/error。特定原生 `ask_question` 工具可报告等待用户；普通 PreToolUse 发生在权限判断前，不能通用映射为 Blocked。没有公开、明确的实际权限对话开始 hook。[统一 hook 配置与 payload](https://antigravity.google/docs/hooks/)

SessionStart 虽出现在 1.2.1 binary 的类型/函数符号中，公开事件表未列出，不据此承诺启动 hook；建议首个可信原生事件前保持 Unknown。子 agent 共用哪些 hooks、怎样排除同进程 subagent，公开 common schema 未给出 parent/root 标志，仍须真实会话验证。

`agy plugin validate` 对上述 named schema 返回成功，但对故意无效 FakeEvent 同样成功；它只证明文件被扫描，不能证明事件会触发。本次没有 Google/Gemini 凭证，未完成真实模型会话。`JETSKI_APP_DATA_DIR` 不足以隔离 CLI plugin discovery，此变量不是已确认的用户配置目录覆盖机制。

自带 Gemini key 模式要求 settings.json 的 `modelProvider:"gemini"` 与 `GEMINI_API_KEY` 同时设置；custom base URL 是 `GOOGLE_GEMINI_BASE_URL`，协议为 Gemini。该路径不能直接消费 Codex Responses 接口；也可 Google account OAuth 登录。[原生认证及 custom endpoint](https://antigravity.google/docs/cli/install/#using-a-gemini-api-key)

## Cursor CLI

官方安装脚本 2026-09-11 指向 **2026.09.10-fd3934a**，支持 Linux/macOS 的 x64/arm64，提供 `agent` 与 `cursor-agent` 两个 symlink。为避免 generic `agent` 误识别，Condr 用 `cursor-agent` 发现即可。包已隔离下载 `/tmp/condr-agent-clis/cursor`；真实 `cursor-agent --help`、`create-chat` 已运行。恢复为 `--resume ID`。[官方安装器](https://cursor.com/install)、[CLI 参数](https://docs.cursor.com/en/cli/reference/parameters)

User hooks 使用 `~/.cursor/hooks.json`，`version:1` 与事件名→command array，不是 Claude 的 matcher-group 套 hooks。实际 2026.09.10 包读取路径顺序是 `CURSOR_CONFIG_DIR`、`XDG_CONFIG_HOME/cursor`、`~/.cursor`；另有 `CURSOR_DATA_DIR` 隔离状态。

```json
{
  "version":1,
  "hooks":{
    "sessionStart":[{"command":"condr agent-hook cursor session-start","timeout":5}],
    "beforeSubmitPrompt":[{"command":"condr agent-hook cursor prompt-submit","timeout":5}],
    "preToolUse":[{"command":"condr agent-hook cursor tool-start","timeout":5}],
    "postToolUse":[{"command":"condr agent-hook cursor tool-complete","timeout":5}],
    "postToolUseFailure":[{"command":"condr agent-hook cursor tool-complete","timeout":5}],
    "stop":[{"command":"condr agent-hook cursor stop","timeout":5}]
  }
}
```

通用输入 `conversation_id` 是跨 turn 稳定 ID；sessionStart 文档也提供 `session_id`，不要把 generation_id 当会话。stop 的 status 为 completed/aborted/error。preToolUse、beforeShellExecution 与 beforeMCPExecution 是执行前检查，不能证明 CLI 已在等人批准。没有公开的实际 permission-wait hook；当前集成不应虚构完整 Blocked 覆盖。subagentStop 是独立事件，不用于父会话 Stop。[Hook 配置和语义](https://cursor.com/docs/hooks)

实际安装包 `1931.index.js` 的 CLI sessionStart 调用受 `!resume` 限制，且异步等待 model 准备；恢复时不会发启动事件，故 Condr 不应无限等待它才发送恢复后的用户 prompt。`190.index.js` 的 hook service 自动补齐 session_id；CLI 普通调用的 conversation_id 在每轮保持同一会话。

普通 CLI 的 `CURSOR_API_KEY` 是 Cursor dashboard key，`--endpoint`/`CURSOR_API_ENDPOINT` 指向 Cursor 自己的 RPC backend（默认 api2.cursor.sh），不能直接替换为 OpenAI Responses endpoint。安装包存在隐藏 `--base-url`、`--authless`，但其代码显式仅允许另一个 `agent-cli-local` runtime；不能据隐藏字符串声称公开 cursor-agent 支持 OpenAI BYOK。本次没有 Cursor 凭证，尚未验证真实模型会话。[CLI 认证](https://docs.cursor.com/en/cli/reference/authentication)

## GitHub Copilot CLI

本机隔离安装了官方 **@github/copilot 1.0.83**（Linux arm64），执行文件 `/tmp/condr-agent-clis/copilot/node_modules/.bin/copilot`。npm 安装自动选择 `@github/copilot-linux-arm64`。真实 --help/`help providers` 正常。恢复用 `--resume=ID`，避免可选参数歧义。[官方 CLI 仓库](https://github.com/github/copilot-cli)

用户 hooks 可写 `$COPILOT_HOME/hooks/condr.json`，默认 `~/.copilot/hooks/condr.json`；不需要 wrapper，也不需要改项目。支持原生 exec+args，Windows 无需拼 PowerShell command：

```json
{
  "version":1,
  "hooks":{
    "userPromptSubmitted":[{"type":"command","exec":"condr","args":["agent-hook","copilot","prompt-submit"],"timeoutSec":5}],
    "agentStop":[{"type":"command","exec":"condr","args":["agent-hook","copilot","stop"],"timeoutSec":5}],
    "notification":[{"type":"command","exec":"condr","args":["agent-hook","copilot","permission-request"],"matcher":"permission_prompt|elicitation_dialog","timeoutSec":5}]
  }
}
```

camelCase 事件名提供 `sessionId`/`toolName`。还可配置 sessionStart、preToolUse、postToolUse、postToolUseFailure；工具成功和失败后仍继续模型循环，均按 ToolComplete 恢复 Working。主 agent 每轮完成用 agentStop；不要注册 subagentStop。当 errorOccurred.recoverable=true 时不能停掉 Working。permissionRequest 在规则、session approval、auto-allow/deny 前就发，不能用作实际等待；用 notification matcher permission_prompt|elicitation_dialog。notification 的 agent_completed/agent_idle 描述后台子 agent，不用于主会话完成。[原生 hooks reference](https://docs.github.com/en/copilot/reference/hooks-reference)

**真实运行检验**：`/tmp/condr-copilot-hook-probe.py` 启动 localhost mock Chat Completions SSE provider，在临时 cwd/profile 下执行真实 CLI `-p 'Reply OK. Do not use tools.'`；原生用户目录 exec/args hooks 确实触发。实际顺序：

```
userPromptSubmitted
sessionStart {source:"new",initialPrompt:"Reply OK. Do not use tools."}
agentStop {stopReason:"end_turn"}
sessionEnd {reason:"complete"}
```

因此 sessionStart 带非空 initialPrompt 时不能把已经 Working 的会话降回 Idle。此次测试证明原生 hook 加载与投递，mock response 不能证明真实模型工具能力；未验证交互式 permission / subagent / Windows ConPTY。

官方网页介绍 OpenAI Chat Completions BYOK，但 **1.0.83 实际 `copilot help providers` 还明确支持 `COPILOT_PROVIDER_WIRE_API=responses`**。可将用户 Codex 配置映射到 `COPILOT_PROVIDER_BASE_URL`、`COPILOT_PROVIDER_API_KEY`、`COPILOT_MODEL`，并设 wire=responses、transport=http（默认）。`COPILOT_OFFLINE=true` 可使 CLI 不访问 GitHub，仅连指定 provider，BYOK 不需要 GitHub 登录。[BYOK 网页](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/use-byok-models)、[BYOK 发布公告](https://github.blog/changelog/2026-04-07-copilot-cli-now-supports-byok-and-local-models/)

上述安装包仅用于只读研究和原生运行，未移植其实现；这些 CLI 的分发许可证不能从 npm 包是否可下载推断为允许复制。

### Copilot TUI 启动时序补测

`/tmp/condr-copilot-pty-probe.py` 用真实 PTY、同一 localhost mock provider 验证两种启动：`copilot -i 'Reply OK...'` 与裸 `copilot` 等待 8 秒再输入 prompt。两者都在第一条用户消息后才发 sessionStart，顺序同样是 userPromptSubmitted→sessionStart(initialPrompt 非空)→agentStop。裸 TUI 等待期间没有启动 hook。因此 **Copilot reports_at_startup 应为 false**，不能先等 hook 再送首条 prompt。临时 config.json 设置 trustedFolders:[临时 cwd]、banner:never；PTY 需响应终端位置查询 ESC[6n，退出可送 `/exit\r`。原始画面日志仅用于人工检验测试驱动，不作为 Condr 状态检测输入。

### Antigravity 二进制附带 guide 补充

1.2.1 内置的 hooks guide 明确 shell 命令 Unix 用 `sh -c`、Windows 用 `cmd /c`，cwd 是 hooks.json 所在目录。Hook stdout 必须 JSON；PostToolUse 使用 `{}`，PreInvocation 的 injectSteps 可省略；Stop 只有 `decision:"continue"` 才强迫继续，其余允许退出。Condr 的 observer 不应返回 `allow` 从而抢改工具权限。fullyIdle 是 proto3 bool（false 可在序列化时省略），缺字段按 false 忽略停止事件更保守。HookArgsCommon 反射字段虽有额外 agentName，仍没有可据以判断 main/subagent 的明确 parent/root 标志。

## Condr 实现与验收结果

本轮新增 Pi、OMP、Antigravity CLI、官方 Grok Build、Cursor CLI、GitHub Copilot CLI 的安装/卸载/状态查询及原生事件适配；进程发现、原生 conversation ID、恢复参数和 GUI 图标同步扩展。Kimi 可发现进程，但 hooks 返回 `unsupported`，GUI 显示 Unavailable，不提供不可用的安装按钮。未写用户真实 hook/config 文件，CLI 安装及测试配置均在 `/tmp/condr-agent-clis` 或独立临时目录。

Grok 使用 `promptId` 关联当前回合，丢弃旧回合延迟到达的完成；额外订阅 `idle_prompt` 修复漏报。发布版 `grok 1.0.25 (f7e67d6988e2)` 的新建 source 为 `new`，恢复为 `load`，退出会另报 `Stop(reason=shutdown)`。`Stop` 本身是前置 gate；其他 Stop hook 要求继续时仍可能短暂/持续显示 Idle，直到新的运行事件或原生 idle 通知。Condr 不猜测其它 hook 的结果。

OMP 的默认 profile 安装路径遵守 `PI_CONFIG_DIR`（默认 `.omp`，相对 home），非空 `PI_CODING_AGENT_DIR` 优先。已与真实 OMP 18.1.17 的 `config path` 比对自定义配置根、空覆盖及显式 agent 目录三种情况，结果一致；CLI 回归还覆盖配置根的前导 `/`。命名 profile 仍需安装到各自 agent 目录。

Windows Grok 默认通过 PowerShell 执行 hooks；Condr 不在 PATH 时，带引号的路径前添加 `&`。安装或检查时若 `GROK_SHELL` 为 `bash/gitbash/git-bash/cmd/cmd.exe`，使用普通命令形式。安装 hooks 与启动 Grok 应使用相同的 `GROK_SHELL`；更改后重新执行 `condr agent hooks install grok`。依据固定上游的 [shell resolver](https://github.com/xai-org/grok-build/blob/37949780c144e37df692e3d669051a21fec24f20/crates/codegen/xai-grok-config/src/shell.rs) 与 [hook runner](https://github.com/xai-org/grok-build/blob/37949780c144e37df692e3d669051a21fec24f20/crates/codegen/xai-grok-hooks/src/runner/command.rs)。

Copilot 1.0.83 的真实 TUI + localhost 401 provider 补测发现：`userPromptSubmitted → sessionStart(initialPrompt) → errorOccurred(recoverable:true)`（错误重试三次）后已回到输入框，却没有 `agentStop`；退出才有 `sessionEnd(user_exit)`。因此 API 错误后 Condr 可能持续 Working，直到下一次完成或进程退出。`recoverable:true` 也覆盖仍在重试的中间错误，不能直接映射 Idle；CLI/GUI 的 hooks note 明确提示此原生限制。

跨 CLI 配置导入统一在 `condr agent-hook` 的进程祖先检查中拒绝：例如 Grok 运行 Claude 的 hook 命令，不会把 Pane 报成 Claude。检查在拥有该 Pane 的 `condr server` 处停止，启动 Server 的外层 coding agent 不参与判断。

真实 Linux arm64 对话已通过 Condr Server → PTY → 原生 CLI → 原生 hook/extension → `condr agent-hook` → OSC 777 → Server 状态全链路验证：

| CLI | 实际版本 | Codex 配置的 Responses 模型 | 原生 hook 额外验证 |
| --- | --- | --- | --- |
| Pi | 0.85.1 | `gpt-6-astra` 成功回复 `CONDR_HOOK_OK`，最终 Idle，保存真实会话 ID | 原生 `ctx.ui.confirm` 已通过 Condr 验证 Idle → Blocked → 取消 → Idle；API 401 的 settled 也会结束 Working |
| OMP | 18.1.17 | 同上 | 模型实际调用 `ask`，Condr 显示 Blocked，取消后回到 Idle。临时配置 `startup.setupWizard:false`；真实使用先完成原生初始化 |
| Grok | 1.0.25 | 同上 | 本地 mock + 真实 PTY 验证 permission_prompt、Ctrl+C → permission_cancelled、resume 保持 ID |
| Copilot | 1.0.83 | 同上，wire=responses、transport=http | 本地 mock + 裸 TUI 证实第一条 prompt 之后才报 SessionStart；初次未知状态启动后先确认 TUI 已可输入 |
| Cursor | 2026.09.10-fd3934a | 未验证：需要 Cursor 账号/API key | 已验证安装、原生参数和配置格式；没有权限等待事件 |
| Antigravity | 1.2.1 | 未验证：需要 Google 登录或 Gemini API key | 已校验下载、运行 help、验证插件格式；validator 不证明事件实际触发，同进程子agent仍待真机验证 |
| Kimi | 源码 1.50.0 | 未运行 | 上游 hook 不能可靠区分父子，不安装状态适配器 |

凭证按用户授权读取本机 Codex provider 配置，优先使用配置中明确设置的 bearer token；没有把凭证、私有端点地址或认证文件写进仓库。Pi/OMP 自定义 provider 添加 `authHeader:true`，Pi 用 `$CONDR_TEST_API_KEY`，OMP 用 `CONDR_TEST_API_KEY`。Copilot 的 provider wire API 能力以实际 binary help 为准，不能仅据较旧网页认定只支持 Chat Completions。

已通过：`cargo test -p condr-core -p condr-server --lib --tests`、新增的真实进程祖先边界回归、`node crates/condr-core/tests/pi_extension.mjs`、`cargo clippy --workspace --all-targets -- -D warnings`（含 GUI 编译检查）。覆盖 native payload 字段、Grok 旧回合过滤、父进程边界、配置保留/重复安装/卸载/外部文件保护、Pi 最终 settled、OMP continuation、多项等待及会话切换。Windows ConPTY 和真实 GUI 尚需 Windows 笔记本验收；Linux 编译检查不能替代它们。

### Windows 验收（2026-09-11）

Windows 11 笔记本、ConPTY、PowerShell Pane，隔离 Server（`CONDR_SOCKET_PATH=condr-verify-pipe`、独立 `CONDR_CONFIG_DIR`、`PI_CODING_AGENT_DIR`，未改用户配置）：

- `cargo test -p condr-core -p condr-server --lib --tests`：275 通过，0 失败；`node crates/condr-core/tests/pi_extension.mjs` 通过。
- Pi 0.85.1（npm 隔离 prefix，`pi.cmd` → node）+ Ollama `qwen3.8:27b`（openai-completions）：`agent hooks install pi` 写入 `extensions/condr-pi.ts`；`agent start` 后 session_start 立即上报 Idle 并带原生 session ID；`agent prompt --wait` Working → Idle；扩展 `ctx.ui.confirm` 触发 Blocked，Escape 后 Idle；`server restart` 用 `pi --session <id>` 恢复同一会话并再次上报 Idle。hook 子进程为 `condr.exe` 绝对路径，经 ConPTY 的 OSC 777 路径全链路通过。
- GUI Agents 页：全部 10 个 agent 行、图标与状态渲染正常；Kimi 显示 Unavailable 且无按钮；Pi 行 Uninstall/Install 按钮实时刷新状态。
- 注意：resume 由 Pane 的 shell 解析 `pi`，与 `agent available` 解析的可执行文件可能不同（本机 PATH 顺序下恢复到了全局 0.80.6）。OMP、Grok、Cursor、Copilot、Antigravity 未在 Windows 安装，未验证。

## Blocked 的 detail 字段来源（ADR 0024）

`condr agent-hook` 只在 `permission-request` / `question-asked` 上附带 `detail`，取值顺序：插件给的 `detail` → 问题文本 → `tool_name` 加命令首行 → 通知 message。Rust 侧统一去控制字符、压空白、截到 200 字符。各 CLI 的原生字段：

| CLI | 权限请求 | 提问 |
| --- | --- | --- |
| Claude Code | `PermissionRequest`：`tool_name`、`tool_input.command`（Bash）；`Notification(permission_prompt)`：`message`，新版本另带 `tool_name` | `PreToolUse(AskUserQuestion)`：`tool_input.questions[0].question` |
| Codex | `PermissionRequest`：`tool_name`、`tool_input.command`（字符串或 argv 数组，数组按空格拼接） | 无 |
| OpenCode | TUI 插件读 `api.state.session.permission(id)[0]` 的 `title` / `permission` / `type` | `api.state.session.question(id)[0]` 的 `questions[0].question` / `question` |
| Pi / OMP | 扩展在 `tool_approval_requested` 记 `toolName`；`ui_prompt_start` 记 `title` | `ask` 工具的 `input.question` / `input.prompt` |
| Grok / Copilot | `Notification` / `notification`：`message`（或 `title`） | 无 |
| Antigravity | 无实际等待事件 | `ask_question`：`toolCall.args.question` |
| Cursor | 无实际等待事件 | 无 |

OpenCode、Pi/OMP 的字段名来自各自公开 API 的类型定义，未逐一在真机上抓包核对；取不到时 `detail` 缺省，状态仍是 `Blocked`。
