# Condr Roadmap

> 最近核对：2026-10-06。本文只回答三个问题：现在有什么、接下来做什么、什么留到远期。功能细节以 [ADR](adr/)、[CONTEXT.md](../CONTEXT.md)、[Development Build](development-build.md) 和 [Releases](releases.md) 为准，不在这里重复。

## 定位

Condr 是跨平台的原生多 Agent 终端控制面：一个常驻 Server 拥有终端、Agent 和 Git 运行时，GUI 与 CLI 只是它的客户端。Agent 是终端里的可选进程，Condr 编排、呈现、通知，不自建对话循环。

一句话闭环：选项目（可选 worktree）→ 并行跑多个原生 Agent CLI → 一眼看状态、随时介入 → 关窗口任务继续 → 重连恢复 → 收尾。

直接对手是 Herdr。Condr 相对它的差异是原生 GUI 而非 TUI、Windows 一等公民、Server 自带加密配对而非只靠 SSH、Git 侧栏与 Diff。这些差异要守住，不能被功能数量带偏。

判断新事项的三个问题，按顺序问：

1. 它让上面的闭环更快、更可靠或更安全吗？
2. 它跨 Agent、跨平台、能被 GUI 和 CLI 共用吗？
3. 它依赖尚未稳定的协议或权限模型吗？

三问都过才排进近期；只过第一问的放到摩擦记录里等证据。

## 现状（2026-10-05）

单人维护，用 Condr 开发 Condr。MVP 七个阶段（GitHub #1–#16）已于 2026-08-30 关闭。0.1.0 于 2026-09-23 作为公开预览发布，到 2026-10-05 已发到 0.1.7。Windows 和 macOS 的新机器已经只看文档走通了安装、连接远程 Device、多 worktree 与多 Agent、断开 GUI 后重连和 Server 重启。官网文档有中英两版，已没有施工中的页面。仓库尚无外部用户反馈，Issue tracker 为空。

**已具备**

| 领域 | 现状 |
| --- | --- |
| 运行时 | 一机一 Server（ADR 0013）；GUI 断开不影响 PTY 和 Agent；重连先取权威 Bootstrap 再订阅事件；Server 重启按 Session Snapshot 恢复结构并 resume 原生会话；当前 Workspace 和 Tab 是每个客户端自己的视图（ADR 0021），GUI 重启后恢复窗口、侧栏与视图（ADR 0023）；Windows 上每个 Terminal 的进程归属由 Job Object 决定（ADR 0030），Server 与 GUI 以桌面 shell 给的进程状态运行（ADR 0031） |
| 终端 | `alacritty_terminal` + 自绘 GPUI 元素；合并视觉流（ADR 0004）；kitty keyboard、OSC 7/52/777、图片粘贴（含远程，ADR 0012）；Server 侧选区（ADR 0008） |
| Agent | 10 种 CLI 的 hook 安装（Kimi 上游不可用）；状态只来自 hooks，经 OSC 777 回写（ADR 0014）；`Blocked` 带 `blocked_on`，sidebar、OS 通知、CLI JSON 和左侧栏顶部跨 Device 的「Needs you」列表都显示它（ADR 0024）；完成或需输入时发 OS 通知 |
| Agent 驱动 | `condr workspace|tab|pane|agent` 全部 JSON 输出；内嵌 Skill（`condr --skill`）；`agent start|prompt|wait` 可跨 Pane 编排；`--device <name>` 直连保存的远程 Device，`device list` 与 `workspace list --all-devices` 汇总多机（ADR 0022） |
| Git | gix 只读查询 + Managed Worktree；右侧栏 Changes 和 Files、Diff Tab、Preview Tab（ADR 0017/0018）；Changes 与 Diff Tab 可在对 HEAD 和对 base 分支之间切换，Condr 新建的 worktree 记下分叉时父 Workspace 的分支（ADR 0034）；两种 Tab 用 syntect 高亮，主题和字号在 Settings 里选（ADR 0032） |
| 远程 | `ssh://` 转发远端私有 socket 并可拉起远端 Server（ADR 0015）；`tcp://` 走 `Noise_IKpsk2` 静态密钥 + 一次性 invite（ADR 0011）；`p2p://` 经 iroh 打洞或自建 relay 连 NAT 后的机器（ADR 0025/0026）；Settings 可签 invite、撤销设备；新建 Workspace 时可浏览远程 Device 的目录；机器唤醒后 GUI 重新探测每条连接 |
| 诊断 | `tracing` 日志按天滚动写入 Log 目录，panic 带 backtrace（ADR 0019）；`condr server status --json` 报 uptime、各类计数、订阅客户端数和最近的 warn/error；三个带负载的 `cargo test` 分别量 VT 增量合并、monitor 按显示节拍发布和 GUI shaping 缓存 |
| 发布 | nightly 与 `v*` 正式版共用一条流水线，产出 Linux/macOS x86_64/arm64 与 Windows x86_64 的 desktop 与 headless 包、校验和、安装脚本；macOS 包以 Developer ID 签名并公证；release notes 由 git-cliff 从 `feat`/`fix` 提交生成；GUI 按渠道检查新版本并提示，不自动安装（ADR 0029），更新后提示重启本机旧版 Server；协议按字段号演进，CI 对 `proto/` 跑 `buf lint` 和 `buf breaking`（ADR 0027/0028） |
| 文档 | condr.dev 中英双语：入门、使用、参考、帮助四组页面，含 CLI、配置与快捷键参考，故障排查、安全模型、架构说明和 Windows SmartScreen 放行步骤 |

