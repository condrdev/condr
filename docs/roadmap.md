# Condr Roadmap

> 最近核对：2026-09-22。本文只回答三个问题：现在有什么、接下来做什么、什么留到远期。功能细节以 [ADR](adr/)、[CONTEXT.md](../CONTEXT.md)、[Development Build](development-build.md) 和 [Releases](releases.md) 为准，不在这里重复。

## 定位

Condr 是跨平台的原生多 Agent 终端控制面：一个常驻 Server 拥有终端、Agent 和 Git 运行时，GUI 与 CLI 只是它的客户端。Agent 是终端里的可选进程，Condr 编排、呈现、通知，不自建对话循环。

一句话闭环：选项目（可选 worktree）→ 并行跑多个原生 Agent CLI → 一眼看状态、随时介入 → 关窗口任务继续 → 重连恢复 → 收尾。

判断新事项的三个问题，按顺序问：

1. 它让上面的闭环更快、更可靠或更安全吗？
2. 它跨 Agent、跨平台、能被 GUI 和 CLI 共用吗？
3. 它依赖尚未稳定的协议或权限模型吗？

三问都过才排进近期；只过第一问的放到摩擦记录里等证据。

## 现状（2026-09-22）

单人维护，用 Condr 开发 Condr。MVP 七个阶段（GitHub #1–#16）已于 2026-08-30 关闭；此后两周补齐了发布和日常体验。仓库 2026-09-16 转为公开，尚无外部用户，Issue tracker 当前为空。

**已具备**

| 领域 | 现状 |
| --- | --- |
| 运行时 | 一机一 Server（ADR 0013）；GUI 断开不影响 PTY/Agent；重连先取权威 Bootstrap 再订阅事件；Server 重启按 Session Snapshot 恢复结构并 resume 原生会话；当前 Workspace/Tab 是每个客户端自己的视图，`ActivateWorkspace/ActivateTab` 只是「请大家看这里」的广播（ADR 0021） |
| 终端 | `alacritty_terminal` + 自绘 GPUI 元素；合并视觉流（ADR 0004）；kitty keyboard、OSC 7/52/777、图片粘贴（含远程，ADR 0012）；Server 侧选区（ADR 0008） |
| Agent | 10 种 CLI 的 hook 安装（Kimi 上游不可用）；状态只来自 hooks，经 OSC 777 回写（ADR 0014）；`Blocked` 带 `blocked_on` 说明在等哪个工具/命令或哪个问题，sidebar、OS 通知、CLI JSON 都显示，左侧栏顶部有跨 Device 的「Needs you」列表（ADR 0024）；完成/需输入时 OS 通知 |
| Agent 驱动 | `condr workspace|tab|pane|agent` 全部 JSON 输出；内嵌 Skill（`condr --skill`）；`agent start|prompt|wait` 可跨 Pane 编排；`--device <name>` 直连保存的远程 Device，`device list` / `workspace list --all-devices` 汇总多机（ADR 0022） |
| Git | gix 只读查询 + Managed Worktree；右侧栏 Changes/Files、Diff Tab（对 HEAD）、Preview Tab（ADR 0017/0018） |
| 远程 | `ssh://` 转发远端私有 socket 并可拉起远端 Server（ADR 0015）；`tcp://` 走 `Noise_IKpsk2` 静态密钥 + 一次性 invite（ADR 0011）；Settings 可签 invite、撤销设备 |
| 诊断 | `tracing` 日志按天滚动写入 Log 目录，panic 带 backtrace（ADR 0019）；`condr server status --json` 报 uptime、Workspace/Tab/Pane/Agent 计数、订阅客户端数、最近 warn/error |
| 发布 | nightly + `v*` 正式版共用一条流水线；Linux/macOS/Windows 各两种架构的 desktop 与 headless 产物、校验和、安装脚本；README/CONTRIBUTING/SECURITY/CoC/Issue 模板 |

**代码里确实没有的**

- 可观测性：burst 基准只覆盖 VT 增量、monitor 合并与 shaping 缓存三段，各自在进程内跑；整条 PTY → GPUI 链路的帧率仍靠 Windows 上手工观察。
- 授权：认证即拥有整个 Session；唯一的分级是"只有 Local/SSH 连接能管理 Server"和单一 controller 租约。没有 capability、密钥进 Keychain。invite 里的 `<server key>` 是 `Noise_IK` 握手的输入，Client 只对它加密第一条消息，所以 invite 本身就是信任锚，不需要再做首连指纹确认。
- 代码签名、自动更新、Quickstart 文档、支持矩阵和协议兼容策略。
- 终端内搜索、命令面板、Diff 对 base 分支比较（`GitDiff.against` 已预留）。

