# Condr Roadmap

> 最近核对：2026-09-19。本文只回答三个问题：现在有什么、接下来做什么、什么留到远期。功能细节以 [ADR](adr/)、[CONTEXT.md](../CONTEXT.md)、[Development Build](development-build.md) 和 [Releases](releases.md) 为准，不在这里重复。

## 定位

Condr 是跨平台的原生多 Agent 终端控制面：一个常驻 Server 拥有终端、Agent 和 Git 运行时，GUI 与 CLI 只是它的客户端。Agent 是终端里的可选进程，Condr 编排、呈现、通知，不自建对话循环。

一句话闭环：选项目（可选 worktree）→ 并行跑多个原生 Agent CLI → 一眼看状态、随时介入 → 关窗口任务继续 → 重连恢复 → 收尾。

判断新事项的三个问题，按顺序问：

1. 它让上面的闭环更快、更可靠或更安全吗？
2. 它跨 Agent、跨平台、能被 GUI 和 CLI 共用吗？
3. 它依赖尚未稳定的协议或权限模型吗？

三问都过才排进近期；只过第一问的放到摩擦记录里等证据。

## 现状（2026-09-19）

单人维护，用 Condr 开发 Condr。MVP 七个阶段（GitHub #1–#16）已于 2026-08-30 关闭；此后两周补齐了发布和日常体验。仓库 2026-09-16 转为公开，尚无外部用户，Issue tracker 当前为空。

**已具备**

| 领域 | 现状 |
| --- | --- |
| 运行时 | 一机一 Server（ADR 0013）；GUI 断开不影响 PTY/Agent；重连先取权威 Bootstrap 再订阅事件；Server 重启按 Session Snapshot 恢复结构并 resume 原生会话；当前 Workspace/Tab 是每个客户端自己的视图，`ActivateWorkspace/ActivateTab` 只是「请大家看这里」的广播（ADR 0021） |
| 终端 | `alacritty_terminal` + 自绘 GPUI 元素；合并视觉流（ADR 0004）；kitty keyboard、OSC 7/52/777、图片粘贴（含远程，ADR 0012）；Server 侧选区（ADR 0008） |
| Agent | 10 种 CLI 的 hook 安装（Kimi 上游不可用）；状态只来自 hooks，经 OSC 777 回写（ADR 0014）；完成/需输入时 OS 通知 |
| Agent 驱动 | `condr workspace|tab|pane|agent` 全部 JSON 输出；内嵌 Skill（`condr --skill`）；`agent start|prompt|wait` 可跨 Pane 编排；`--device <name>` 直连保存的远程 Device，`device list` / `workspace list --all-devices` 汇总多机（ADR 0022） |
| Git | gix 只读查询 + Managed Worktree；右侧栏 Changes/Files、Diff Tab（对 HEAD）、Preview Tab（ADR 0017/0018） |
| 远程 | `ssh://` 转发远端私有 socket 并可拉起远端 Server（ADR 0015）；`tcp://` 走 `Noise_IKpsk2` 静态密钥 + 一次性 invite（ADR 0011）；Settings 可签 invite、撤销设备 |
| 诊断 | `tracing` 日志按天滚动写入 Log 目录，panic 带 backtrace（ADR 0019）；`condr server status --json` 报 uptime、Workspace/Tab/Pane/Agent 计数、订阅客户端数、最近 warn/error |
| 发布 | nightly + `v*` 正式版共用一条流水线；Linux/macOS/Windows 各两种架构的 desktop 与 headless 产物、校验和、安装脚本；README/CONTRIBUTING/SECURITY/CoC/Issue 模板 |

**代码里确实没有的**

- 可观测性：burst 合并、最终帧不丢、未变 cell 不重 shaping 各有一个单元测试，但没有带负载的可重复基准；AGENTS.md 要求的性能验收仍靠手工观察。
- 授权：认证即拥有整个 Session；唯一的分级是"只有 Local/SSH 连接能管理 Server"和单一 controller 租约。没有 capability、首连指纹确认、密钥进 Keychain。
- 代码签名、自动更新、Quickstart 文档、支持矩阵和协议兼容策略。
- 终端内搜索、命令面板、Diff 对 base 分支比较（`GitDiff.against` 已预留）。

