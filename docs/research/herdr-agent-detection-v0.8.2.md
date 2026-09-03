# herdr agent 检测与 Condr 现状对照

> herdr commit：`5158adab10b6dcfea9370782043392f80fa0643c`（本地 checkout `../herdr`，2026-09-03 读取）
>
> 目的：逐项比较 herdr 的 agent 识别 / 状态分类 / 迟滞逻辑与 Condr 现有实现，作为对齐工作的依据。herdr 为 Apache-2.0；移植其 manifest 数据文件需随附 notice。

## 1. herdr 架构

### 进程识别

- 入口 `detect::foreground_job(shell_pid)` 返回 `ForegroundJob { process_group_id, processes[] }`（`src/detect/mod.rs:329-333`）。Linux/macOS 取 tty 前台进程组；Windows 取 shell 后代快照并挑"最顶层 agent 链候选"（`src/platform/windows.rs:982-1015, 1136-1149`）。
- `identify_agent_in_job`（`mod.rs:258-286`）：先看进程组 leader，再对全部成员按 `process_priority` 打分取最优。`normalized_process_name`（`mod.rs:345-382`）依次看 argv0、已知名、node/bun/python/sh/cmd/powershell 包装脚本的参数，最后 cmdline 兜底。
- 环境变量 `HERDR_AGENT=<label>` 优先于进程名（`src/platform/mod.rs:321-329`，`src/pane.rs:596-630`）。
- 节奏（`src/pane.rs:291-299, 727-731`）：主循环 300 ms；已识别 agent 每 5 s 复查，前台组变化立即复查，无前台组 30 s；acquisition 窗口 8 s（前 1.5 s 每 500 ms，之后 2 s）。**清除 agent 需连续 6 次未命中**（`AGENT_MISS_CONFIRMATION_ATTEMPTS`，`pane.rs:1013-1037`）；前台回到 pane shell 时先发一次 `Idle + process_exited`，再清除身份（`pane.rs:345-377`，`agent_detection.rs:305-313`）。

### Manifest

每个 agent 一份 TOML，`include_str!` 内嵌 21 份（`src/detect/mod.rs:98-121`），结构见 `src/detect/manifest.rs:140-198`：

- 顶层：`id, version, min_engine_version, updated_at, aliases[], rules[]`。
- rule：`id`、`state`（idle/working/blocked/unknown）、`priority: i32`、`region`（默认 `whole_recent`）、`visible_idle/visible_blocker/visible_working`、`skip_state_update`；匹配器 `contains[]`（小写子串，对 lowercased 文本）、`regex[]`（Rust regex，对原文整块）、`line_regex[]`（任一行匹配）、嵌套门 `all/any/not`。上限 128 rules / 512 gates / 1024 matchers。
- region（`manifest.rs:1286-1324`）：`osc_title`、`osc_progress`、`whole_recent`、`after_last_prompt_marker`（Codex `›`）、`whole_recent_without_current_prompt_marker`、`prompt_box_body`（最后两条 `─` 之间，Claude 输入框）、`above_prompt_box`、`last_non_empty_above_prompt_box`、`after_last_horizontal_rule`、`bottom_lines(n)`、`bottom_non_empty_lines(n)`、`top_non_empty_lines(n)`（engine v3）。
- 屏幕文本 = 活动屏底部 `rows` 行（视口高度），不受用户滚动影响（`src/pane/terminal.rs:41, 2542-2549, 2682-2695`）。OSC 0/2 标题与 OSC 9 进度按 pane 保留，agent 替换时清空（`pane/terminal.rs:1191-1214`，`pane.rs:380-384`）。
- 加载优先级：override `config_dir/agent-detection/<id>.toml` > 远程缓存 `state_dir/agent-detection/remote/<id>.toml` > 内嵌（`manifest.rs:599-696, 1129-1135`）。`manifest_update.rs` 每 30 min 拉 `https://herdr.dev/agent-detection/index.toml`，版本单调递增，拒绝降级和同版本改内容，256 KB 上限，原子写，`MANIFEST_ENGINE_VERSION = 3`。

### 状态

`Idle`（agent 结束、prompt 可见）、`Working`、`Blocked`（需人工输入）、`Unknown`（非 agent 或未识别）（`mod.rs:10-21`）。`AgentDetection` 附加 `skip_state_update / visible_idle / visible_blocker / visible_working`（`mod.rs:24-40`）。