**代码里确实没有的**

- 可观测性：上面三个基准各自在进程内跑，整条 PTY → GPUI 链路的帧率仍靠 Windows 上手工观察。
- 授权：认证即拥有整个 Session；唯一的分级是"只有 Local/SSH 连接能管理 Server"和单一 controller 租约。没有 capability，密钥也没有进 Keychain。invite 里的 `<server key>` 是 `Noise_IK` 握手的输入，Client 只对它加密第一条消息，所以 invite 本身就是信任锚，不需要再做首连指纹确认。
- Windows 代码签名（SmartScreen）。
- 终端内搜索。
- 协议兼容窗口的长度尚未决定。ADR 0028 原定在首个正式版时决定；Releases 目前只写了 Client 与 Server 不承诺跨 build 兼容，需要一起更新。

**已知限制**（出现摩擦再立 Issue）：

- 终端颜色查询（OSC 4/10/11）回复的是 Server 内置的默认深色配色，不是 GUI 当前选的终端配色。因此选了浅色配色时，按背景色判断明暗的程序会判断成深色。
- kitty keyboard 协议可协商（Claude Code 请求标志 5，Codex 请求 7），按下、重复与松开都会上报；但不上报单独的修饰键、小键盘键和标志 8 下普通文字键的松开，也不支持 xterm modifyOtherKeys。
- OpenCode、Cursor 和 Antigravity 还没跑过真实会话；OMP、Grok 和 Copilot 只在 Linux 上验证过。OSC 支持范围没有对照表。

## 近期方向

按优先级排列。每项写清为什么、做到哪、什么时候停。

### 1. Agent 集成收拢与扩充

**为什么**：Condr 只做官方支持的 Agent，未支持的 CLI 通过 Issue 或 PR 加入（ADR 0035）。因此加一种 Agent 必须容易写、容易审。现在一种 Agent 的知识分散在 `condr-core`、协议和 GUI 的十来处，加一种要逐处修改。其中 `reports_at_startup()` 这类带默认分支的函数，漏改了也能编译通过。另外，Settings 里显示的 Unavailable 没有说清 Condr 对这个 Agent 承诺了什么。

**做到哪**（按顺序，见 ADR 0035）：

1. 给 10 种 Agent 生成的 hook 文件补快照测试，逐字固定下来。
2. 重构：每种 Agent 一个模块，一份所有字段必填的规格。重构后快照必须一字不差。
3. 两级支持：Settings 和文档区分“完整支持”和“仅识别”；CLI 对仅识别的 Agent 立即拒绝 `agent wait` 和 `agent prompt --wait`。级别只看 hooks 能报告什么，是否做过真机验证另行记录。
4. 新增官方 Agent，按两级门槛逐个评估。候选来自 2026-10-06 对其他工具所支持 CLI 的调查。
5. 在 `docs/agents/` 写一页贡献指南，说明加一种 Agent 要改哪几处，并加一个 Issue 模板。