**已知限制**（原 Issue #23/#26/#28/#36 已随仓库公开被删除，暂记于此，出现摩擦再立新 Issue）：终端颜色查询（OSC 4/10/11）未回应；kitty keyboard 协议只覆盖已协商的子集；OpenCode 集成缺真机验证；OSC 支持范围没有对照表。

## 环境判断

调查日期 2026-09-17，来源见文末。

- **赛道拥挤，基础功能已成标配。** 公开的 agent orchestrator 列表已超过百个；"并行会话、worktree 隔离、审查改动"是入场券，Condr 三者都有基本形态，靠它们不再能区分自己。
- **厂商自己在做远程和移动端。** Claude Code 的 Remote Control（网页/iOS/Android）和 Codex 的 app-server + relay 都在解决"离开电脑继续控制自己的 Agent"。Condr 近期不在这条线上和厂商竞争；它的远程价值在于跨 Agent、跨机器的一个窗口，现阶段 SSH/TCP 够用，Relay 和 Mobile 放到远期，Web 端不做。
- **直接对手是 Herdr。** 同一设计谱系（Condr 的 Agent 检测和布局语义源自 herdr 研究），0.9.x 已有多机统一视图、每客户端独立视图、Windows 作 SSH 主机、Apache-2.0。Condr 相对它的差异是：原生 GUI 而非 TUI、Windows 一等公民、Server 自带加密配对而非只靠 SSH、Git 侧栏与 Diff。这些差异要守住，不能被"功能数量"带偏。
- **Agent CLI 的 hooks 越来越能说清"在等什么"。** Claude Code 近期给更多 hook 事件加了 `session_id`，`PermissionRequest` 可 allow/deny，`Notification(permission_prompt)` 带工具名；Pi/OMP 有 `ui_prompt_*`。Condr 已经把这些映射成 `Blocked`，但丢掉了细节。跨 Agent 的"待处理清单"是 Condr 这一层能做、单个厂商不会做的事。
- **外部审批 API 还不存在。** Claude Code 的远程审批仍是 open feature request；Codex 审批只在自己的 app-server 客户端里。Condr 先做"看见并跳转"；"替用户点批准"等厂商开放接口后再接。

## 近期方向

按优先级排列。每项写清为什么、做到哪、什么时候停。

### 1. 让 Server 可诊断

**为什么**：单人 dogfood 也已遇到只能靠猜的故障（重连、hook 配对、PTY 收尾）；有外部用户之前必须先能拿到证据，否则 Issue 无法处理。

**做到哪**：日志、panic hook 和 `server status --json` 已在现状表里。剩下一项：一个可重复跑的终端 burst 基准（合并到最新 revision、最终帧不丢、未变 cell 不重新 shaping），放进 `cargo test`，不引入 criterion 之外的东西。

**停在哪**：不做遥测上报、不做 metrics exporter。

### 2. 多客户端各看各的

已完成（ADR 0021，2026-09-18）：当前 Workspace/Tab 从 Session 移到每个客户端本地；`ActivateWorkspace`/`ActivateTab` 变为广播事件 `Activated`，CLI 的 `focus` 与 `--focus` 用它；`CreateTab` 以 `cwd_from` 指明继承哪个 Pane 的目录。GUI 重启后从第一个 Workspace 打开，记住上次视图留到有摩擦再做。

### 3. Blocked 说清在等什么

**为什么**：sidebar 现在只有一个圆点和一条 OS 通知；用户要切过去看屏幕才知道是权限请求还是提问。这正是 hooks 能给而屏幕识别给不了的信息。

**做到哪**：

- `agent-hook` 把 `permission-request`/`question-asked` 里的工具名或问题摘要带进 OSC 777 事件，`AgentChanged` 携带它。
- sidebar 的 Agent 行、OS 通知、`condr agent list`/`wait` 的返回都显示这段摘要。
- 一个跨 Workspace 的"需要你"列表，点击即跳转到对应 Pane。