## 2. herdr 分类流水线（`src/pane.rs:700-980`）

1. 新 agent 识别后 **3 s startup grace**，期间不扫屏（`AGENT_STARTUP_GRACE_WINDOW`）。
2. Idle 且 `detection_content_seq` 未变则跳过读屏（`agent_detection.rs:91-103`）。
3. `should_skip_state_update`：命中 `skip_state_update` 规则（transcript viewer、model picker）则保持旧状态并清 pending idle。
4. `detect_agent_with_osc`：评估全部 rule，**取 priority 最高的命中**，不是固定 blocked > working > idle（`manifest.rs:446-478`）。无命中且已知 agent → 兜底 `Idle`。
5. 进程退出 → 强制 `Idle, visible_idle=true`。
6. 迟滞（`agent_detection.rs:5-13, 39-77`）：只有 `Working → 无 visible_idle/visible_blocker 的 Idle` 被扣留；扣留期间循环改为 100 ms（`AGENT_PENDING_IDLE_RECHECK`）；连续 **3 次**确认（`AGENT_PENDING_IDLE_CONFIRMATIONS`）或超过 **700 ms**（`AGENT_PENDING_IDLE_CAP`）放行；`visible_idle` 直接绕过。
7. 发布条件（`agent_detection.rs:140-168`）：state 或任一 visible 标志变化、agent 变化、进程退出，或 blocker 持续可见且距上次刷新 ≥ **800 ms**（`STABLE_VISIBLE_SIGNAL_REFRESH`）。

### Claude manifest 关键规则（`manifests/claude.toml`，priority 降序）

- `osc_title_working` 1100，`osc_title`：`'^[\x{2800}-\x{28FF}\x{25D0}-\x{25D3}] '`
- `transcript_viewer` 1000 skip，`bottom_non_empty_lines(3)`：`"showing detailed transcript"` + any(`"ctrl+o","to toggle"` | `"ctrl+e","show all"` | `"ctrl+e","collapse"` | `"↑↓ scroll"` | `"? for shortcuts"`)
- `live_blocked_form` 980，`after_last_horizontal_rule`：`"esc to cancel"` + any(`"enter to confirm"` | `"enter to select"` + 导航提示 `"tab/arrow keys to navigate"|"arrow keys to navigate"|"arrows to navigate"|"↑/↓ to navigate"|"↑↓ to navigate"`)
- `dynamic_workflow_prompt` 980：`"run a dynamic workflow?","esc to cancel"`；`mcp_elicitation_prompt` 980：`"esc to cancel"` + `'(?i)^\s*MCP server ["\x{201C}].+["\x{201D}] requests your input\s*$'` + Accept/Decline 行
- `btw_overlay_working` 975，`bottom_non_empty_lines(5)`：`'^\s*/btw(?:\s|$)'` 且 `'(?i)esc to close\s*$'`
- `live_turn_working` 970，`bottom_non_empty_lines(12)`：`'^\s*[⏸⏵].*esc to interrupt(?:\s|·|$)'` 或 `'^\s*[\x{002A}\x{00B7}\x{2722}\x{2736}\x{273B}\x{273D}]\s+\S.*…(?:\s+\(\d+[smh](?:\s|·)|\s*$)'`
- `background_shell_working` 965：`'^\s*[⏸⏵].*·\s+[1-9]\d*\s+shells?\s+(?:·|$)'`；`background_agents_working` 965（`last_non_empty_above_prompt_box`）：`'^\s*[*·✢✶✻✽]\s+Waiting for [1-9]\d* background agents? to finish\s*$'`；`background_mcp_task_working` 965（多行 regex `…MCP tasks? still running`）
- `live_prompt_box` **idle** 950，`prompt_box_body`：`'^\s*❯'`，not `"enter to select"|"esc to cancel"|"tab/arrow keys"|"arrow keys to navigate"|"↑/↓ to navigate"`
- `model_picker_menu` 900 skip：`"select model","enter to set as default","esc to cancel"`
- `bash_permission_prompt` 850，`whole_recent`：`"do you want to proceed?"` + any(`"bash command"|"bash("|"contains expansion"|"tab to amend"|"ctrl+e to explain"`) + yes/no 行 `'(?i)^\s*❯?\s*yes\b'|'(?i)^\s*1\.\s*yes\b'|'(?i)^\s*2\.\s*no\b'`
- `generic_permission_prompt` 840，`after_last_horizontal_rule`：`"do you want to proceed?","esc to cancel"` + 编号 yes/no 行
- `legacy_no_prompt_blocker` 300：`do you want to`/`would you like to` + `yes|❯`、`"waiting for permission"`、`"do you want to allow this connection?"`、`"tab to amend"`、`"ctrl+e to explain"`、`"review your answers"`、`"skip interview and plan immediately"`；not `'(?m)^\s*❯\s*$'`
- `osc_title_idle` 250：`'^\x{2733} '`；`osc_progress_idle` 250：`'^4;0'`