**已知限制**（原 Issue #23/#26/#28/#36 已随仓库公开被删除，暂记于此，出现摩擦再立新 Issue）：终端颜色查询（OSC 4/10/11）未回应；kitty keyboard 协议只覆盖已协商的子集；OpenCode 集成缺真机验证；OSC 支持范围没有对照表。

## 环境判断

调查日期 2026-09-17，来源见文末。

- **赛道拥挤，基础功能已成标配。** 公开的 agent orchestrator 列表已超过百个；"并行会话、worktree 隔离、审查改动"是入场券，Condr 三者都有基本形态，靠它们不再能区分自己。
- **厂商自己在做远程和移动端。** Claude Code 的 Remote Control（网页/iOS/Android）和 Codex 的 app-server + relay 都在解决"离开电脑继续控制自己的 Agent"。Condr 近期不在这条线上和厂商竞争；它的远程价值在于跨 Agent、跨机器的一个窗口。SSH/TCP 覆盖有固定地址或已配好 SSH 的机器；两台都在 NAT 后今天没有答案，所以 Peer-to-peer 已排进近期方向（方向 8）。Mobile 仍放远期，Web 端不做。
- **直接对手是 Herdr。** 同一设计谱系（Condr 的 Agent 检测和布局语义源自 herdr 研究），0.9.x 已有多机统一视图、每客户端独立视图、Windows 作 SSH 主机、Apache-2.0。Condr 相对它的差异是：原生 GUI 而非 TUI、Windows 一等公民、Server 自带加密配对而非只靠 SSH、Git 侧栏与 Diff。这些差异要守住，不能被"功能数量"带偏。
- **Agent CLI 的 hooks 越来越能说清"在等什么"。** Claude Code 近期给更多 hook 事件加了 `session_id`，`PermissionRequest` 可 allow/deny，`Notification(permission_prompt)` 带工具名；Pi/OMP 有 `ui_prompt_*`。Condr 已经把这些映射成 `Blocked`，但丢掉了细节。跨 Agent 的"待处理清单"是 Condr 这一层能做、单个厂商不会做的事。
- **外部审批 API 还不存在。** Claude Code 的远程审批仍是 open feature request；Codex 审批只在自己的 app-server 客户端里。Condr 先做"看见并跳转"；"替用户点批准"等厂商开放接口后再接。

## 近期方向

按优先级排列。每项写清为什么、做到哪、什么时候停。

### 1. 让 Server 可诊断

**为什么**：单人 dogfood 也已遇到只能靠猜的故障（重连、hook 配对、PTY 收尾）；有外部用户之前必须先能拿到证据，否则 Issue 无法处理。

已完成（2026-09-22）：日志、panic hook 和 `server status --json` 在现状表里；终端 burst 基准是三个带负载的 `cargo test`，不引入任何依赖：VT 侧 5000 次状态行重绘合并为 125 个稀疏 delta 且末帧带最后的 revision（`condr-core` `terminal/tests/view.rs`），monitor 侧每秒十万次唤醒只按显示节拍发布、顺序不乱、退出必达（`condr-server` `server/tests/unit.rs`），GUI 侧 2000 帧只换 spinner 的整屏只重新 shaping 变化的 cell（`condr-gui` `terminal_element/cache.rs`）。三者都用 `--nocapture` 打印耗时供对比。没有做遥测上报、metrics exporter，也没有端到端帧率测量。

### 2. 多客户端各看各的

已完成（ADR 0021，2026-09-18）：当前 Workspace/Tab 从 Session 移到每个客户端本地；`ActivateWorkspace`/`ActivateTab` 变为广播事件 `Activated`，CLI 的 `focus` 与 `--focus` 用它；`CreateTab` 以 `cwd_from` 指明继承哪个 Pane 的目录。GUI 重启后恢复上次的窗口、侧栏与视图（ADR 0023）。

### 3. Blocked 说清在等什么

已完成（ADR 0024，2026-09-21）：`agent-hook` 从原生 payload 取工具名加命令首行（`Bash: cargo test`）或问题文本，截到 200 字符后随 `permission-request`/`question-asked` 进 OSC 777；OpenCode 插件与 Pi/OMP 扩展传同一个 `detail` 字段。`AgentSnapshot.blocked_on` 只在 `Blocked` 期间存在，离开即清；sidebar Agent 行的第二行、OS 通知正文、`agent list|wait|prompt --wait` 与 `pane list` 的 `blocked_on` 都显示它；左侧栏顶部的「Needs you」列出所有 Device 上的 Blocked Agent，点击落到 Pane。没有做替用户批准，也没有解析屏幕文本补全缺失的 hook。

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
3. 转发（TCP/Noise Device）：协议增加一种 multiplexed 字节流帧，把本机 listener 的连接经 Noise 隧道接到远端 `localhost:<port>`。默认只有配对设备能开转发；若届时已有 observe/control capability，则只允许 `control`。
4. 一键在浏览器打开本机转发地址。