**停在哪**：不代替用户批准；不解析屏幕文本补全缺失的 hook。

### 4. 远程安全边界（对外宣传 remote 之前）

**为什么**：TCP 配对已经能用，但认证等于全权。在只有自己两台机器时够用；一旦有第二个人或不受信网络，这就是最先被问的问题。

**做到哪**（按顺序，前两项先做）：

- 首次连接在 GUI 显示 Server 指纹并要求确认，而不是只写在 invite 里。
- `observe` 与 `control` 两级 capability：观察者拿 Bootstrap 和视觉流，不能输入、改布局、读文件；capability 在配对时决定，`condr server clients` 可看可改。
- 之后再谈：设备密钥进 OS Keychain、密钥轮换、审计日志。

**停在哪**：不做账号体系、不做多用户、不做公网 listener。

### 5. 对外发布门（等发布决定）

发布基础设施已就绪，剩下的全是"有外部用户才值得付的成本"，一起做，不拆开：

- 代码签名：Windows 安装器与 EXE（SmartScreen），macOS 签名与公证。需要证书，是流程决定不是脚本改动。
- Quickstart、支持矩阵（现有构建：Linux x86_64/arm64、Windows x86_64、macOS x86_64/arm64，不多承诺）、远程安全边界说明、故障排查。文档站放在 `condr-website`。
- 冻结 `PROTOCOL_VERSION` 的语义并写下兼容策略：公开前仍保持 `1`，不做兼容层。
- 第一批外部 dogfood 跑同一个 canonical workflow：多 worktree/Agent、断开 GUI、重连、Server 重启。

进入条件是一句明确的"邀请外部用户"，退出条件是新机器按文档能装、能连、能恢复，并且方向 1 的日志能支撑排障。

### 6. 本机作为编排端驾驭远程 Device

已完成（ADR 0022，2026-09-19）：配置读写底座在 `condr-core`，`[[client.servers]]` 的 `SavedServer` 在 `condr-server` 的 `Endpoint` 旁边，CLI 与 GUI 共用；`condr --device <name>`（或 `CONDR_DEVICE`）把 `workspace|tab|pane|agent` 命令直连到保存的 Device，每次调用独立建连；`condr device list` 报可达性与 Workspace 数，`workspace list --all-devices` 汇总并列出 `unreachable`；内嵌 Skill 写明跨 Device 用法；SSH Device 在 Unix 上用 OpenSSH `ControlMaster/ControlPersist` 复用连接，尊重用户已配置的 `ControlPath`。没有做 Server 联邦、跨 Device 迁移或任务队列。

### 7. 探测 Workspace 里的端口并转发到本机

**为什么**：Agent 在远端 Workspace 里起了前端 dev server，用户要在本机浏览器看效果，今天只能自己开 `ssh -L`。"看见 Agent 做出来的东西"是介入/收尾闭环的一部分；Server 已有 per-OS 进程表（Agent 检测用），加一步"该进程树在监听哪些端口"是同一条路。

**做到哪**（按顺序）：

1. 探测：Server 在进程表变化时查 Pane 进程树的监听端口（Linux `/proc/net/tcp*`，macOS `proc_pidfdinfo`，Windows `GetExtendedTcpTable`），作为 Pane 状态经事件下发；GUI 在 Workspace/Pane 上显示端口徽标，`condr pane|workspace` JSON 携带。
2. 转发（SSH Device）：点击端口即在本机开 listener，另起一条 `ssh -N -L` 到远端 `localhost:<port>`；`condr port forward <device> <port>` 同义。
3. 转发（TCP/Noise Device）：协议增加一种 multiplexed 字节流帧，把本机 listener 的连接经 Noise 隧道接到远端 `localhost:<port>`。需要方向 4 的 capability：只有 `control` 能开转发。
4. 一键在浏览器打开本机转发地址。

**停在哪**：不做反向转发、不做 UDP、不自动转发所有探测到的端口（默认只列出，点了才转）、不解析进程输出里的 URL。

## 按摩擦记录再做