### Codex manifest（`manifests/codex.toml`）

- `osc_title_blocked` 1100：`"Action Required"`；`osc_title_working` 1050：`'(?:^| )[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏](?: |$)'`
- `transcript_viewer` 1000 skip，`after_last_prompt_marker`：`"↑/↓ to scroll","pgup/pgdn to","home/end to jump","q to quit"` + `"esc to edit prev"|"esc/← to edit prev"`
- `trust_directory` 950，`top_non_empty_lines(20)`：`'\A> You are in [^\r\n]+(?:\r?\n|$)'` + `'(?s)Do\s+you\s+trust\s+the\s+contents\s+of\s+this\s+directory\?'`
- `live_strong_blocker` 900，`after_last_prompt_marker`：`"press enter to confirm or esc to cancel"|"enter to submit answer"|"enter to submit all"|"allow command?"`
- `weak_blocker` 600，`whole_recent_without_current_prompt_marker`：`"[y/n]"|"yes (y)"|do you want to/would you like to + yes|❯`
- `screen_working_fallback` 500，`bottom_non_empty_lines(3)`：`'^[•◦]\s+Working \([^)]*esc to interrupt\)(?: · .*)?$'`，not `"■ Conversation interrupted"`
- `osc_title_idle` 100：`'\S'` 且 not 上述 spinner / `"Action Required"`

## 3. Condr 现状

- 仅 `AgentKind::{Claude, Codex}`；`AgentState` 四值同 herdr（`crates/condr-core/src/agent.rs:3-24`）。GUI 侧 `AgentTracker` 追加 `Done`（未被看到的完成）（`agent.rs:53-93`，`condr-gui/src/app/events.rs:657-667`）。
- 识别 `identify_agent_process`（`agent.rs:95-169`）：名字 → argv[0] → cmd `/c|/k` → node/bun/python/pwsh 任一参数。进程来源：Unix 用 PTY `process_group_leader()` + `getpgid` 成员（`terminal/process.rs:309-340`）；Windows 用 shell 后代 + `root_agent`（`process.rs:342-383`）。sysinfo 快照 1 s，kind 复查 5 s（`terminal.rs:49-51`，`runtime.rs:1114-1124`）。**单次未命中即 `None`** → `AgentChanged { agent: None }` → GUI 删除 tracker（`terminal_monitor.rs:295-316`）。无 env 提示。
- 屏幕文本 `bottom_text`：活动屏全部行，去尾部空行（`runtime.rs:1149-1176`），与 herdr 等价。无 OSC 标题 / 进度输入。
- 分类 `classify_agent`（`agent.rs:115-127`）：transcript → 返回 `previous`；否则 blocked → working → idle 固定顺序，全文 lowercase 子串匹配。Claude working = 最后 5 个非空行中含 `"esc to interrupt"` 且不以 `›/❯` 开头，且其后无 prompt / `conversation interrupted`；Codex working = `• working (` / `◦ working (` + `esc to interrupt`。Blocked 公共：`[y/n]`、`waiting for permission`、`do you want to/would you like to` + `yes`；Claude 加 `do you want to proceed?`、`esc to cancel` + `enter to confirm|select`；Codex 加 `action required`、`allow command?`、`press enter to confirm or esc to cancel`、`enter to submit answer|all`。自研 prompt-marker 切分（`agent.rs:259-302`）。
- 节奏：server `AGENT_SCAN_INTERVAL = 500 ms`，仅在终端有输出 nudge 时扫描（`server.rs:47`，`terminal_monitor.rs:133-175`）；快照变化即发布。无迟滞、startup grace、visible 标志。

