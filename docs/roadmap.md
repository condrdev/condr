# Condr Roadmap

> 状态：执行中（2026-08-30）
>
> MVP Phase 7 已完成，验收记录见 [GitHub #16](https://github.com/condrdev/condr/issues/16)；当前进入 M0。

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

## 排序原则

每个候选项目先回答三个问题：

1. 它是否让“启动 -> 并行 Agent -> 监控/介入 -> 重连 -> 收尾”更快、更可靠或更安全？
2. 它是否跨 Agent、跨平台，或者能被多个客户端复用？
3. 它是否依赖尚未稳定的协议、权限模型或服务端基础设施？

优先做高频、差异化、低依赖的能力；纯装饰、单一厂商特例、需要重写 Agent loop 的项目后置。

| 优先级 | 方向 | 判断 |
| --- | --- | --- |
| P0 | 发布信任 | Ph7、稳定性、诊断、安装/升级、支持矩阵和性能回归 |
| P0-R | Remote 安全门 | 对外 remote 前必须有鉴权、配对、授权、撤销和加密 |
| P1 | 日常桌面体验 | Pane/Tab 操作、命令可发现性、最小设置、通知和终端实用功能 |
| P2 | 自动化与壁垒 | 稳定 Client API、CLI、Agent Profile、Skill |
| P3 | 分发与扩张 | Relay、Web、Mobile、团队协作和 Hosted 服务，按需求证据推进 |

## 依赖关系

```text
M0  MVP RC / Private Alpha
 |\
 | \-- M1  Desktop Daily Driver
 |
 \---- M2  Protocol/Client API + Headless Server 发布
              |\
              | \-- M3  Secure Direct Remote / Pairing
              |        \
              |         \-- M5  Relay（需求门控）
              |
              \---- M4  CLI -> Agent Profile -> Skill

M3 + 稳定语义 API -> M6 Web 只读 -> Mobile Companion -> 受控交互

文档、品牌、社区治理从 M0 开始低强度并行；正式推广不需要等待完整 Mobile。
```

M1 和 M2 可以在 M0 之后并行。M3 需要 M2 的 Server 发布和协议/客户端边界；M4 的 CLI 需要稳定的语义 API，Skill 再依赖 CLI。Relay 和新客户端都不能绕过 M3 的身份与授权模型。

## 里程碑

### M0：MVP RC / Private Alpha

**核心交付**

- 以已完成的 Phase 7 验收证据为基线，维护一个可复现的 Private Alpha 候选版本。
- 增加结构化日志、Server health/status、故障诊断、崩溃/PTY 孤儿排查信息和终端 burst 基准。
- 冻结 Session、Bootstrap、可靠事件和视觉流语义；定义从当前严格协议版本到公开版本的兼容策略。
- 固定首发支持矩阵：先覆盖 Windows GUI 与 Linux x64/arm64 Server，不提前承诺所有平台。
- 建立小规模 dogfood，使用同一个 canonical workflow：多个 worktree/Agent、断开 GUI、重连和 Server 重启。

**退出条件**

- 新环境能按文档安装、启动、连接和恢复。
- 输入延迟、视觉丢帧、重连成功率、Agent 状态误报和 Server 崩溃都有可重复的测量方法。
- 已知限制和安全边界可被用户理解；诊断默认不采集终端内容，额外数据采用 opt-in。

### M1：Desktop Daily Driver

**核心交付**

- 先建立统一 `Command Registry`，再由它生成快捷键、菜单、Toolbar、右键菜单和命令面板。
- 完善 Pane/Tab/Workspace 的创建、关闭、移动、聚焦、拆分、交换、缩放和恢复反馈。
- 加入最小设置：字体/字号、终端主题、默认 Shell、启动行为、连接项、快捷键和通知。
- 明确配置归属：GUI 外观与快捷键属于 Client；Shell、Agent Profile 和 Server 配置属于 Server；GUI 偏好不进入 Session Snapshot。
- 加入桌面通知：Agent `done/blocked`、Server 离线、重连成功，并提供静默/过滤选项。
- 优先实现跨平台的搜索、复制路径、Reveal in Explorer/Finder、在 IDE 或自定义命令中打开当前 `pwd` 等 context action。

**非目标**

- 不为每一个 IDE 或 Agent 写硬编码入口。
- 不因为“功能完整”而提前实现罕见终端协议；按真实 Agent 工作流逐项加入。

**退出条件**

- 新用户可以在几分钟内创建三个 Pane、启动一个代表性 Agent 并完成一次切换/介入流程。
- 高速 Agent 输出下，拖选、输入、滚动和 Pane 操作仍保持可用帧率。
- 所有布局变更仍经过 Server 权威状态确认，GUI 缓存不能成为真相来源。

### M2：Headless Server 与稳定 Client API

**核心交付**

- 发布独立的 `condr-server`：Linux x64/arm64、Windows Server 构建产物、checksum/signing、配置、数据目录、日志、health/status、systemd 或 Windows service。
- 明确升级、回滚、备份和 Server identity 的持久化位置。
- 从当前实现中拆出可复用的 `condr-protocol` 和 `condr-client` 边界；Server 保留 Runtime，GUI 不再反向定义协议。
- 在 Hello/Welcome 和语义 API 中加入 client kind、capabilities、typed errors、幂等 request ID、event cursor、权限上下文和 controller lease。
- 先提供 CLI 只读骨架：Server、Session、Workspace、Tab、Pane 的 `list/status`，统一 `--json`、稳定退出码、超时和诊断信息。

**协议原则**

当前 `bincode + serde` 适合作为 native 内部传输，但不要把当前 Rust enum 编码形式当作跨语言公共 ABI。公开后应区分 wire codec、语义 API、capability 和 build version；破坏性变更升 major，增量能力通过协商。

**退出条件**

- 一台干净 Linux 机器可以在无 GUI 的情况下安装、启动、重启和恢复 Server。
- GUI 与 CLI 可以共享同一客户端状态机，并能清楚区分版本不兼容、未授权和运行时错误。
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

### M4：CLI 与 Agent Workflow

**实施顺序**

1. CLI 查询和只读观察。
2. Session/Workspace/Tab/Pane 生命周期。
3. `send`、`capture`、`wait agent`、`attach`、取消和失败策略。
4. 多 Pane/worktree 的声明式工作流。
5. 在 CLI/API 之上提供 Agent Profile 和 Skill。

Agent Profile 使用声明式 manifest 描述 executable、argv、环境、启动模板、图标和状态检测规则。先验证 2～3 个代表性 CLI；未知 CLI 仍然可以手动启动和使用。

Skill 是调用 Condr CLI/API 的受限工作流模板，不是新的对话循环。每个 Skill 声明所需 capability，对启动进程、输入、文件和网络操作提供显式确认。

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

这条线从 M0 开始并行，但投入强度随产品成熟度增加：

- M0：Quickstart、安装、支持矩阵、远程安全边界、已知限制、故障排查和 canonical demo。
- M2：Server 运维、数据目录、升级/回滚、版本兼容矩阵、录屏和部署示例。
- 公共 Beta：官网、品牌/图标/截图和文案统一，CHANGELOG、SECURITY.md、贡献指南、Code of Conduct、Issue/Discussion 模板和发布节奏。
- Hosted Relay、账号体系、团队协作、企业 SSO/RBAC 和云端数据存储不在早期宣传中承诺。

首发内容应围绕一个可演示闭环：同一项目的多个 worktree/Agent、GUI 断开后继续运行、重新连接后恢复，而不是堆叠营销功能。

## 持续指标

不只看 GitHub stars，至少记录以下指标。诊断数据默认不包含 Terminal 内容，并采用 opt-in：

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

1. 完成 M0 发布信任基础：结构化日志、Server health/status、故障诊断、PTY 孤儿排查信息和 terminal burst 基准。
2. 完成 Quickstart、支持矩阵、已知限制与 canonical dogfood，冻结 Private Alpha 候选版本。
3. M0 退出后并行启动 M1 的 `Command Registry`、最小设置和高频 Pane 工作流，以及 M2 的 `condr-protocol` / `condr-client` 拆分和 Server 发布包。
4. 编写 Remote threat model、配对/授权 ADR 和公开协议兼容策略，为 M3 建立安全门。
5. 为 Relay、Web、Mobile 建立需求验证任务；在达到 Go/No-Go 条件前不进入完整实现。

建议将这些内容拆成带有 deliverable、退出条件和非目标的里程碑 issue，并用依赖关系表达 `protocol/client -> server packaging -> auth -> relay/web` 与 `client API -> CLI -> Skill` 两条链，避免把愿望清单直接变成无边界 backlog。