这些都通过了第一问，但还没有足够的使用证据；出现两次以上真实摩擦再排：

- 终端内搜索；命令面板。
- Diff 对 base 分支比较；Diff Tab 内跳到编辑器。
- Kimi hooks（等上游能区分主任务与子 agent 的 Stop）。
- Agent Profile（声明式 manifest 描述可执行文件、参数、图标、检测规则）：目前 10 种 CLI 都是代码内置，第 11 种出现时再抽象。
- 把 `ClientConnection` 拆成独立 crate：只有 GUI 和 `condr` CLI 两个客户端时没有收益。

## 远期

会做，但都排在近期方向之后，且各自有启动条件。条件未到之前不写 ADR、不预留接口。

| 事项 | 启动条件 | 前置 |
| --- | --- | --- |
| 自动更新（GUI 提示新版本 → 下载校验 → 同时替换 Client 与 Server） | 有外部用户；Client/Server 必须同版本这一点已在手动更新中造成抱怨 | 代码签名（方向 5）；`condr server install` 已能原地替换运行中的二进制（ADR 0016） |
| Relay（rendezvous + 加密转发，不落盘终端内容） | 远程用户经常因 NAT/防火墙无法直连，且明确不愿配 SSH/Tailscale | 方向 4 的 capability 模型；Relay 只转发 opaque stream，不能绕过 Server 授权 |
| 遥测（opt-in） | 有外部用户，且方向 1 的本地日志已不足以排障 | 诊断数据默认不含终端内容；先有本地日志再谈上报 |
| Mobile Companion（done/blocked 通知、查看、少量动作） | 厂商 Remote Control 覆盖不了的跨 Agent 场景被反复提出 | 方向 4；一个不依赖 GPUI 的语义 API 适配层；Push 只作提醒，打开后重新拉取 Server 权威状态 |
| 插件 SDK / Agent Profile 市场 | 社区开始提交第三方 Agent 集成或工作流 | 方向 3 之后 Agent Profile 先从代码内置抽成 manifest |
| 团队协作、账号与 RBAC | 出现多人共用一个 Server 的真实需求 | 方向 4 的 capability 扩展为多用户；审计日志 |
| 编辑器、内置浏览器、任务看板等 IDE 化能力 | dogfood 中反复出现"为了这件事必须离开 Condr" | 逐项立 ADR，不成套引入 |

不做的两件事：**Web 客户端**（Condr 是原生 GUI，不把控制面搬进浏览器；异地需求由 Relay 和 Mobile Companion 承接）；**自建对话或工具循环、绑定单一模型厂商**（Condr 编排原生 Agent CLI，不替代它）。

## 节奏

- 日常修复合入 `main` 即进入次日 Nightly；需要固定版本时按 [Releases](releases.md) 打 `v*` 标签。
- 每个近期方向落地前先写 ADR，落地时同步更新 CONTEXT.md、AGENTS.md 与本文的现状表。
- 本文每次重大方向变化时整体重写，不追加"已完成"段落；历史看 git log 和 ADR。

## 调查来源

- Herdr 0.9.0 / 0.9.1 变更：[CHANGELOG](https://github.com/ogulcancelik/herdr/blob/master/CHANGELOG.md)
- 生态盘点：[awesome-agent-orchestrators](https://github.com/andyrewlee/awesome-agent-orchestrators)、[cmux 对比页](https://cmux.com/compare)、[Nimbalyst 开源 agent workspace 对比](https://nimbalyst.com/blog/open-source-agent-workspace-alternatives-2026/)、[Superset](https://superset.sh/)
- Claude Code：[Changelog](https://code.claude.com/docs/en/changelog)、[远程审批 feature request #38299](https://github.com/anthropics/claude-code/issues/38299)、[permission_prompt 通知细节 #32952](https://github.com/anthropics/claude-code/issues/32952)
- Codex：[App Server 协议指南](https://codex.danielvaughan.com/2026/04/15/codex-app-server-complete-guide/)
- 本仓库既有研究：[Superset 状态监测](research/superset-agent-status.md)、[新增 Agent hooks](research/additional-agent-hooks.md)