**停在哪**：不做用户自定义 Agent，GUI 启动菜单在摩擦记录里，不接入 ACP，也不为没有 hook 的 CLI 猜状态。

### 2. 终端内搜索

**为什么**：这一项原本在摩擦记录里等证据。Agent 的输出很长，要在里面找一段报错或一个路径，现在只能靠滚动。对外宣布后，新用户会拿 Condr 和他们用惯的终端比较，而这些终端都能搜索。

**做到哪**：回滚内容只在 Server 的 VT 里，所以搜索是一个协议请求，由 Server 在 VT 上查找并返回匹配的位置，GUI 高亮并滚动过去。CLI 可以用同一个请求。

**停在哪**：只在当前 Pane 里搜索，不做跨 Pane 或跨 Workspace 的搜索。命令面板和文件快速打开不做。

### 3. 移动端 Companion（iOS 与 Android）

**为什么**：离开电脑时，用户最想知道哪个 Agent 做完了、哪个在等自己。厂商的 Remote Control 只管自家的 Agent；Condr 已经有跨 Agent、跨 Device 的「Needs you」列表和 `blocked_on`，把它们带到手机上是这一层能做、单个厂商不会做的事。

**已定**：

- 两端 UI 都用原生开发。
- 客户端逻辑只有一份 Rust 实现，经 uniffi 生成 Swift 与 Kotlin 绑定。iroh、Noise 和 Condr 协议编进同一个库，每个平台只链接这一个库，所以不用 iroh 官方的 Swift 与 Kotlin 绑定：iOS 上链接两个各自构建的 Rust 静态库，会因标准库和分配器符号重复而失败。
- 手机支持 `p2p://` 与 `tcp://`，和桌面共用同一份连接与握手代码。
- 推送要自建中转。原生 App 的推送经 APNs 和 FCM，它们的凭据只能放在 Condr 运营的服务端，不能放进每个用户的 Server。
- 推送正文带 `blocked_on`，由 Server 用手机的设备公钥加密，中转、苹果和谷歌都看不到。这把公钥 Server 在配对时已经记进 `authorized-clients`，不需要另外约定密钥；手机端解密需要读到 device key，iOS 上 Notification Service Extension 要和 App 共享它。
- 推送中转用设备 key 认证，和 relay 一样不需要登录。手机用自己的 device key 签名，登记推送 token，以及允许哪些 Server 给它推送；Server 用自己的 device key 签名推送请求，请求带时间戳防重放，签名内容加专用前缀，不能挪到其他协议里冒用。Server 和手机都不新增凭据，协议也不用为推送加消息。代价是中转要落盘保存哪些 Server 和哪部手机配对，以 Device id 记录。

**做到哪**（按顺序）：

1. 写 ADR，定下这些问题：
   - 客户端库的边界：`ClientConnection` 和设备密钥在 `condr-server` 里，协议和领域类型在 `condr-core` 里，和 PTY、VT、Git 同在一个 crate。手机库只该带协议、密钥和连接，所以要先把它们拆出来，或者用 feature 隔开。
   - 手机的 Peer-to-peer 端点：App 自己绑定 iroh endpoint，直接拨 Server，不经 `Tunnel`。这仍符合 ADR 0025「一个 key 只在一个 endpoint 上」，因为 App 是手机上唯一使用这把 key 的进程；iOS 的 Notification Service Extension 只负责解密，不能再绑定 endpoint。
   - 手机能做哪几个动作，至少要回答能不能给 Blocked 的 Agent 输入回复。
2. 客户端库：拆出的 crate 加上 uniffi 绑定，产出 xcframework 与 AAR，CI 增加 iOS 与 Android 目标；按 ADR 0028 的预留，在 `Hello` 里加帧率上限、视口等 Client 提示。
3. 配对与连接：桌面 Settings 的 Invite 把链接显示成二维码，手机扫码配对，Server 照常记进 `authorized-clients`。走 Peer-to-peer 的 Server 要开启 `[server.p2p] enabled`，配对引导要提示这一步。App 只在前台保持连接，回到前台时重连并重新 Bootstrap；后台全靠推送。
4. 查看：各 Device 的 Workspace 与 Agent 状态、「Needs you」列表、只读的终端视图。
5. 推送：中转上线，按客户端限速，并在 SECURITY.md 写明它保存配对关系和推送 token、看不到通知内容；Agent 完成或进入 Blocked 时推送，点开后重新拉取 Server 的权威状态。
6. 少量动作：按第 1 步的 ADR 实现。

