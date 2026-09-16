# Condr Roadmap

> 状态：M0/M1 已完成，post-M1 持续 dogfood 中；发布基础设施（nightly、三平台 desktop/headless 产物、安装脚本、社区文档）已就位，是否邀请外部用户尚未决定（最近核对 2026-09-16）
>
> MVP Phase 7 验收见 [GitHub #16](https://github.com/condrdev/condr/issues/16)，M1 自举验收记录在下文「已完成」一节。功能细节以 [ADR](adr/)、[Development Build](development-build.md)、[Releases](releases.md) 和 git 历史为准，本文只记录方向、边界和决定。

本文档描述 MVP 通过 Phase 7 发布门之后的方向。它不修改 [MVP 验收计划](mvp-plan.md) 中已经约定的 Server、Session、Terminal、Agent 和 Git 边界。

## 目标与边界

Condr 的北极星不是“功能最多的终端”，而是一个可靠、易上手、可安全远程的 native-agent terminal 控制面：

```text
选择项目/Root
  -> 可选 Git worktree
  -> 并行运行多个原生 Agent CLI
  -> 快速切换、监控、介入
  -> GUI 断开后任务继续运行
  -> 重连恢复
  -> 完成与清理
```

Agent 仍然是 Terminal 中的可选进程，Condr 负责编排、呈现和生命周期，不自建对话循环，也不把自己定位成 IDE、通用 SaaS 或云端 Agent 平台。

用户只需要认识两种构建：**desktop**（GUI + CLI，各平台安装包）和 **headless**（仅 `condr` 二进制，安装脚本）。两者来自同一 commit，Client 与 Server 之间没有跨构建兼容承诺。

当前项目由单个开发者维护，维护者用 Condr 开发 Condr。发布基础设施已经完成，但没有对外发布计划；在决定邀请外部用户之前，不承担 Private Alpha 的兼容和支持成本。

## 排序原则

每个候选项目先回答三个问题：

1. 它是否让“启动 -> 并行 Agent -> 监控/介入 -> 重连 -> 收尾”更快、更可靠或更安全？
2. 它是否跨 Agent、跨平台，或者能被多个客户端复用？
3. 它是否依赖尚未稳定的协议、权限模型或服务端基础设施？

优先做高频、差异化、低依赖的能力；纯装饰、单一厂商特例、需要重写 Agent loop 的项目后置。

| 优先级 | 方向 | 判断 |
| --- | --- | --- |
| P0 | 日常体验 | 真实 dogfood 中出现的摩擦、Pane/Tab 操作、命令可发现性和最小设置 |
| P0-C | 核心可靠性 | 数据安全、重连、终端性能和真实故障所需的诊断能力 |
| P0-R | Remote 安全门 | 对外 remote 前必须有鉴权、配对、授权、撤销和加密 |
| P1 | 自动化与壁垒 | 稳定 Client API、CLI、Agent Profile、Skill |
| Gate | 对外发布信任 | 有明确发布计划后再做诊断、签名、支持矩阵和兼容策略 |
| P3 | 分发与扩张 | Relay、Web、Mobile、团队协作和 Hosted 服务，按需求证据推进 |

## 依赖关系

```text
M0  Rolling Developer Preview        [完成]
 |
 \---- M1  Solo Daily Driver / 自举   [完成，持续 dogfood]
          |\
          | \-- R0  Release Readiness / Private Alpha（基础设施就位，等待发布决定）
          |
          \---- M2  Protocol/Client API + Headless Server 发布
              |\
              | \-- M3  Secure Direct Remote / Pairing
              |        \
              |         \-- M5  Relay（需求门控）
              |
              \---- M4  condr-cli Workflow -> Agent Profile -> Skill

M3 + 稳定语义 API -> M6 Web 只读 -> Mobile Companion -> 受控交互
```

R0 与 M2 互不阻塞：R0 取决于是否邀请外部用户，M2 的协议/客户端拆分是 M3 授权模型和 M4 `condr-cli` 的共同前置。M1 已在 `condr` 中落地自举所需的 CLI 编排与 Skill；M2 再建立独立的 `condr-cli` 和可复用客户端边界。Relay 和新客户端都不能绕过 M3 的身份与授权模型。

## 已完成：M0 与 M1（截至 2026-09-16）

M0 原本只要求一个手动移动 `dev` tag 的滚动开发版，M1 只要求自举；两者关闭后的一周里，发布和日常体验都超出了原定范围。以下是现状，不再逐条重述过程。

**发布与安装**（[Releases](releases.md)、[Development Build](development-build.md)）

- `nightly.yml` 每日构建 `main` 并重建可变的 Nightly 预发布；`release.yml` 由 `v*` 标签触发不可变正式版；两者共用 `build.yml`。PR 有 CI 检查，工具链由 `rust-toolchain.toml` 固定。
- 每次发布从同一 commit 生成 Linux x86_64/arm64 AppImage、Windows x86_64 Inno Setup 安装器与 ZIP、macOS x86_64/arm64 拖放式 DMG，以及各平台 `condr-headless-*` 归档；Release 附带 `SHA256SUMS`，包内 `BUILD-COMMIT` 记录 SHA。
- Windows 安装器只有 desktop 一种形态：升级前停掉运行中的 Server，PATH 只注册一次，卸载时停 Server 并清理 PATH。macOS 启动器每次启动幂等执行 `condr server install`（[ADR 0016](adr/0016-server-install-installs-the-running-binary.md)）。
- `script/install-condr.sh` / `.ps1` 安装 headless 归档，处理校验和、PATH 与覆盖确认；各平台包检查脚本在 CI 中实际安装并卸载。
- 仓库已有双语 README、CONTRIBUTING、SECURITY、Code of Conduct、Issue 模板和 triage 标签。

**桌面体验**

- Settings 拆为 Appearance、Terminal、Daemon、Clients、Agents、Notifications 页；Daemon 页可改 TCP 监听、签发 invite、重启 Server，Clients 页可撤销设备。
- Agent 完成或需要输入时发送 OS 通知并请求窗口注意；可选「保持屏幕唤醒」。
- 连接状态可见并自动重连；Server 报告停止后 GUI 自行重连。无 Workspace 的 Device 显示 Welcome 页；界面统一用「Device」指一台机器。
- Workspace 的 Tab 条移入标题栏；标题栏「Open in」按可探测的编辑器预设、`[[client.editors]]` 自定义命令或系统文件管理器打开 Workspace 根目录（仅本地 Server）。
- Pane 缩放、拖动分割条、Tab 悬停关闭、远程图片粘贴进度提示。
- 右侧栏 Files / Changes 两个视图：Changes 由 Server 用 gix 计算并推送，点击打开每 Workspace 唯一的只读 Diff Tab（[ADR 0017](adr/0017-changes-sidebar-and-diff-tab.md)）；Files 按需列目录、标记 ignore 与变更、右键复制路径/用编辑器打开/插入终端，点击打开只读 Preview Tab（[ADR 0018](adr/0018-files-sidebar-and-preview-tab.md)）。
- macOS 有原生菜单栏（Cmd+Q/H/M），本地 socket 在 macOS 上可连。

**远程与安全**

- TCP 连接按 WireGuard 模型认证加密：`Noise_IKpsk2`、持久静态密钥、一次性 invite 配对、`condr server clients|revoke`（[ADR 0011](adr/0011-tcp-endpoints-authenticate-like-wireguard.md)）。
- 原生 `ssh://` 连接：系统 `ssh -T` 运行远端 `condr server bridge` 转发私有 socket，远端 Server 未运行时自动启动；要求远端预装 `condr`（[ADR 0015](adr/0015-ssh-forwards-the-remote-private-socket.md)）。

**Agent**

- Agent 状态只来自 Agent CLI 自己的 hooks，经 OSC 777 回写（[ADR 0014](adr/0014-agent-status-comes-from-hooks-over-osc-777.md)）；Windows hooks 验收见 [#30](https://github.com/condrdev/condr/issues/30)，跨 Pane 编排见 [#27](https://github.com/condrdev/condr/issues/27)，远程图片粘贴见 [#29](https://github.com/condrdev/condr/issues/29)。

**M1 遗留的可选项**

- 终端内搜索和命令注册表 / 命令面板至今没有出现足以触发实现的摩擦记录，仍未实现。
- Changes 的「分支 vs base」比较模式（`GitDiff.against` 已预留）等摩擦出现后单独立 ADR。
- 已知限制独立跟踪：[颜色查询 #23](https://github.com/condrdev/condr/issues/23)、[Kitty keyboard #28](https://github.com/condrdev/condr/issues/28)、[OpenCode 真机验证 #36](https://github.com/condrdev/condr/issues/36)、[OSC 支持对照 #26](https://github.com/condrdev/condr/issues/26)。当前状态以 GitHub Issues 为准。

**仍是非目标**：自动升级、系统服务、遥测、协议兼容层、代码签名。

## R0：Release Readiness / Private Alpha（等待发布决定）

**进入条件**

- 明确决定邀请外部用户、目标平台和发布范围。

**已就位**

三平台安装包、headless 归档、校验和、安装脚本、nightly/正式版流水线和社区文档（见上一节）。R0 的基础设施部分已经完成；剩余部分全是“有外部用户才值得做”的工作。

**剩余交付**

- 代码签名：Windows 安装器与 EXE（否则 SmartScreen 拦截「未知发布者」）、macOS 签名与公证。需要证书，是流程决定而非脚本改动。
- 诊断：结构化日志、Server health/status、崩溃与 PTY 孤儿排查信息、终端 burst 基准。代码中目前一项都没有。
- 冻结 Session、Bootstrap、可靠事件和视觉流语义；定义从当前严格协议版本到公开版本的兼容策略。
- 固定首发支持矩阵（当前实际构建：Linux x86_64/arm64、Windows x86_64、macOS x86_64/arm64），不提前承诺更多。
- 用户文档：Quickstart、远程安全边界、故障排查和一个 canonical demo。
- 小规模外部 dogfood，使用同一个 canonical workflow：多个 worktree/Agent、断开 GUI、重连和 Server 重启。

**退出条件**

- 新环境能按文档安装、启动、连接和恢复。
- 输入延迟、视觉丢帧、重连成功率、Agent 状态误报和 Server 崩溃都有可重复的测量方法。
- 已知限制和安全边界可被用户理解；诊断默认不采集终端内容，额外数据采用 opt-in。

## M2：Headless Server 与稳定 Client API

headless 构建产物、校验和、安装脚本、`condr server restart|clients|revoke|invite` 与 Settings 的 Daemon/Clients 页已经存在。以下仍全部未开始：协议/客户端 crate 拆分、独立 `condr-cli`、health/status、系统服务、升级与回滚。

**核心交付**

- headless Server 的运维闭环：配置、数据目录、日志、health/status、systemd 或 Windows service；明确升级、回滚、备份和 Server identity 的持久化位置。
- 从当前实现中拆出可复用的 `condr-protocol` 和 `condr-client` 边界；Server 保留 Runtime，GUI 不再反向定义协议。
- 在 Hello/Welcome 和语义 API 中加入 client kind、capabilities、typed errors、幂等 request ID、event cursor、权限上下文和 controller lease。
- 发布独立的 `condr-cli` 只读骨架：它是面向 Agent 的纯 Client，不依赖 GPUI、不承载 Server 生命周期；先提供 Session、Workspace、Tab、Pane 的 `list/status`，统一 `--json`、稳定退出码、超时和诊断信息。

**协议原则**

当前 `bincode + serde` 适合作为 native 内部传输，但不要把当前 Rust enum 编码形式当作跨语言公共 ABI。公开后应区分 wire codec、语义 API、capability 和 build version；破坏性变更升 major，增量能力通过协商。开发阶段仍保持 `PROTOCOL_VERSION = 1`，不做兼容层。

**退出条件**

- 一台干净 Linux 机器可以在无 GUI 的情况下安装、启动、重启和恢复 Server。
- GUI 与 `condr-cli` 可以共享同一客户端状态机，并能清楚区分版本不兼容、未授权和运行时错误。
- Server 断开 GUI 不会停止 Session、PTY 或 Agent。

## M3：Secure Direct Remote

这是对外宣传 remote 的硬门槛。传输层认证与加密、invite 配对和设备管理已在 M1 落地（见「已完成」）。以下为仍未完成的部分；其中 capability 授权在代码中尚无任何雏形，认证通过即拥有全部权限。

**核心交付**

- 首次连接在 GUI 中显示指纹并要求人类确认；invite 支持 QR。
- 设备密钥迁入 OS Keychain/Keystore，而不是配置目录中的文件。
- 设备过期、密钥轮换、审计和限速。
- 认证与授权分开，至少定义 `observe`、`input`、`layout`、`workspace/git`、`server_admin`、`clipboard/sensitive`、`agent_automation` capability。
- 未完成授权前不得发送 Bootstrap、Terminal 内容、剪贴板或 Agent 数据；默认仍是单用户、一个 active controller。

**威胁模型必须覆盖**

网络窃听和 MITM、配对码泄露/重放、设备丢失、GUI 被攻陷、跨 Session 越权、路径穿越、任意进程执行、Agent 权限提升、资源耗尽和审计泄露。

**退出条件**

- Windows GUI 能安全连接 Linux Server，并验证拒绝、撤销、重连、版本不兼容和控制权转移。
- 未授权客户端无法观察到结构或终端数据；安全失败有明确诊断。
- 单用户自托管场景的部署和恢复有完整文档。

## M4：condr-cli Agent Workflow

**实施顺序**

1. 在 M2 只读能力之上加入 Session/Workspace/Tab/Pane 生命周期。
2. 加入 `send`、`capture`、`wait agent`、`attach`、取消和失败策略。
3. 加入多 Pane/worktree 的声明式工作流。
4. 在 `condr-cli`/API 之上提供 Agent Profile 和 Skill。

Agent Profile 使用声明式 manifest 描述 executable、argv、环境、启动模板、图标和状态检测规则。先验证 2～3 个代表性 CLI；未知 CLI 仍然可以手动启动和使用。

Skill 是调用 `condr-cli`/API 的受限工作流模板，不是新的对话循环。每个 Skill 声明所需 capability，对启动进程、输入、文件和网络操作提供显式确认。

**退出条件**

- CI 或 Agent 可以在没有 GUI 的情况下查询、创建和等待一个工作流。
- 失败、超时、重试、幂等和控制权边界都可预测。
- Agent 集成减少核心闭环中的步骤，但不牺牲任意 CLI 的可用性。

**后置**

自研 Agent 对话协议、每个厂商的深度插件、插件市场和静默执行任意脚本。

## M5：Relay Server（需求门控）

Relay 不是第二个 Condr Server。它只解决 rendezvous、NAT/防火墙穿透和加密流转，Session、PTY、Snapshot、终端明文和 Agent 状态仍只存在于 Server。

**首版原则**

- `condr-relay` 独立部署，分离连接注册/配对/在线状态的 control plane 与带背压的 data plane。
- Server 和 Client 都主动出站连接 Relay；Relay 只转发端到端加密的 opaque stream。
- 不落盘终端内容和 Session，不把 route ID 当作授权，不记录长期 secret。
- 先做可自托管实验版；官方 Hosted Relay 要等隐私、滥用防护、带宽、区域、成本和运维方案成熟。
- LAN、SSH 或直连成功时不经过 Relay。

**Go/No-Go**

先用 SSH、Tailscale、反向代理和文档验证需求。只有远程用户经常因 NAT/防火墙失败，且明确需要免 SSH 配置的体验时，才进入 Relay 实现；上线前要有 NAT 测试矩阵、限速/配额、连接过期、审计和安全审查。

## M6：Web 与 Mobile Companion

Web 和 Mobile 共享语义 API、鉴权、capability、事件 cursor 和测试向量，但不共享 GPUI，也不把含 PTY/系统调用的 Server Runtime 编译到浏览器。

**Web 顺序**

- 先做只读 Dashboard：Server、Workspace、Agent 状态、最近 Terminal view、连接状态和通知。
- 使用 HTTPS WebSocket gateway 或等价的稳定协议适配层；浏览器 WebSocket 支持二进制数据，但跨语言 schema 仍需明确版本和兼容策略。[MDN WebSocket API](https://developer.mozilla.org/en-US/docs/Web/API/WebSockets_API)
- 通过安全审查和多客户端 controller lease 后，再开放输入、布局和 worktree 写操作。

**Mobile 顺序**

- 先做 PWA 或 companion：Agent `done/blocked`、Server offline、状态查看、停止或批准等少量动作。
- QR/deep link 配对，设备密钥进入 Keychain/Keystore；移动端默认 `observe`，take-control 时显式取得 lease。
- 原生 iOS/Android 和完整多 Pane Terminal 后置，除非实际使用数据形成明确拉力。
- Push 只作提醒，不作状态真相；APNs 是尽力投递，可能延迟、合并或在离线期间暂存，因此通知只带 opaque event ID，打开 App 后重新拉取 Server 权威状态。[Apple Remote Notification 文档](https://developer.apple.com/documentation/usernotifications/setting-up-a-remote-notification-server)

**退出条件**

- 用户可以安全地从浏览器查看状态并完成一次受限操作。
- 移动端在断线、权限撤销和重复推送后仍不会产生错误状态。
- Web/Mobile 不会绕过 Server 的授权、事件顺序和控制权规则。

完整 Web 控制面、多端协作和移动终端编辑属于后续独立阶段，不作为桌面 1.0 的前置条件。

## 文档、品牌与社区

已完成：滚动开发版与安装说明、发布流程说明、双语 README 与 hero 图、CONTRIBUTING、SECURITY、Code of Conduct、Issue 模板、triage 标签、应用图标与各平台品牌资源。

按阶段继续：

- R0：Quickstart、支持矩阵、远程安全边界、故障排查和 canonical demo。
- M2：Server 运维、数据目录、升级/回滚、版本兼容矩阵、录屏和部署示例。
- 公共 Beta：官网、截图和文案统一、CHANGELOG、Discussion 模板和发布节奏。
- Hosted Relay、账号体系、团队协作、企业 SSO/RBAC 和云端数据存储不在早期宣传中承诺。

首发内容应围绕一个可演示闭环：同一项目的多个 worktree/Agent、GUI 断开后继续运行、重新连接后恢复，而不是堆叠营销功能。

## 持续指标

当前不建设遥测，只记录 dogfood 中直接观察到的摩擦与性能问题。进入 R0 后再考虑以下指标；诊断数据默认不包含 Terminal 内容，并采用 opt-in：

- 首次安装到第一个 Agent 的时间。
- 新用户创建三个 Pane 并完成一次任务的时间。
- 高频输出下的输入延迟、视觉丢帧、重连成功率和 Server 崩溃率。
- Server 安装、升级、恢复和 remote 配对成功率。
- Agent 状态误报率、控制权冲突率和错误恢复时间。
- Remote attach 中 SSH/直连失败后实际需要 Relay 的比例。
- Web/Mobile 的状态查看、通知打开和受限操作频率。

## 暂不承诺

- 自研 Agent 对话/工具循环或绑定单一模型供应商。
- 未鉴权的公网 Server listener。
- 在多用户授权、审计和资源隔离之前承诺团队协作或云托管。
- 为了平台数量而同时维护完整的 Web、iOS、Android 和桌面功能 parity。
- 在真实工作流验证之前实现完整插件 SDK、插件市场或大量终端边缘协议。

## 当前下一步

1. 持续用 Condr 开发 Condr，只处理真实出现的摩擦；终端内搜索、命令面板和「分支 vs base」比较等可选项在摩擦记录出现后再实现。
2. 决定是否邀请外部用户。是，则进入 R0，第一步是代码签名证书和诊断能力；否，则 R0 继续等待，工程主线转向 M2 的协议/客户端拆分。
3. 发布节奏交给 nightly：日常修复合入 `main` 即进入次日 Nightly，不再手工移动 tag；需要固定版本时按 [Releases](releases.md) 打 `v*` 标签。
4. 已知限制和延期项以 GitHub Issues 与 triage 标签为准，本文不再重复其状态。