## 4. 差距清单

| 项 | herdr | Condr | 用户可见影响 | 对齐规模 |
| --- | --- | --- | --- | --- |
| 规则引擎 | TOML manifest + priority + region + regex/gates | 硬编码小写子串 | CLI 改 UI 时 Condr 要改代码 | 大（移植 manifest 系统，可去掉远程更新） |
| Claude working | `[⏸⏵]` 前缀或 spinner + `…`，12 行，另有 btw / 后台 shell / 后台 agent / MCP task 四种 | 任意 `esc to interrupt`，5 行 | 等待后台 agent/MCP 时显示 idle；旧式文本可能误判 working | 中（走 manifest 则免费） |
| Claude idle | `prompt_box_body` 内 `❯` 且非菜单，带 `visible_idle` | 纯兜底 | 接近；herdr 可绕过扣留 | 小 |
| Claude blocked 集合 | 含 dynamic workflow、MCP elicitation、`review your answers`、`skip interview…`、`allow this connection?`、`tab to amend`、`ctrl+e to explain`，区域 `after_last_horizontal_rule` | 子集 | 这些对话框 Condr 判 idle | 小 |
| OSC title / progress | Codex blocked/working、Claude working/idle 的最高优先级信号 | 无；Codex `"action required"` 只在屏幕文本上找，实际在标题里 | Codex 权限弹窗、Claude 半圆 spinner 识别更慢或漏掉 | 中（标题已通过 notices 上报，需喂给分类器；OSC 9 需新增） |
| Codex 区域语义 | `weak_blocker` 用 `whole_recent_without_current_prompt_marker`；transcript 需 4 串 + `esc to edit prev`；`trust_directory` 看顶部 20 行 | 自研切分；transcript 2 串；无 trust | 首次进入目录的 trust 弹窗判 idle | 小/中 |
| 迟滞 | Working → plain Idle 扣留，100 ms 复查，3 次或 700 ms | 无 | 回合间重绘导致 Working/Idle/Done 闪烁与误报完成 | 小 |
| Startup grace 3 s | 有 | 无 | 启动画面 / 残留内容先判 idle 再切 working | 小 |
| Agent 存在性 | 6 次未命中才清除，清除前先发 `Idle + process_exited` | 一次未命中即清 | 进程表抖动使徽标消失；正常退出时 `Done` 丢失 | 小 |
| 扫描节奏 | 300 ms 常驻 + content_seq 跳读 | 500 ms 且仅输出触发 | 进程退出、OSC 变化等无输出事件滞后 | 小 |
| visible_* 与 800 ms 刷新 | 用于通知 / hook 仲裁 | 无 | Condr 暂无消费者 | 中，需求驱动 |
| Agent 覆盖 | 23 种 | 2 种 | — | 大，但移植 manifest 后直接复用 herdr TOML |

## 5. Condr 更优或不值得对齐之处

- `AgentTracker::Done` 是 GUI 需要的呈现层概念，herdr 无同形物，保留；须配合"先发 `process_exited` 再清身份"，否则 Done 会丢。
- Windows 后代树 + `start_time` 身份校验 + `root_agent`（`process.rs:342-383`）与 herdr 等价，不需改。
- 远程 manifest 自动更新（herdr.dev catalog、版本仲裁，约 1000 行）不值得移植：引入对 herdr 服务器的运行时依赖。内嵌 TOML + 本地 override 文件即可。
- `explain` 诊断输出与 hook / lifecycle authority 仲裁是 herdr 集成体系的一部分，Condr 暂无对应需求。
- Condr 的 transcript "返回 previous" 与 herdr `skip_state_update` 效果一致，形式更简单。

## 6. 建议路径

1. 先做全部"小"项（server 侧约 100–150 行）：迟滞、startup grace、6 次未命中确认、退出前发 Idle、标题喂给分类器、扫描改常驻 300 ms。
2. 再决定是否移植 manifest 引擎（去掉远程更新与 explain，预估 600–800 行 + `regex` 依赖），内嵌 herdr 的 TOML 并随附 Apache-2.0 notice。