**要实测的风险**：蜂窝网络下打洞可能更常失败，流量更多经 relay.condr.dev 转发，会推高带宽成本，也会碰到 relay 对每个客户端的限速；Wi-Fi 与蜂窝切换时连接能否保住（iroh 1.0 支持 QUIC multipath，未验证），保不住就走第 3 步的重连。

**停在哪**：不把桌面 GUI 的全部功能搬到手机；`ssh://` 不在这一方向内，手机要内置 SSH 客户端，等有需求再议；不做 Web 客户端。

### 4. 探测 Workspace 里的端口并转发到本机

**为什么**：Agent 在远端 Workspace 里起了前端 dev server，用户要在本机浏览器看效果，今天只能自己开 `ssh -L`。"看见 Agent 做出来的东西"是介入和收尾闭环的一部分；Server 已有按平台实现的进程表（Agent 检测用），加一步"该进程树在监听哪些端口"是同一条路。

**做到哪**（按顺序，落地前先写 ADR）：

1. 探测：Server 在进程表变化时查 Pane 进程树的监听端口（Linux `/proc/net/tcp*`，macOS `proc_pidfdinfo`，Windows `GetExtendedTcpTable`），作为 Pane 状态经事件下发；GUI 在 Workspace 和 Pane 上显示端口徽标，`condr pane|workspace` 的 JSON 携带端口。
2. 转发（SSH Device）：点击端口即在本机开 listener，另起一条 `ssh -N -L` 到远端 `localhost:<port>`；`condr port forward <device> <port>` 同义。
3. 一键在浏览器打开本机转发地址。

**停在哪**：只做 SSH Device 的转发。TCP 与 Peer-to-peer Device 的转发要在协议里新增一种多路复用的字节流帧，改动大，放进摩擦记录等真实需求。不做反向转发、不做 UDP、不自动转发所有探测到的端口（默认只列出，点了才转）、不解析进程输出里的 URL。

### 5. GitHub：PR 状态与检出 PR

**为什么**：收尾的最后一步通常是推分支、开 PR、等 CI。现在用户要切到浏览器，才知道这个 Workspace 的分支有没有 PR、检查过没过；要在本机看别人的 PR，也得自己建 worktree 再检出。把这些状态放到 Workspace 上，收尾就不用离开 Condr。

**做到哪**（按顺序，落地前先写 ADR）：

1. Server 识别 Workspace 当前分支对应的 GitHub PR，查询它的状态（打开、草稿、已合并、已关闭）和 CI 检查结果，作为 Workspace 状态经事件下发。侧栏和 Workspace 上显示 PR 编号与检查状态，点击在浏览器打开；`condr workspace` 的 JSON 带上这些字段。
2. 输入 PR 编号或链接，新建一个检出该 PR 的 Managed Worktree。
3. PR 合并后，Workspace 上提示可以 Remove Worktree。

ADR 要回答三个问题：

- 认证从哪来：复用 `gh` 的登录，还是读 `GITHUB_TOKEN`。无论哪种，token 只留在 Server 所在的机器上。
- 多久查一次。只查有 Workspace 打开的分支，并按 GitHub API 的限额安排。
- 检出 PR 要先 fetch，gix 的 fetch 和凭据处理够不够用。

**停在哪**：只做 GitHub，GitLab 和 Gitea 等真实需求。不在 Condr 里创建、合并或评审 PR，这些交给 Agent 和 `gh`。PR 合并后不自动删除 worktree，删除仍是用户的显式操作（ADR 0002）。不做从 Issue、PR 评论或聊天工具触发 Agent，那需要一个面向公网的服务。

### 6. 公开预览收尾，拿到第一批外部用户

**为什么**：摩擦记录和远期表里几乎每一项的启动条件都是外部用户或真实需求。没有外部用户，这些条件不会满足，路线只能靠单人 dogfood 推进。但宣布带来的新用户会按宣布时的样子判断 Condr，所以先做完前五项再宣布，让他们第一次试用就能走完多 worktree 的完整流程，离开电脑也能在手机上看到 Agent 的状态。

**做到哪**（按顺序）：