**停在哪**：不做反向转发、不做 UDP、不自动转发所有探测到的端口（默认只列出，点了才转）、不解析进程输出里的 URL。

### 8. Peer-to-peer 连接（NAT 后两台机器直连）

**为什么**：TCP 要固定地址，SSH 要用户已经配好登录，两台都在 NAT 后的机器今天没有答案——这是 remote 价值里唯一被 SSH/TCP 落下的场景。relay 只转发密文、不参与授权，Peer-to-peer 的授权模型与 TCP 完全相同（同一套 Invite / `authorized-clients` / revoke）。

已完成（ADR 0025、0026，2026-09-22 验收）：一台机器一把 device-key；`p2p://<id>` 作为第三种 Endpoint 与 `tcp://`、`ssh://` 并列；Server 是机器唯一的 Peer-to-peer 端点，GUI/CLI 经本地 socket 的 `Tunnel` 帧隧道出去；`[server.p2p] enabled` 与 `--p2p` 是 opt-in 的接受端；自建 relay（`relay.condr.dev`）与 DNS/pkarr（`dns.condr.dev`）已上线，真实 NAT 后两台机器打洞连通；GUI 里 Peer-to-peer 设备可重命名，Settings 显示接受状态；SECURITY.md 写明 relay 与 DNS 能看到什么。指纹与 invite 链接改为 43 字符 base64url。

**停在哪**：不用 n0 的任何基础设施，连兜底也不接；不做用户自建 relay/DNS（现阶段没有配置键）；不做 mDNS 局域网发现；relay 不落盘、无账号。

## 按摩擦记录再做

这些都通过了第一问，但还没有足够的使用证据；出现两次以上真实摩擦再排：

- 终端内搜索；命令面板。
- Diff 对 base 分支比较；Diff Tab 内跳到编辑器。
- `observe` 与 `control` 两级 capability（观察者拿 Bootstrap 和视觉流，不能输入、改布局、读文件；配对时决定，`condr server clients` 可看可改）：触发条件是第二个人开始共用同一个 Server。之后再谈设备密钥进 OS Keychain、密钥轮换、审计日志。不做账号体系、不做公网 listener。
- Kimi hooks（等上游能区分主任务与子 agent 的 Stop）。
- Agent Profile（声明式 manifest 描述可执行文件、参数、图标、检测规则）：目前 10 种 CLI 都是代码内置，第 11 种出现时再抽象。
- 把 `ClientConnection` 拆成独立 crate：只有 GUI 和 `condr` CLI 两个客户端时没有收益。

## 远期

会做，但都排在近期方向之后，且各自有启动条件。条件未到之前不写 ADR、不预留接口。

| 事项 | 启动条件 | 前置 |
| --- | --- | --- |
| 自动更新（GUI 提示新版本 → 下载校验 → 同时替换 Client 与 Server） | 有外部用户；Client/Server 必须同版本这一点已在手动更新中造成抱怨 | 代码签名（方向 5）；`condr server install` 已能原地替换运行中的二进制（ADR 0016） |
| 遥测（opt-in） | 有外部用户，且方向 1 的本地日志已不足以排障 | 诊断数据默认不含终端内容；先有本地日志再谈上报 |
| Mobile Companion（done/blocked 通知、查看、少量动作） | 厂商 Remote Control 覆盖不了的跨 Agent 场景被反复提出 | 一个不依赖 GPUI 的语义 API 适配层；Push 只作提醒，打开后重新拉取 Server 权威状态 |
| 插件 SDK / Agent Profile 市场 | 社区开始提交第三方 Agent 集成或工作流 | 方向 3 之后 Agent Profile 先从代码内置抽成 manifest |
| 团队协作、账号与 RBAC | 出现多人共用一个 Server 的真实需求 | observe/control capability（见摩擦记录）扩展为多用户；审计日志 |
| 编辑器、内置浏览器、任务看板等 IDE 化能力 | dogfood 中反复出现"为了这件事必须离开 Condr" | 逐项立 ADR，不成套引入 |

不做的两件事：**Web 客户端**（Condr 是原生 GUI，不把控制面搬进浏览器；异地需求由 Peer-to-peer 和 Mobile Companion 承接）；**自建对话或工具循环、绑定单一模型厂商**（Condr 编排原生 Agent CLI，不替代它）。

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
