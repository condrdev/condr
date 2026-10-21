# Companion 推送：同类产品、请求认证与内容加密

日期：2026-10-21。本文合并推送相关的五份调研，是 ADR 0047（`docs/adr/0047-pushes-reach-the-companion-through-condr-cloud.md`）的依据，不是 ADR。未核实之处照原调研标注。

## 结论

- **同类产品都推明文。** Paseo、Happy、Omnara 经 Expo 推送，标题和正文对 Expo、Apple、Google 可见；Superset 没有移动推送。Paseo 最简单：守护进程匿名 POST 到 Expo，「知道 token 就能推」，没有账号也没有厂商后端。
- **成熟中转只认证调用方，不认证请求。** VAPID 用自签短时 JWT，Bitwarden 用 24 小时 bearer，ntfy、Sygnal、Home Assistant 干脆以推送 token 本身为凭据；没人签 body 或路径，TLS 内的重放都被接受，滥用靠作用域和限速约束。Condr Cloud 照做：调用方用自己的 device key 自签 EdDSA JWT（`kid` = Device id，`exp` 5 分钟），服务端不发任何凭据，不用 RFC 9421。
- **Condr Cloud 有状态。** 它存手机注册和每台手机允许推送的 Server（VAPID / Bitwarden 式作用域），按调用方限速。库都现成：Java 侧 jjwt 0.13 + JDK 自带 Ed25519 验签，不新增依赖；限速用 Bucket4j（`bucket4j_jdk17-core` + Caffeine）；APNs 用 Pushy（定案，本文未单独调研）。Rust 侧用已有的 `ed25519-dalek`、`base64`、`serde_json` 手工拼 JWS。
- **内容加密可行但不划算。** 要给 iOS 加 Notification Service Extension、扩展与 App 共享 keychain、专用 X25519 key 和 padding；做端到端加密的服务主流是发不带内容的推送再让 App 去取，而 Companion 后台没有连接可取。所以推送是明文，没有 NSE。

## 1. 同类产品怎么推送

| 产品 | 移动推送 | 链路 | 有状态/无状态 | 内容 |
| --- | --- | --- | --- | --- |
| Paseo | 有 | 守护进程匿名 POST Expo → APNs/FCM，不经厂商 | 无状态（token 存本机，经 Expo） | 明文：Agent 回复预览 ≤220 字、`cwd` |
| Happy | 有 | CLI → happy-server → Expo → APNs/FCM | 有状态（推断：服务器按账号存 token） | 明文：会话摘要、工具名 |
| Omnara（旧版） | 有，已删 | 自营后端 → Expo → APNs/FCM | 有状态（推断：同上） | 明文：提问原文 ≤100 字 |
| t3code | 有 | environment → T3 Connect relay → APNs/FCM 直连 | 有状态（Clerk 账号 + environment 链接） | 明文：thread 标题、状态、project 名 |
| Orca | 有 | 桌面 → push.onorca.dev → APNs/FCM 直连 | 有状态（按主机密钥存 token，无账号） | 明文：标题 ≤80、正文 ≤180 字，可含 Agent 回复 |
| Superset | 无 | 只有桌面 Electron 通知 | — | — |
| Vibe Kanban、Crystal | 无 | 只有桌面通知 | — | — |

Paseo、Happy、Omnara 把内容明文交给 Expo，再由 Expo 转 APNs/FCM；t3code、Orca 用自营网关直连 APNs/FCM，也是明文。没有一家做应用层加密，也没找到解密推送的 NSE。Condr 推 `detail`、Workspace、终端标题和 Server 名，不推终端内容、`cwd` 或工具名，暴露面不超过这几家。

### Paseo

