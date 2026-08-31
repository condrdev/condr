# Condr Roadmap

> 状态：执行中（2026-08-31）
>
> MVP Phase 7 已完成，验收记录见 [GitHub #16](https://github.com/condrdev/condr/issues/16)；当前进入 M0 Rolling Developer Preview，随后用 M1 完成自举。

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

Phase 7 release gate 已在 [GitHub #16](https://github.com/condrdev/condr/issues/16) 完成，Condr 已进入 post-MVP。后续每个发布候选仍需在同一 commit 上通过适用的自动化与人工验证。

当前项目由单个开发者维护，近期没有对外发布计划。现阶段先建立一个可下载、可更新的滚动开发版，再使用 Condr 开发 Condr 本身；完成自举后才重新决定对外发布范围，不提前承担 Private Alpha 的安装、兼容和支持成本。

## 排序原则

每个候选项目先回答三个问题：

1. 它是否让“启动 -> 并行 Agent -> 监控/介入 -> 重连 -> 收尾”更快、更可靠或更安全？
2. 它是否跨 Agent、跨平台，或者能被多个客户端复用？
3. 它是否依赖尚未稳定的协议、权限模型或服务端基础设施？

优先做高频、差异化、低依赖的能力；纯装饰、单一厂商特例、需要重写 Agent loop 的项目后置。

| 优先级 | 方向 | 判断 |
| --- | --- | --- |
| P0 | 自举与日常体验 | 滚动开发版、真实 dogfood、Pane/Tab 操作、命令可发现性和最小设置 |
| P0-C | 核心可靠性 | 数据安全、重连、终端性能和真实故障所需的诊断能力 |
| P0-R | Remote 安全门 | 对外 remote 前必须有鉴权、配对、授权、撤销和加密 |
| P1 | 自动化与壁垒 | 稳定 Client API、CLI、Agent Profile、Skill |
| Gate | 对外发布信任 | 有明确发布计划后再做诊断、安装/升级、支持矩阵和兼容策略 |
| P3 | 分发与扩张 | Relay、Web、Mobile、团队协作和 Hosted 服务，按需求证据推进 |

## 依赖关系

```text
M0  Rolling Developer Preview
 |
 \---- M1  Solo Daily Driver / 自举
          |\
          | \-- R0  Release Readiness / Private Alpha（有发布计划后）
          |
          \---- M2  Protocol/Client API + Headless Server 发布
              |\
              | \-- M3  Secure Direct Remote / Pairing
              |        \
              |         \-- M5  Relay（需求门控）
              |
              \---- M4  condr-cli Workflow -> Agent Profile -> Skill

M3 + 稳定语义 API -> M6 Web 只读 -> Mobile Companion -> 受控交互

开发文档从 M0 开始按需维护；面向用户的品牌、社区治理和正式推广在 R0 再启动，不需要等待完整 Mobile。
```

M0 只建立自举所需的滚动开发版，完成后进入 M1。M1 自举完成后再决定进入 R0 还是继续 M2；M3 需要 M2 的 Server 发布和协议/客户端边界，M2 才建立独立的 `condr-cli` 只读骨架，M4 再扩展 Agent workflow，Skill 最后依赖 CLI。Relay 和新客户端都不能绕过 M3 的身份与授权模型。

## 里程碑

### M0：Rolling Developer Preview

**核心交付**

- 在私有 GitHub 仓库中只维护一个标记为 Pre-release 的 `Development Build`，由固定的可变 `dev` tag 指向当前选定 commit。
- 开发中的每个 commit 不自动发布；只有主动移动并推送 `dev` tag 才触发更新。
- 从同一个 tagged commit 生成 Windows GUI 与同目录 Server bundle，以及 Linux x64/arm64 Server artifacts；asset 名称、release notes 和 artifact 都记录 commit SHA，避免滚动更新后混淆版本。
- 先手工完成一次构建、打包和解压验证，再把已验证流程做成由 `dev` tag 触发的最小 GitHub Actions 自动化。
- 提供仅供开发者使用的下载、启动、SSH 连接和更新说明，使日常运行不依赖 `cargo run`。

**非目标**

- 不作为 Private Alpha，不承诺外部用户或 Windows x64、Linux x64/arm64 之外的平台支持。
- 不做安装器、代码签名、checksums、自动升级、系统服务、遥测、协议兼容层或完整发布流水线。
- 不为尚未发生的故障预建结构化日志、health/status、诊断包或性能基准；在 M1 的真实使用需要时加入最小工具。

**退出条件**

- Windows 可以从 `Development Build` 解压并启动 GUI；GUI 能发现同目录 Server，且不需要从源码启动。
- 同一 release 的 Windows GUI 能通过既有 SSH tunnel 连接 Linux Server，并完成一次断开、重连和恢复；Linux x64/arm64 artifacts 均能独立启动。
- 所有目标构建成功后才更新 release assets；成功更新后 `dev` tag、release notes 和所有 artifacts 指向同一 commit。
- Condr 可以用该滚动开发版开始开发 Condr 自身。

### M1：Solo Daily Driver / Desktop Daily Driver

**核心交付**

- 使用 Condr 完成真实的 Condr 开发任务，记录仍需退回其他终端或手工处理的原因，并优先消除最高频摩擦。
- 先完善实际阻塞自举的 Pane/Tab/Workspace 创建、关闭、移动、聚焦、拆分、交换、缩放和恢复反馈。
- 当快捷键、菜单、Toolbar、右键菜单或命令面板出现真实的重复与不一致时，再建立覆盖当前命令的最小 `Command Registry`。
- 按实际需要加入最小设置，候选范围包括字体/字号、终端主题、默认 Shell、启动行为、连接项、快捷键和通知。
- 在既有 SSH tunnel 验证稳定后，增加原生 `SSH` 连接项：Client 通过系统 `ssh` 的 stdio 连接远端 `condr-server bridge`，bridge 只负责连接远端私有 endpoint 并双向转发现有协议；远端 Server 仍独立存活。保留 Local/TCP，首版不做自动安装、升级或 live handoff。设计依据见 [Herdr SSH remote 调查](research/herdr-ssh-v0.8.2.md)。
- 明确配置归属：GUI 外观与快捷键属于 Client；Shell、Agent Profile 和 Server 配置属于 Server；GUI 偏好不进入 Session Snapshot。
- 根据真实使用加入桌面通知：Agent `done/blocked`、Server 离线、重连成功，并提供静默/过滤选项。
- 根据真实使用加入跨平台的搜索、复制路径、Reveal in Explorer/Finder、在 IDE 或自定义命令中打开当前 `pwd` 等 context action。

**非目标**

- 不为每一个 IDE 或 Agent 写硬编码入口。
- 不因为“功能完整”而提前实现罕见终端协议；按真实 Agent 工作流逐项加入。

**退出条件**

- 已使用 Condr 完成多个真实的 Condr 开发任务，主要回退原因已经记录并处理或明确接受。
- 新用户可以在几分钟内创建三个 Pane、启动一个代表性 Agent 并完成一次切换/介入流程。
- 高速 Agent 输出下，拖选、输入、滚动和 Pane 操作仍保持可用帧率。
- 所有布局变更仍经过 Server 权威状态确认，GUI 缓存不能成为真相来源。

### R0：Release Readiness / Private Alpha（延期）

**进入条件**

- M1 已完成自举，并且已经明确决定邀请外部用户、目标平台和发布范围。

**核心交付**

- 以已完成的 Phase 7 验收证据和后续 dogfood 为基线，维护一个可复现的 Private Alpha 候选版本。
- 增加结构化日志、Server health/status、故障诊断、崩溃/PTY 孤儿排查信息和终端 burst 基准。
- 冻结 Session、Bootstrap、可靠事件和视觉流语义；定义从当前严格协议版本到公开版本的兼容策略。
- 固定首发支持矩阵，不提前承诺所有平台。
- 建立小规模外部 dogfood，使用同一个 canonical workflow：多个 worktree/Agent、断开 GUI、重连和 Server 重启。

**退出条件**

- 新环境能按文档安装、启动、连接和恢复。
- 输入延迟、视觉丢帧、重连成功率、Agent 状态误报和 Server 崩溃都有可重复的测量方法。
- 已知限制和安全边界可被用户理解；诊断默认不采集终端内容，额外数据采用 opt-in。

### M2：Headless Server 与稳定 Client API

**核心交付**

- 发布独立的 `condr-server`：Linux x64/arm64、Windows Server 构建产物、checksum/signing、配置、数据目录、日志、health/status、systemd 或 Windows service。
- 明确升级、回滚、备份和 Server identity 的持久化位置。
- 从当前实现中拆出可复用的 `condr-protocol` 和 `condr-client` 边界；Server 保留 Runtime，GUI 不再反向定义协议。
- 在 Hello/Welcome 和语义 API 中加入 client kind、capabilities、typed errors、幂等 request ID、event cursor、权限上下文和 controller lease。
- 发布独立的 `condr-cli` 只读骨架：它是面向 Agent 的纯 Client，不依赖 GPUI、不承载 Server 生命周期；先提供 Session、Workspace、Tab、Pane 的 `list/status`，统一 `--json`、稳定退出码、超时和诊断信息。

**协议原则**

当前 `bincode + serde` 适合作为 native 内部传输，但不要把当前 Rust enum 编码形式当作跨语言公共 ABI。公开后应区分 wire codec、语义 API、capability 和 build version；破坏性变更升 major，增量能力通过协商。

**退出条件**

- 一台干净 Linux 机器可以在无 GUI 的情况下安装、启动、重启和恢复 Server。
- GUI 与 `condr-cli` 可以共享同一客户端状态机，并能清楚区分版本不兼容、未授权和运行时错误。
- Server 断开 GUI 不会停止 Session、PTY 或 Agent。

### M3：Secure Direct Remote

这是对外宣传 remote 的硬门槛。当前 trusted TCP/SSH 配置适合 MVP 和技术用户内测，不能直接当作公网安全产品。

**核心交付**

- Server 持久化身份和公钥指纹；使用成熟的 TLS/SSH/Noise 实现，不自创密码学。
- 由 Server owner 通过 CLI 或 GUI 生成一次性、短时有效的 invite code/QR；首次连接显示指纹并要求人类确认。
- Client 生成设备密钥，长期凭据保存在 OS Keychain/Keystore，而不是 URL、日志或普通配置文本中。
- 提供设备列表、过期、撤销、轮换、审计和限速。
- 认证与授权分开，至少定义 `observe`、`input`、`layout`、`workspace/git`、`server_admin`、`clipboard/sensitive`、`agent_automation` capability。
- 未完成授权前不得发送 Bootstrap、Terminal 内容、剪贴板或 Agent 数据；默认仍是单用户、一个 active controller。

**威胁模型必须覆盖**

网络窃听和 MITM、配对码泄露/重放、设备丢失、GUI 被攻陷、跨 Session 越权、路径穿越、任意进程执行、Agent 权限提升、资源耗尽和审计泄露。

**退出条件**

- Windows GUI 能安全连接 Linux Server，并验证拒绝、撤销、重连、版本不兼容和控制权转移。
- 未授权客户端无法观察到结构或终端数据；安全失败有明确诊断。
- 单用户自托管场景的部署和恢复有完整文档。

### M4：condr-cli Agent Workflow

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

### M5：Relay Server（需求门控）

Relay 不是第二个 Condr Server。它只解决 rendezvous、NAT/防火墙穿透和加密流转，Session、PTY、Snapshot、终端明文和 Agent 状态仍只存在于 `condr-server`。

**首版原则**

- `condr-relay` 独立部署，分离连接注册/配对/在线状态的 control plane 与带背压的 data plane。
- Server 和 Client 都主动出站连接 Relay；Relay 只转发端到端加密的 opaque stream。
- 不落盘终端内容和 Session，不把 route ID 当作授权，不记录长期 secret。
- 先做可自托管实验版；官方 Hosted Relay 要等隐私、滥用防护、带宽、区域、成本和运维方案成熟。
- LAN、SSH 或直连成功时不经过 Relay。

**Go/No-Go**

先用 SSH、Tailscale、反向代理和文档验证需求。只有远程用户经常因 NAT/防火墙失败，且明确需要免 SSH 配置的体验时，才进入 Relay 实现；上线前要有 NAT 测试矩阵、限速/配额、连接过期、审计和安全审查。

### M6：Web 与 Mobile Companion

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

这条线按当前阶段控制投入：

- M0：仅维护滚动开发版的下载、启动、SSH 连接和更新说明。
- M1：记录实际 dogfood workflow、已知限制和需要回退到其他工具的场景。
- R0：补齐 Quickstart、安装、支持矩阵、远程安全边界、故障排查和 canonical demo。
- M2：Server 运维、数据目录、升级/回滚、版本兼容矩阵、录屏和部署示例。
- 公共 Beta：官网、品牌/图标/截图和文案统一，CHANGELOG、SECURITY.md、贡献指南、Code of Conduct、Issue/Discussion 模板和发布节奏。
- Hosted Relay、账号体系、团队协作、企业 SSO/RBAC 和云端数据存储不在早期宣传中承诺。

首发内容应围绕一个可演示闭环：同一项目的多个 worktree/Agent、GUI 断开后继续运行、重新连接后恢复，而不是堆叠营销功能。

## 持续指标

M0/M1 不建设遥测，只记录自举中直接观察到的摩擦与性能问题。进入 R0 后再考虑以下指标；诊断数据默认不包含 Terminal 内容，并采用 opt-in：

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

## 当前第一批工作

1. 从一个明确 commit 手工构建 Windows bundle 和 Linux arm64 Server artifact，验证原生打包内容。
2. 从解压目录验证 Windows 本地 Server、Windows GUI 到 Linux Server 的 SSH 连接，以及断开和重连。
3. 将已验证的构建和上传步骤做成最小 GitHub Actions 自动化，同时生成 Linux x64 artifact；只有主动移动并推送滚动 `dev` tag 才触发更新，不按普通 commit 触发。
4. 用滚动开发版进入 M1，使用 Condr 开发 Condr；只为真实出现的高频摩擦创建和排序任务。
5. 自举完成后重新评估 R0 与 M2，不在当前阶段展开 Private Alpha、公开协议或远程产品化工作。