1. 前五项完成后，对外宣布。
2. 外部 Issue 按 [triage 标签](agents/triage-labels.md) 分流。缺陷直接修；功能请求进摩擦记录，同一件事出现两次以上再排。

**停在哪**：Windows 安装包预览期仍不签名，文档已写明 SmartScreen 放行步骤。

**退出条件**：外部用户按文档装好，并提出第一批 Issue。

## 按摩擦记录再做

这些都通过了第一问，但还没有足够的使用证据；出现两次以上真实摩擦再排。外部用户的 Issue 和单人 dogfood 一样算数。

- Diff Tab 内跳到编辑器。
- GUI 里启动 Agent 的菜单：列出这台 Device 上装了哪些 Agent，点一下就在 Pane 里启动。现在在终端里输入命令已经够快。真要做时，用协议里现成的 `AgentCommand::Available` 和 `Start`。
- 定时任务：按 cron 启动新的 Agent，或定时给已有的 Agent 发提示。现有的 `agent start`、`agent prompt` 和 `agent wait` 已经覆盖了大半。它会让 Condr 从看着 Agent 干活走向无人值守地派活，用户的期待也会跟着变，所以等有人提出需求再定。
- TCP 与 Peer-to-peer Device 的端口转发：协议增加一种多路复用的字节流帧，把本机 listener 的连接经 Noise 或 iroh 连接接到远端 `localhost:<port>`。默认只有配对设备能开转发；若届时已有 observe/control capability，则只允许 `control`。
- 颜色查询回复 GUI 当前的终端配色（见已知限制）。
- `observe` 与 `control` 两级 capability（观察者拿 Bootstrap 和视觉流，不能输入、改布局、读文件；配对时决定，`condr server clients` 可看可改）：触发条件是第二个人开始共用同一个 Server。之后再谈设备密钥进 OS Keychain、密钥轮换、审计日志。不做账号体系、不做公网 listener。
- Kimi hooks（等上游能区分主任务与子 agent 的 Stop）。

## 远期

会做，但都排在近期方向之后，且各自有启动条件。条件未到之前不写 ADR、不预留接口。

| 事项 | 启动条件 | 前置 |
| --- | --- | --- |
| 遥测（opt-in） | 有外部用户，且本地日志已不足以排障 | 诊断数据默认不含终端内容；先有本地日志再谈上报 |
| 团队协作、账号与 RBAC | 出现多人共用一个 Server 的真实需求 | observe/control capability（见摩擦记录）扩展为多用户；审计日志 |
| 替用户批准 Agent 的权限请求（现在只做"看见并跳转"） | 厂商开放外部审批接口；Claude Code 的远程审批仍是 [open feature request](https://github.com/anthropics/claude-code/issues/38299)，Codex 的审批只在它自己的 app-server 客户端里 | `Blocked` 与 `blocked_on`（ADR 0024） |
| 编辑器、内置浏览器、任务看板等 IDE 化能力 | dogfood 中反复出现"为了这件事必须离开 Condr" | 逐项立 ADR，不成套引入 |

不做的三件事：**Web 客户端**（Condr 是原生 GUI，不把控制面搬进浏览器；异地需求由 Peer-to-peer 和移动端 Companion 承接）；**自建对话或工具循环、绑定单一模型厂商**（Condr 编排原生 Agent CLI，不替代它；因此也不在界面上另做输入框、模型选择或语音对话，这些交给各个 CLI 自己）；**用户自定义 Agent 和插件 SDK**（ADR 0035：Agent 种类变化不频繁，官方支持常见的 CLI 就够了；每种集成都需要声明写不出来的 hook 代码和真实会话验证；未支持的 CLI 通过 Issue 或 PR 加入）。

## 节奏

- 日常修复合入 `main` 即进入次日 Nightly；需要固定版本时按 [Releases](releases.md) 打 `v*` 标签。
- 对外宣布之后正式版是否放慢，尚未决定。要权衡的是：每个正式版都会让 GUI 亮起更新提示，更新后还会提示重启旧版 Server，发得越勤，修复越快到用户手里，提示也越频繁。
- 每个近期方向落地前先写 ADR，落地时同步更新 CONTEXT.md、AGENTS.md 与本文的现状表。
- 本文每次重大方向变化时整体重写，不追加"已完成"段落；历史看 git log 和 ADR。