[getpaseo/paseo](https://github.com/getpaseo/paseo)（HEAD `5d70bf0`）：用户本机的守护进程直接、匿名地 POST 到 Expo Push，不经厂商后端，也不直接碰 APNs/FCM。同组织的 [paseo-relay](https://github.com/getpaseo/paseo-relay) 只转守护进程连接，推送不经过它。

- **调用**：`PushService` POST 到 `https://exp.host/--/api/v2/push/send`，每批最多 100 条；Agent finished / permission / error 或终端需要关注时发，先用 `computeNotificationPlan` 判断有没有客户端正在看。[push-service.ts](https://github.com/getpaseo/paseo/blob/main/packages/server/src/server/push/push-service.ts)、[websocket-server.ts](https://github.com/getpaseo/paseo/blob/main/packages/server/src/server/websocket-server.ts)
- **认证：没有。** 请求头只有 `Content-Type` 和 `Accept`，没有 Expo access token、账号或签名。Expo 默认只要知道 ExpoPushToken 就能发，项目开启 "enhanced push security" 后才要 access token，而这个 token 没法安全地分给每个用户的守护进程（Paseo 的项目设置未核实）。回执 `DeviceNotRegistered` / `InvalidCredentials` 时撤销 token（`handleTickets`）。
- **token 去向**：手机经已有的守护进程连接发 `register_push_token { token }`（[messages.ts](https://github.com/getpaseo/paseo/blob/main/packages/protocol/src/messages.ts)），守护进程以 48 小时租约（`PUSH_TOKEN_LEASE_MS`）存到本机 `$PASEO_HOME/push-tokens.json`，发送时群发给所有未过期 token。[push/index.ts](https://github.com/getpaseo/paseo/blob/main/packages/server/src/server/push/index.ts)、[session.ts](https://github.com/getpaseo/paseo/blob/main/packages/server/src/server/session.ts)
- **内容：明文**，Expo、Apple、Google 都看得到全文。`{ to, title, body, data, sound }`；`title` 是 "Agent finished" / "Agent needs permission" / "Agent needs attention"，`body` 是 Agent 最后一条回复或权限请求的预览，截断到 220 字符；终端通知的 `data` 带本机 `cwd`。[agent-attention-notification.ts](https://github.com/getpaseo/paseo/blob/main/packages/protocol/src/agent-attention-notification.ts)

### Happy

[slopus/happy](https://github.com/slopus/happy)（HEAD `b99e7f8`）：CLI/daemon → 自营 happy-server → Expo → APNs/FCM。

- CLI 用用户的 Bearer token 把 `kind`、`title`、`body`、`data` POST 到 `/v1/sessions/:id/push-event`，缺 sessionId 时才直连 Expo；服务器先看用户是否正在前台看 UI，再 POST 到 Expo。[pushNotifications.ts](https://github.com/slopus/happy/blob/b99e7f83a4162b2ab6ed999fafa2398f9cc1d237/packages/happy-cli/src/api/pushNotifications.ts)、[pushDispatch.ts](https://github.com/slopus/happy/blob/b99e7f83a4162b2ab6ed999fafa2398f9cc1d237/packages/happy-server/sources/app/push/pushDispatch.ts)
- **内容明文**：`title` 固定（"It's ready!" / "Permission request" / "Clarification needed"），`body` 是 Agent 生成的会话摘要（`metadata.summary.text`），没有时取工作目录最后一段；权限请求的 `data` 还带工具名。[permissionHandler.ts](https://github.com/slopus/happy/blob/b99e7f83a4162b2ab6ed999fafa2398f9cc1d237/packages/happy-cli/src/claude/utils/permissionHandler.ts)
- README 宣称移动端「端到端加密」、happy-server 是 "the relay server for encrypted sync"，但推送这条路径不例外，摘要和工具名对 Happy 服务器、Expo、Apple、Google 都可见。`happy-ios` 未逐一检查。

### Omnara

[omnara-ai/omnara](https://github.com/omnara-ai/omnara) 已转型为 "The open-source managed agent platform"，Go 重写后没有移动推送代码。它还是 agent 伴侣应用时（2025-10-01 前最后一个提交 `fa80862`），自营后端用 `exponent_server_sdk` 发 Expo：提问通知 `title` 为 `"{Agent 名} needs your input"`，`body` 是提问原文截断到 100 字符；步骤通知带步骤描述原文。Agent 消息本来就明文存在后端。[notifications.py@fa80862](https://github.com/omnara-ai/omnara/blob/fa808622ed5eead79cfdb4eb6411bff4e76ab722/src/servers/shared/notifications.py)

### Superset

[superset-sh/superset](https://github.com/superset-sh/superset)（2026-10-21）：**没有移动推送。** `expo-notifications` 只出现在 [apps/mobile/app.config.ts](https://github.com/superset-sh/superset/blob/main/apps/mobile/app.config.ts) 与 `package.json`，且配置为 `enableBackgroundRemoteNotifications: false`；代码搜索 `exp.host`、`expo-server-sdk`、`pushToken`、`getExpoPushTokenAsync` 零结果。现有通知都是桌面本地通知，`apps/api` 里的 push 路由是 Gmail、Teams 等第三方集成。结论基于代码搜索，未逐文件核实。

复查（HEAD `aca8060`，2026-10-21，本地克隆全文 grep）：结论不变。`apps/mobile` 的源码没有一处 import `expo-notifications`，只在 [app.config.ts#L165](https://github.com/superset-sh/superset/blob/aca8060379e7b00fe7b57c3c2173856c80290af0/apps/mobile/app.config.ts#L165) 声明插件；`apps/api`、`apps/relay`、`apps/realtime` 没有 APNs、FCM、Expo Push 或 Web Push 代码。通知只有桌面端 Electron 的 `new Notification`（[main.ts#L198-L200](https://github.com/superset-sh/superset/blob/aca8060379e7b00fe7b57c3c2173856c80290af0/apps/desktop/src/main/windows/main.ts#L198-L200)），不经任何服务端。

### t3code

[pingdotgg/t3code](https://github.com/pingdotgg/t3code)（HEAD `74541ba`，2026-10-21）：**有，且是有状态的，结构与 Condr Cloud 最像。** Expo 写的 iOS/Android App，推送走自营的 T3 Connect relay（Cloudflare Worker + PlanetScale Postgres），relay 直连 APNs（`api.push.apple.com`）和 FCM v1，不经 Expo Push：App 用 `getDevicePushTokenAsync` 取原生 token。也做 iOS Live Activity / Android 常驻通知。直连或 Tailscale 连接拿不到后台推送，必须登录 T3 Connect（Clerk 账号）。[mobile-notifications.md](https://github.com/pingdotgg/t3code/blob/74541bab17270c35f4418d4e543d85628632ed8f/docs/user/mobile-notifications.md)、[relay README](https://github.com/pingdotgg/t3code/blob/74541bab17270c35f4418d4e543d85628632ed8f/infra/relay/README.md)、[ApnsClient.ts#L241](https://github.com/pingdotgg/t3code/blob/74541bab17270c35f4418d4e543d85628632ed8f/infra/relay/src/agentActivity/ApnsClient.ts#L241)

- **relay 存什么**：`relay_mobile_devices`（按 Clerk `userId` 存 APNs/FCM token、Live Activity token、通知偏好）、`relay_environment_links`（用户与 environment 的链接、environment 公钥、`notifications_enabled`）、`relay_environment_credentials`（environment 凭据的哈希），还有每个 thread 的最新活动状态 `relay_agent_activity_rows`（明文 JSON，用来拼 Live Activity）。推送时按 environment 找出所有链接了它的用户，再发给这些用户的设备。[schema.ts#L20-L166](https://github.com/pingdotgg/t3code/blob/74541bab17270c35f4418d4e543d85628632ed8f/infra/relay/src/persistence/schema.ts#L20-L166)、[AgentActivityPublisher.ts#L230](https://github.com/pingdotgg/t3code/blob/74541bab17270c35f4418d4e543d85628632ed8f/infra/relay/src/agentActivity/AgentActivityPublisher.ts#L230)
- **认证**：手机注册用 Clerk 身份 + DPoP（scope `mobile:registration`）。environment 发布双重认证：一是 relay 发给它的 bearer 凭据（库里只存哈希），二是用 environment 私钥签的 JWT（`iss = t3-env:<id>`，`aud` = relay，带 `exp`），JWT 里含整份 `state`，relay 比对 `stableStringify(proof.state)` 与请求体，再消费 nonce 防重放。**这是第 2 节那五家都没有的做法：签了 body，也防了重放。** [Api.ts#L1193-L1214](https://github.com/pingdotgg/t3code/blob/74541bab17270c35f4418d4e543d85628632ed8f/infra/relay/src/http/Api.ts#L1193-L1214)、[EnvironmentPublishSignatures.ts](https://github.com/pingdotgg/t3code/blob/74541bab17270c35f4418d4e543d85628632ed8f/infra/relay/src/environments/EnvironmentPublishSignatures.ts)
- **限速**：relay 只对 webhook 转发配了 Cloudflare RateLimit（[worker.ts#L189](https://github.com/pingdotgg/t3code/blob/74541bab17270c35f4418d4e543d85628632ed8f/infra/relay/src/worker.ts#L189)），发布接口没找到限速（未核实）。
- **内容：明文**，不加密。`title` 是 thread 标题，`body` 是 `"{status}: {project 标题}"`，每段截断到 120 字符，另带 `deepLink`。relay、Apple、Google 都看得到。[agentActivityPayloads.ts#L106-L118](https://github.com/pingdotgg/t3code/blob/74541bab17270c35f4418d4e543d85628632ed8f/infra/relay/src/agentActivity/agentActivityPayloads.ts#L106-L118)

### Orca

确认是 [stablyai/orca](https://github.com/stablyai/orca)（HEAD `bbc8303`，2026-10-21）："The AI Orchestrator"，Codex、Claude Code、OpenCode 各占一个 worktree 并排运行，有 iOS（App Store）和 Android（APK）Companion。**有移动推送，有状态，但不要账号：** 桌面端替手机注册。[README.md#L35-L39](https://github.com/stablyai/orca/blob/bbc8303b47ff6985b3065bea9627f9e2e398e0a8/README.md#L35-L39)

- **链路**：手机经已配对的 relay 连接把原生 APNs/FCM token 交给桌面，桌面再注册到自营的 `push.onorca.dev`（Cloud Run + Postgres），并在每次桌面通知后调 `POST /v1/send`；网关直连 APNs/FCM，不经 Expo Push，持有 Apple `.p8` 是它存在的唯一理由。`push_devices` 存 `host_fingerprint`、`device_id`、平台、token；发送时只查 `host_fingerprint = 当前会话主机` 的注册，即「谁注册的谁才能推」。[cloud/README.md](https://github.com/stablyai/orca/blob/bbc8303b47ff6985b3065bea9627f9e2e398e0a8/cloud/README.md)、[push-gateway.md#L1-L9](https://github.com/stablyai/orca/blob/bbc8303b47ff6985b3065bea9627f9e2e398e0a8/cloud/docs/push-gateway.md#L1-L9)、[push-schema.ts#L23-L30](https://github.com/stablyai/orca/blob/bbc8303b47ff6985b3065bea9627f9e2e398e0a8/cloud/apps/push/src/push-schema.ts#L23-L30)、[device-registry-store.ts#L143](https://github.com/stablyai/orca/blob/bbc8303b47ff6985b3065bea9627f9e2e398e0a8/cloud/apps/push/src/device-registry-store.ts#L143)
- **认证**：桌面用连 relay 的同一把 X25519 主机密钥应答加密挑战（`POST /v1/host/challenge`），换 24 小时 bearer 会话（库里存哈希）；主机密钥由桌面自生成、没有账号（推断：文档说「换主机密钥」重置不了 IP 预算）。请求体不签名。
- **防滥用**：每个 `hostFingerprint` 每 15 分钟 300 条提醒、300 条撤回，每次最多 20 个 `registrationIds`；未认证握手和错误 bearer 每 IP 30 次/分钟；已认证请求每主机 600 次/分钟，另有每实例每 IP 6000 次/分钟，文档明说「换主机密钥也重置不了 IP 预算」。日志只记计数，不记 token、标题、正文。[push-gateway.md#L319-L343](https://github.com/stablyai/orca/blob/bbc8303b47ff6985b3065bea9627f9e2e398e0a8/cloud/docs/push-gateway.md#L319-L343)
- **内容：明文**，网关、Apple、Google 可见。`title` ≤ 80、`body` ≤ 180 字符，取自桌面通知，如 `Task complete in {worktree}`，正文可含 Agent 的 prompt、工具名或最后一条回复；另带 `worktreeId`。payload 五分钟内过期删除。[send-messages.ts#L27-L28](https://github.com/stablyai/orca/blob/bbc8303b47ff6985b3065bea9627f9e2e398e0a8/cloud/packages/push-contract/src/send-messages.ts#L27-L28)、[notification-options.ts#L110-L154](https://github.com/stablyai/orca/blob/bbc8303b47ff6985b3065bea9627f9e2e398e0a8/src/main/ipc/notification-options.ts#L110-L154)

### Vibe Kanban 与 Crystal

- **Vibe Kanban**（[BloopAI/vibe-kanban](https://github.com/BloopAI/vibe-kanban)，HEAD `d5cbb53`，2026-09-19，README 已宣布停止维护）：**没有手机 App，也没有移动推送或 Web Push。** 代码里叫 `PushNotifier` 的只是本机桌面通知：默认调各平台 OS 命令，Tauri 版换成原生通知 API（[notification.rs#L10-L53](https://github.com/BloopAI/vibe-kanban/blob/d5cbb5380fa0b32e98ef9b8d987f63decce4be3a/crates/services/src/services/notification.rs#L10-L53)）。手机只能经 Tailscale + Caddy 打开 remote-web 前端（[mobile-testing.md](https://github.com/BloopAI/vibe-kanban/blob/d5cbb5380fa0b32e98ef9b8d987f63decce4be3a/mobile-testing.md)）。全库 grep APNs、FCM、Expo、VAPID、`PushManager` 均无结果。
- **Crystal**（[stravu/crystal](https://github.com/stravu/crystal)，HEAD `1e18e0b`，2026-02-26，已更名 Nimbalyst）：**没有移动推送**，只有 Electron 渲染进程的 HTML5 `new Notification`（[useNotifications.ts#L92](https://github.com/stravu/crystal/blob/1e18e0bc981225f75b5226f82a300fa741970c6f/frontend/src/hooks/useNotifications.ts#L92)）。Nimbalyst 是否有移动端未核实。
- Conductor 闭源，Claude Squad 是 TUI，Sculptor 未看，都未核实。

## 2. 成熟中转的请求认证

五家都只认证「谁在调」，不认证「这条请求」：没人签 body 或路径，完整性交给 TLS，内容靠端到端加密或根本不经中转；TLS 内的重放都被接受，窗口还远大于 5 分钟。防滥用靠作用域和限速。

| 中转 | 调用方凭据 | 有效期 | 作用域 | 限速 |
| --- | --- | --- | --- | --- |
| VAPID | 自签 ES256 JWT | ≤24 小时 | 订阅只认一把公钥 | 未规定 |
| Bitwarden | OAuth bearer 令牌 | 24 小时 | id 加 installation 前缀 | 按 IP |
| ntfy | 无（可选 bearer） | 不过期 | topic = URL 的 SHA-256 | 按 IP |
| Sygnal | 无 | 不过期 | pushkey + `app_id` | 只有资源上限 |
| Home Assistant | 无 | 不过期 | FCM 令牌本身 | 每个目标令牌每天 500 条 |

### VAPID（RFC 8292）

Web Push 里应用服务器向推送服务证明身份，同样没有账号，只有一对密钥，和 Server → Condr Cloud 最像。

- `Authorization: vapid t=<JWT>, k=<公钥>`，JWS 必须是 ES256。[§2](https://www.rfc-editor.org/rfc/rfc8292.html#section-2)、[§3](https://www.rfc-editor.org/rfc/rfc8292.html#section-3)
- 声明只有 `aud`（推送服务的 origin，必需）、`exp`（必需，"MUST NOT be more than 24 hours from the time of the request"）和可选的 `sub` 联系地址；没有 `iat`、`jti` 或 nonce。推送服务把已过期或 `exp` 超过 24 小时的令牌视为无效。[§2](https://www.rfc-editor.org/rfc/rfc8292.html#section-2)、[§4.2](https://www.rfc-editor.org/rfc/rfc8292.html#section-4.2)
- 令牌刻意可复用：`aud` 只到 origin，好让推送服务缓存验签结果。RFC 明说 "vulnerable to replay attacks if an attacker can acquire a valid JWT"，缓解只有 HTTPS 和短 `exp`。[§5](https://www.rfc-editor.org/rfc/rfc8292.html#section-5)
- 不签 body 的理由 RFC 没写；从结构推断是令牌要复用，且 body 另由 RFC 8291 端到端加密，签名密钥与加密密钥必须不同。[§3.2](https://www.rfc-editor.org/rfc/rfc8292.html#section-3.2)
- 授权模型是 restricted subscription：订阅时把应用服务器公钥交给推送服务，此后只认这把私钥签的令牌。[§4](https://www.rfc-editor.org/rfc/rfc8292.html#section-4) 这就是 Companion 注册时列出允许的 Server id。

### Bitwarden 自托管 → 推送中继

源码固定在 [`d6efd3b`](https://github.com/bitwarden/server/tree/d6efd3b745c3a6d13afd5a8d24ddcd5bb637819f)。

- 管理员在官网申请 Installation Id/Key 写进配置，自托管实例用 OAuth client credentials 换 24 小时 bearer 令牌（scope `api.push`），快过期才重新申请。服务端只存 Key 的 SHA-256，可一键停用某个实例。[RelayPushEngine.cs](https://github.com/bitwarden/server/blob/d6efd3b745c3a6d13afd5a8d24ddcd5bb637819f/src/Core/Platform/Push/Engines/RelayPushEngine.cs)、[InstallationClientProvider.cs](https://github.com/bitwarden/server/blob/d6efd3b745c3a6d13afd5a8d24ddcd5bb637819f/src/Identity/IdentityServer/ClientProviders/InstallationClientProvider.cs)
- 租户隔离靠命名空间：调用方传来的 device / user / org id 一律加 `"{InstallationId}_"` 前缀，一个实例碰不到别家的注册。[PushController.cs](https://github.com/bitwarden/server/blob/d6efd3b745c3a6d13afd5a8d24ddcd5bb637819f/src/Api/Platform/Push/Controllers/PushController.cs)
- 默认按 IP 限速，`post:*` / `put:*` / `delete:*` 每分钟 60 次、每秒 5 次。[Api/appsettings.json](https://github.com/bitwarden/server/blob/d6efd3b745c3a6d13afd5a8d24ddcd5bb637819f/src/Api/appsettings.json)（生产值未核实）

### ntfy 自托管 → ntfy.sh

源码固定在 [`5e11f4f`](https://github.com/binwiederhier/ntfy/tree/5e11f4fc344202eb4c9310bf67f17ca121815290)。iOS 限制后台活动，自托管服务器要经 ntfy.sh 由 APNs 唤醒手机，否则 "delivery can take hours"。[配置文档](https://docs.ntfy.sh/config/#ios-instant-notifications)

- 上游请求是 `POST {upstream}/{sha256(topicURL)}`，body 为空，只带 `X-Poll-ID`；上游推固定文本 "New message"，手机回自托管服务器拉内容。[server.go `forwardPollRequest`](https://github.com/binwiederhier/ntfy/blob/5e11f4fc344202eb4c9310bf67f17ca121815290/server/server.go#L1212-L1242)
- 默认不认证，只在配置 `upstream-access-token` 时加 bearer（"optional, only if rate limits exceeded, or upstream server protected"）。安全靠 topic 不可猜、伪造也拿不到内容（推断）。
- 按 visitor（默认 IP）令牌桶限速：突发 60、每 5 秒补 1。[Rate limiting](https://docs.ntfy.sh/config/#rate-limiting)

### Matrix homeserver → Sygnal

源码固定在 [`4700184`](https://github.com/matrix-org/sygnal/tree/4700184bac220125a51fed7ece428e233fb0c824)。

- `POST /_matrix/push/v1/notify` 标注 "Requires authentication: No"、"Rate-limited: No"。[Push Gateway API](https://spec.matrix.org/latest/push-gateway-api/#post_matrixpushv1notify) Sygnal 按 `app_id` 找 pushkin 直接投递，不检查调用方。[http.py](https://github.com/matrix-org/sygnal/blob/4700184bac220125a51fed7ece428e233fb0c824/sygnal/http.py#L250-L275)
- 只有资源上限：请求体 512 KiB，每个 pushkin 在途请求 512。规范要求按 `event_id` 去重，防的是重试造成的重复提醒，不是安全上的重放。

### Home Assistant → mobile-apps-fcm-push

中继 [`7220046`](https://github.com/home-assistant/mobile-apps-fcm-push/tree/722004671420023901de7d68d4bcbc5cda6abd8a)（Firebase Cloud Functions），调用方 [core `a38890c`](https://github.com/home-assistant/core/tree/a38890c0da2fd9cf085ee60e1e931f997a3ce34f)。

- HA 实例 POST 到手机注册时给的 `push_url`，不带 `Authorization` 或签名。[notify.py](https://github.com/home-assistant/core/blob/a38890c0da2fd9cf085ee60e1e931f997a3ce34f/homeassistant/components/mobile_app/notify.py#L264-L290) 中继唯一的检查是 body 里的 `push_token` 像 FCM 令牌，FCM 令牌本身就是能力凭据。[handlers.js](https://github.com/home-assistant/mobile-apps-fcm-push/blob/722004671420023901de7d68d4bcbc5cda6abd8a/functions/handlers.js#L56-L66)
- 按目标令牌限速：默认每天 500 条，计数存 Valkey 或 Firestore；超限回 429 并附剩余额度，首次超限还给手机推一条提示。[handlers.js](https://github.com/home-assistant/mobile-apps-fcm-push/blob/722004671420023901de7d68d4bcbc5cda6abd8a/functions/handlers.js#L5-L100)

### 对 Condr Cloud 的含义

- 签名形式不是重点。VAPID 形状的自签 JWT 已经和同行一致；签方法、路径和 body 的现成标准只有 RFC 9421 + RFC 9530 `Content-Digest`，五家都没用。
- 5 分钟 `exp` 比所有先例都严，不需要 nonce 缓存；被截获的令牌在有效期内能配任意请求重放，最坏是垃圾推送或改写调用方自己的注册。
- 真正的防滥用照搬两条：Bitwarden / VAPID 式作用域（Server 只能推给列了它的手机），Home Assistant 式按收件方限速。

## 3. 认证的库

condr-cloud 是 Spring Boot 4.1.1 / Spring Security 7 / Java 25，已经依赖 jjwt 0.13.0（只在 `framework` 模块），没有 `spring-boot-starter-oauth2-resource-server`。Condr 已经通过 iroh 依赖 `ed25519-dalek` 3.0.0，另有 `base64` 0.22、`serde_json`、`reqwest` 0.13。三种候选中只有 EdDSA JWT 两端都不加依赖：

| | EdDSA JWT | RFC 9421 | 自定义签名头 |
| --- | --- | --- | --- |
| condr-cloud | jjwt + JDK Ed25519，零新依赖 | Spring Security 不支持，Java 库小众 | 全部自写 |
| Condr | 现有 crate 拼 JWS，零新依赖 | `httpsig` 0.0.x，自带另一份 Ed25519 | 全部自写 |
| 标准 | [RFC 7519](https://www.rfc-editor.org/rfc/rfc7519)、[RFC 8037](https://www.rfc-editor.org/rfc/rfc8037)，VAPID 先例 | RFC 9421 | 无 |
| 绑定请求内容 | 否，只绑 `aud` / `exp` | 方法、路径、摘要 | 自定 |

### Java：jjwt + JDK Ed25519

- JDK 15 起内置 Ed25519（[JEP 339](https://openjdk.org/jeps/339)），Java 25 不需要 BouncyCastle。
- jjwt 0.13.0 是 Maven Central 最新版，`Jwts.SIG.EdDSA` 原生支持 Ed25519，密钥类型是 JDK 的 `EdECKey`。[README：Edwards Curve](https://github.com/jwtk/jjwt#jws-key-eddsa)
- 按请求定位公钥用 `Jwts.parser().keyLocator(...)`，它在验签前调用，**只看得到 header**，所以 Device id 放 `kid`。[README：Key Locator](https://github.com/jwtk/jjwt#key-locator) Device id 本身就是 RFC 8037 OKP JWK 的 `x`，交给 `Jwks.parser()` 解析 `{"kty":"OKP","crv":"Ed25519","x":…}` 即得公钥，不用手写 DER 前缀。[README：Ed public JWK](https://github.com/jwtk/jjwt#example-jwk-edpub)
- Spring 自带的 `NimbusJwtDecoder` 不合适：`SignatureAlgorithm` 枚举没有 EdDSA，`withPublicKey` 只收 RSA 公钥（[7.0.x 源码](https://github.com/spring-projects/spring-security/blob/7.0.x/oauth2/oauth2-jose/src/main/java/org/springframework/security/oauth2/jwt/NimbusJwtDecoder.java)）；nimbus 的 `Ed25519Verifier` 还要自己加可选依赖 Tink，且默认 verifier 工厂不含 EdDSA。

### condr-cloud 的过滤器链

- 模板的认证链是自写的：`JwtHeaderTokenExtractor` 只认 `Authorization: Bearer`，`JwtAuthenticationProvider` 用配置里的单把 HMAC/RSA 密钥验签、按 `sub` 查用户表。这些不能复用于 Device。
- 加第二条 `SecurityFilterChain`，用 `securityMatcher` 限定路径、`@Order` 排在模板那条无 matcher 的链前面。[Multiple HttpSecurity Instances](https://docs.spring.io/spring-security/reference/servlet/configuration/java.html#_multiple_httpsecurity_instances) 可复用的只有 EntryPoint、AccessDeniedHandler 和 STATELESS / 关 CSRF 的写法，不要调 `applyDefaultSecurity(http)`。
- 坑（据代码推断，未运行验证）：`JwtTokenFilter` 是 bean，Spring Boot 会把它自动注册成 Servlet 过滤器，在不含它的链上也会跑；凭据若用 `Bearer`，它验签失败后 `clearContext()`。[Spring Boot：Filters as beans](https://docs.spring.io/spring-boot/reference/web/servlet.html#web.servlet.embedded-container.servlets-filters-listeners.beans) 所以 scheme 用 `Condr` 而不是 `Bearer`。

### RFC 9421 库现状

规范 2024-02 发布，`ed25519` 是注册算法（§3.3.6），签名基串依赖 [RFC 8941](https://www.rfc-editor.org/rfc/rfc8941) 结构化字段，两端都要正确序列化。

- **Spring Security 不支持**：[#18502](https://github.com/spring-projects/spring-security/issues/18502)（2026-01）仍是 `waiting-for-triage`。
- Java 库：`io.bspk:httpsig` 1.0.0（RFC 作者 Justin Richer，14 星，最后更新 2025-05），依赖 nimbus、BouncyCastle、Guava、`spring-web`，是否适配 Jakarta / Boot 4 未核实；`aauth-signing` 0.3.0 要 Java 26+；`dev.bluestep:http-signatures` 在 Maven Central 查不到。实际多半要按 §2.5 自己拼基串。
- Rust 库：[`httpsig`](https://crates.io/crates/httpsig/0.0.26) 0.0.26，用 `ed25519-compact`，还拉预发布的 `rsa 0.10.0-rc`；Cloudflare 的 [`web-bot-auth`](https://crates.io/crates/web-bot-auth/0.7.0) 0.7.0 面向 Web Bot Auth，带 dalek 2.1.1。都是 0.x，都不与 Condr 的 dalek 3.0.0 共用。

### Rust：手工构造 JWS

- EdDSA 的 JWS Compact 是 `base64url(header).base64url(payload).base64url(Ed25519(前两段 ASCII))`。[RFC 7515 §5.1](https://www.rfc-editor.org/rfc/rfc7515#section-5.1)、[RFC 8037 §3.1](https://www.rfc-editor.org/rfc/rfc8037#section-3.1) 用已有的 `ed25519-dalek`（或 `iroh_base::SecretKey::sign`）、`base64`（`URL_SAFE_NO_PAD`）、`serde_json`，十来行出标准 JWT，Java 侧照常用 jjwt 验，没有自造协议。
- 不选 [`jsonwebtoken`](https://crates.io/crates/jsonwebtoken/11.1.0) 11.1.0：`rust_crypto` 后端带进 dalek 2.1.1 和整套 RSA/ECDSA，`aws_lc_rs` 后端虽能共用锁里的 aws-lc-rs 1.18.1，但签名要 PKCS#8 DER，32 字节种子还得先包一层；Condr 只签不验，用不上它。
- `alg` 写 `EdDSA`（RFC 8037）。[RFC 9864](https://www.rfc-editor.org/rfc/rfc9864.html#section-2.2)（2025-10）注册了全指定的 `Ed25519` 并弃用多态的 `EdDSA`；ADR 沿用 jjwt `Jwts.SIG.EdDSA` 对应的 `EdDSA`，jjwt 是否接受 `Ed25519` 未核实。

### Bucket4j

- 当前 8.21.0（2026-10-02），坐标 **`com.bucket4j:bucket4j_jdk17-core`**（"For java 17+"）；`bucket4j-core`、`_jdk8`、`_jdk11` 都已停更。[Release](https://github.com/bucket4j/bucket4j/releases/tag/8.21.0)、[README](https://github.com/bucket4j/bucket4j#readme) 对 Java 25 没有明确测试声明。
- 纯库，不依赖 Spring；Boot 4.1.1 的 BOM 不管 Bucket4j 版本（要自己写），但管 Caffeine（3.2.4）。[spring-boot-dependencies-4.1.1.pom](https://repo1.maven.org/maven2/org/springframework/boot/spring-boot-dependencies/4.1.1/spring-boot-dependencies-4.1.1.pom)
- 按 Device id 的内存桶：`Bucket.builder().addLimit(...).build()` 存进带过期的 Caffeine 缓存即可，JCache 和分布式模块只在多实例共享限额时才需要。
- 官方没有 Spring Boot 集成；第三方 [bucket4j-spring-boot-starter](https://github.com/MarcGiffing/bucket4j-spring-boot-starter) 0.14.0 已升 Boot 4，但它的卖点是 yaml 配置，而这里的 key 是认证后的 Device id，手写过滤器更直接。

## 4. 推送内容加密

### 主流：推送里不放内容

做端到端保密的服务大多不加密推送，而是让推送不带内容，手机回源去取。代价是 NSE（或 App）必须能联网直达服务器；Companion 在后台没有连接（ADR 0040），手机也未必连得上 Server（局域网、Peer-to-peer、SSH），所以这条路只能换来占位文案。

| 项目 | 推送里有什么 | 保密方式 | 手机取内容要联网 |
| --- | --- | --- | --- |
| Mastodon（Web Push） | 完整通知 JSON 的密文 | RFC 8291，中继原样转发 | 否，NSE 内解密 |
| Bitwarden | 只有 ID 和时间 | 推送里本来没秘密 | 是，回服务器同步 |
| ntfy 上游 | 消息 ID + topic 哈希，正文 "New message" | 内容不出自托管服务器 | 是，NSE 内 poll |
| Matrix / Sygnal | `event_id`、`room_id`、计数 | 规范建议不放内容 | 是，NSE 内取事件 |
| Delta Chat | 固定文案 | 只加密设备令牌 | 是，IMAP 收信 |

- **Bitwarden**：`SyncCipherPushNotification` 只带 `Id`、`RevisionDate` 等，`AuthRequestPushNotification` 只带 `UserId`、`Id`，手机收到再拉；较新的 `NotificationPushNotification` 带明文 `Title` / `Body`（是否经中继未核实）。[PushType.cs](https://github.com/bitwarden/server/blob/main/src/Core/Platform/Push/PushType.cs)
- **ntfy**：上游给 APNs 发 `poll_request` + `mutable-content`，iOS NSE 把 topic 哈希映射回订阅，在 NSE 里联网 `poll` 自托管服务器替换内容。[server_firebase.go](https://github.com/binwiederhier/ntfy/blob/b1eb90addc7dc392a1d91f46700ab92d14035f49/server/server_firebase.go#L105-L256)、[ntfyNSE](https://github.com/binwiederhier/ntfy-ios/blob/132c411b6345ca2b7bd8e4f53f5730bd2eb52a4b/ntfyNSE/NotificationService.swift#L34-L89)
- **Matrix**：pusher 的 `data.format` 只有 `event_id_only`；规范安全考量说 "message content shouldn't be sent in the push itself where possible"。[push.md](https://github.com/matrix-org/matrix-spec/blob/main/content/client-server-api/modules/push.md#security-considerations) Element X 的 NSE 拿 `eventID` 去 homeserver 取事件再解密。[NotificationServiceExtension.swift](https://github.com/element-hq/element-x-ios/blob/ac29354b08efae48c32c64e1ce55908eb0c82fa3/NSE/Sources/NotificationServiceExtension.swift#L142-L191)
- **Delta Chat**：推送是固定的 "New messages" + `collapse-id: new_messages`；它用硬编码的 OpenPGP 公钥加密的是设备令牌（先用空格补齐到固定长度），让第三方 chatmail 服务器拿不到令牌。[core push.rs](https://github.com/chatmail/core/blob/35555ca75306a2d0a7149db6c4c7183e9edc184f/src/push.rs#L35-L88)、[notifiers server.rs](https://github.com/chatmail/notifiers/blob/3957ae1ba091f9c59d48b84a0559329b5f22161a/src/server.rs#L418-L433)

### 唯一的离线解密先例：Web Push + Mastodon iOS

- RFC 8291：UA 每个订阅一对长期 P-256 密钥，应用服务器每条消息一对临时密钥，ECDH → HKDF-SHA256 → AES-128-GCM（RFC 8188 `aes128gcm`），外加一个 16 字节 `auth_secret` 防止只知道公钥的人伪造。明文最多 3993 字节；不保护头、时间、长度和收发双方，长度只有填充才藏得住。[§3](https://www.rfc-editor.org/rfc/rfc8291.html#section-3.1)、[§4](https://www.rfc-editor.org/rfc/rfc8291.html#section-4)、[§7](https://www.rfc-editor.org/rfc/rfc8291.html#section-7) 结构上就是 HPKE base 模式（判断，非规范原文）。
- Mastodon 中继 [webpush-apn-relay](https://github.com/mastodon/webpush-apn-relay/blob/81d1ac42cc8cff06b8a41e21ecae90f4c5d99aec/main.go#L188-L222) 把密文 base85 后原样塞进 APNs 自定义字段 `p`，可见部分固定是 `alert: "🎺"` + `mutable-content`；NSE 从 App 与扩展的共享存储读私钥和 `auth`，用 CryptoKit 手写 P256 协商、HKDF、AES.GCM 解开。[NotificationService+Decrypt.swift](https://github.com/mastodon/mastodon-ios/blob/main/NotificationService/NotificationService%2BDecrypt.swift) 解密不联网，但之后会下载头像。

### 若要加密：HPKE 的做法与库

草案曾是 HPKE（[RFC 9180](https://www.rfc-editor.org/rfc/rfc9180.html)）base 模式，DHKEM(X25519, HKDF-SHA256) / HKDF-SHA256 / ChaCha20-Poly1305，只加密 `detail` 和终端标题，Server 用 Rust `hpke` seal，iOS NSE 用 CryptoKit 离线 open。它和 Web Push 同构，方向合乎实践，比 Mastodon 手写 RFC 8188 省代码。

- **CryptoKit**：iOS 17.0 起提供 `HPKE.Ciphersuite.Curve25519_SHA256_ChachaPoly`（RFC 9180 KEM 0x0020 / KDF 0x0001 / AEAD 0x0003），Companion 下限 iOS 26（ADR 0042）。[HPKE](https://developer.apple.com/documentation/cryptokit/hpke)、[Ciphersuite](https://developer.apple.com/documentation/cryptokit/hpke/ciphersuite/curve25519_sha256_chachapoly) `HPKE.Recipient.init(privateKey:ciphersuite:info:encapsulatedKey:)` 接受 `Curve25519.KeyAgreement.PrivateKey`，后者可由 32 字节 `rawRepresentation` 还原（swift-crypto 校验 32 字节，设备上的 CryptoKit 未核实）。`enc` 是 32 字节临时公钥（RFC 9180 §7.1 Nenc=32），两端须用同一 `info`、AAD 和 base 模式。
- **Rust `hpke`**（[rozbb/rust-hpke](https://github.com/rozbb/rust-hpke)）：0.14.1（2026-09-06，GitHub 只有 `v0.14.0` tag），默认 feature 含 `x25519` 与 `chacha`，原语全来自 RustCrypto / dalek。**从未做过付费审计**：README 原文 "To the authors' knowledge, nobody has performed a paid audit of this crate"；唯一的外部审查是 Cloudflare 对 0.8 的内部审查，"we found no security issues"。[Cloudflare 博客](https://blog.cloudflare.com/using-hpke-to-encrypt-request-payloads/) 0.14 依赖 `aead` 0.6、`chacha20poly1305` 0.11、`hkdf` 0.13，与 Condr 锁里的 0.5 / 0.10 / 0.12 重复一套；0.13 与现有锁重合更多。缓解是 RFC 9180 测试向量加 Rust → CryptoKit 互通测试。
- **替代**：Cryspen 的 [`hpke-rs`](https://github.com/celabshq/libcrux/blob/main/crates/protocols/hpke/Readme.md) 0.8.0 有形式化验证的 libcrux 后端，但组合层没有公开审计、libcrux 后端是否覆盖 ChaCha20Poly1305 都未核实；rustls 为 ECH 暴露的 `aws_lc_rs::hpke` 要给 `condr-server` 加 aws-lc（C），它只启用了 `ring`。

### 专用 X25519 key 与 padding

- **不复用 Ed25519 device key。** 草案把手机 Ed25519 公钥按 libsodium 方式转成 X25519；没有先例这么做，Web Push 订阅专门生成一对 P-256 密钥和 `auth_secret`。libsodium 文档："using distinct keys for signing and for encryption is still highly recommended"。[libsodium](https://doc.libsodium.org/advanced/ed25519-curve25519) 同一密钥兼用有安全证明（[ePrint 2021/509](https://eprint.iacr.org/2021/509)，按记忆，未逐字核实），问题在别处：base 模式不认证发送方，知道接收方公钥就能造出解得开的密文，而 Condr Cloud 从 `kid` 就知道手机的 Ed25519 公钥，于是能伪造 `detail`。Web Push 的 `auth_secret` 正是防这个。
- 做法是 Companion 用 CryptoKit 生成独立的 `Curve25519.KeyAgreement` 密钥，私钥放 App 与 NSE 共享的 Keychain，公钥**只经已认证的 Server 连接**交给 Server，不给 Condr Cloud，兼起 `auth_secret` 的作用。
- **加固定长度 padding。** HPKE 不填充，RFC 8291 §7 明说不填充会泄露长度，Delta Chat 加密令牌前也补齐。≤200 字符的 `detail` 长度本身就是信息。`{title, detail}` 补齐到固定长度（如 512 字节）后 seal 一次，APNs 可见的 alert 用占位文案加 `mutable-content`，与 Mastodon 的 `"🎺"`、ntfy 的 "New message" 同形；NSE 解不开就保留占位文案。

### 为什么没做

加密要带来 NSE、App 与 NSE 共享 keychain、专用 X25519 key、padding、两端各一套 HPKE 和互通测试，保护的是锁屏上本来就显示给人看的文字；同类产品全推明文，主流的端到端方案是不带内容的推送，而 Companion 后台无连接可取。ADR 0047 因此定为明文推送、没有 NSE；Condr Cloud、Apple、Google 能看到 Agent 名与状态、Workspace、`detail`、终端标题、Server 名和两端 Device id，看不到终端内容。
