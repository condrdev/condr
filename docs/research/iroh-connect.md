# 基于 iroh 的远程连接可行性

日期：2026-09-21。本文是可行性调研与实现草案，不是 ADR。「架构决定」一节记录 2026-09-21 讨论后已定的方向；下一步写 ADR。Condr 对照 HEAD `4474031`。

## 结论

**可行。** iroh 可以作为第三种 Endpoint scheme（对外叫 `p2p://`）与 `tcp://`、`ssh://` 平行，复用现有 `authorized-clients` / invite / revoke 模型；它正好落在 roadmap 远期表的「Relay」一项上，只是 rendezvous 与打洞不再自建。最大代价是依赖体量：给 `condr-server` 新增约 157 个 crate，本机（arm64）全量编译多出约三分钟，GUI 与 CLI 都要链。

## iroh 现状（2026-09）

| 项 | 值 |
| --- | --- |
| 版本 | 1.2.0（2026-09-09）；1.0.0-rc.0 2026-05、1.0.3 2026-07、1.1.0 2026-08-25 |
| MSRV / License | 1.91（工作区 1.95）/ MIT OR Apache-2.0 |
| QUIC 实现 | n0 自家 fork `noq`（不是 quinn），rustls；`ed25519-dalek` 仍是 `3.0.0-rc` 预发布 |
| 运行时 | tokio 必需（非 optional） |
| 最小 features | `default-features = false, features = ["tls-ring"]`（关掉 metrics / portmapper） |
| 身份 | EndpointId = ed25519 公钥；TLS 双向认证；`Connection::remote_id()` 给出对端 key |
| 地址 | `EndpointAddr { id, relay_url, direct addrs }`；`presets::Minimal` 不接任何发现服务，`presets::N0` 接 n0 的 DNS/pkarr 发现（把 relay/直连地址发布到 n0 DNS） |
| Relay | 只转发加密 QUIC 包，看不到内容也不落盘；打洞成功后退出数据路径。n0 公共 relay 免费但限速（定位 dev/test，1.1 起客户端会收到限速提示）；Pro 计划有 Shared Relays；自建 `iroh-relay` 是一个二进制，支持 token 访问控制 |
| 平台 | Linux/macOS/Windows/wasm；Windows 未在本仓库实测 |

### 实测

`/tmp/iroh-probe`：空项目 `iroh = "1.2"`（`tls-ring` only）+ tokio。

- 全量编译 2m57s（arm64 Linux，debug）。
- 依赖树 296 个 crate，其中 62 个与 `condr-server` 现有的 310 个重合，**新增 157 个**（reqwest、rustls、iroh-dns、noq、netwatch、n0-* 等）。
- `Endpoint::builder(presets::Minimal).bind()` 可在无网络发现的情况下起来并打印 EndpointId。

## Condr 现有连接层（要接入的地方）

- `Endpoint { Local, Tcp, Ssh }`（`crates/condr-server/src/endpoint.rs`）：`parse` 按 scheme 分发；`connect()` 返回 `EndpointStream`。
- `EndpointStream`（`endpoint/stream.rs`）：阻塞 `Read + Write`，`try_clone` 供 reader/writer 两线程共享，另有 `shutdown`、`set_handshake_timeout`、`peer_key`、`peer_authorized`、`complete_pairing`、`record_peer_seen`、`may_administer`。
- `EndpointListener { Local, Tcp }`（`endpoint/local.rs`）：`Server::run_blocking` 轮询 `accept()`，每个连接一个线程 `handle_client`。
- `Server::run` 已创建 tokio multi-thread runtime，`spawn_blocking` 跑主循环。GUI 与 CLI 客户端侧没有 runtime。
- `ServerIdentity`（`noise/identity.rs`）：`is_authorized` / `complete_pairing` / `record_seen` 以 32 字节 `PublicKey` 工作，`authorized-clients` 一行 `key paired-at last-seen name`；invite 十分钟 TTL，first-wins；`identity.lock` 串行化。这套逻辑与 Noise 无关，可直接给 iroh 用。
- `NoiseStream` 硬绑定 `TcpStream`，不泛型。
- `SavedServer.address` 是字符串，`Endpoint::parse` 决定 scheme；`condr --device` 与 GUI Connect Remote Device 共用。

## 实现草案

### 地址与配对

`p2p://<endpoint id>[.<invite>]`。Server 与客户端都用一个 Condr 自己拼的 preset：`RelayMode::Custom(RelayMap { https://relay.condr.dev })` + `DnsAddressLookup { origin: dns.condr.dev, pkarr: https://dns.condr.dev/pkarr }`。Server 定期把 relay URL 与直连地址签名发布到 Condr 的 DNS；客户端只拿 id 解析出 `EndpointAddr`。解析失败时退回编进二进制的默认 relay 试一次 `EndpointAddr::new(id).with_relay_url(default)`，所以默认配置的用户不依赖 DNS 存活。配置只有一个键：`[server.p2p] enabled = true`，与 TCP listener 一样 opt-in，改动后重启 Server 生效（沿用 `listen` 的行为与 Settings 里的重启提示）。relay 与 DNS 地址编进二进制，**现阶段不考虑用户自建**，所以没有 `relay` / `dns` 配置键；将来要支持自建时再决定地址如何携带 relay。

### 认证：用 iroh 自己的，不叠 Noise

通道已由 Server 的 EndpointId 认证并加密，invite secret 可以作为 Hello 之前的第一帧明文发送（等价于粘贴 token）。Server accept 后 `remote_id()` → `is_authorized`；未知则要求 invite，常量时间比对后 `complete_pairing`。与 TCP 不同，任何拿到 Server id 的 peer 都能完成 QUIC/TLS 握手（知道 id 即知道地址），随后因不在授权表且无 invite 被关闭；接受为已知面，不做限流。ed25519 key 是 32 字节，直接进同一个 `authorized-clients`，`condr server clients | revoke` 零改动。

否决 Noise-over-iroh：双重加密、两把 key、每字节两次加解密，换不到任何安全性。

### 传输适配（最大的一块，约 250 行）

新 `crates/condr-server/src/p2p.rs`：

- `P2pStream { send: Arc<Mutex<SendStream>>, recv: Arc<Mutex<RecvStream>>, conn: Connection, handle: tokio::runtime::Handle }`。`read` / `write` 用 `Handle::block_on`；`set_handshake_timeout` 包一层 `tokio::time::timeout`；`shutdown` = `conn.close()`；`try_clone` 共享两把锁。不用 `tokio_util::io::SyncIoBridge`：它没有 timeout。
- QUIC 自带 keep-alive（iroh 默认 5s）与 idle timeout（30s），与现在 TCP 的 ~30s 探活等价，替代 `enable_keepalive`，不另配置。
- Server：在 `Server::run` 的 runtime 里 bind Endpoint，spawn accept task 往 `mpsc::SyncSender<EndpointStream>` 投递；`EndpointListener::P2p { rx }` 的 `accept()` 就是 `try_recv`，现有轮询循环不动。
- Client：不绑 Endpoint。GUI/CLI 连本地 socket 发 `Tunnel` 帧，由本机 Server 代拨（见「一把 key」一节）。

### 边界不变

`may_administer` 对 P2p 为 false（同 TCP）；`tcp_peers` 改名为 remote peers 后复用；`RevokeDevice` 关闭连接；`condr --device` 因走 `Endpoint::parse` 自动支持；GUI Connect Remote Device 已按 scheme 分发。

### 改动清单

- 新 `condr-server/src/p2p.rs`：`device-key` 文件（复用 `read_secret_file` / `write_secret_file`）、`P2pStream`、`Tunnel` 帧的代拨与 splice、server bind + accept task、pairing 检查。
- `endpoint.rs` / `endpoint/stream.rs` / `endpoint/local.rs` 各加一臂；`PublicKey` 需要 `from_bytes`。
- `server/config.rs` 的 `[server.p2p]` 表；`main.rs` 的 start / status / invite 输出同时打印 `tcp://` 与 `p2p://`。
- GUI `settings/server.rs` 一个开关与文案；`server_management/dialogs.rs` 的 label 一臂：p2p Device 显示 id 前 8 位，连上后换成 Welcome 里的机器名，用户可用现有 rename 改名。
- ADR + CONTEXT.md 的 Device / Invite 段 + roadmap 现状表。
- 测试：进程内两个 Endpoint（`RelayMode::Disabled`，localhost 直连）跑 Hello/Welcome、invite 配对、撤销；headless 可跑。

## 架构决定（2026-09-21 讨论）

### iroh 与 TCP、SSH 并列，不替代

对有稳定可达地址的 Server（VPS、公网 IP、同一局域网、Tailscale 内），TCP + Noise 是最简单、最可预测的路：一个 TCP 端口、没有第三方、没有打洞的不确定性、代码已完成并测过。iroh 能不经 relay 直连公网地址，但那是拿 UDP/QUIC 换 TCP，云安全组与公司网络对 UDP 的放行远不如 TCP；有稳定 IP 时 iroh 的发现、打洞、relay 回退、常驻 relay 连接全是负担。砍掉约千行的 Noise 换 157 个 crate 不是简化。

| 方式 | 什么时候用 | Server 侧 |
| --- | --- | --- |
| `ssh://` | 已有 SSH 访问，不想再配任何东西 | 什么都不用开 |
| `tcp://` | Server 有稳定可达地址 | 开一个 TCP 端口 |
| `p2p://` | 两台机器都在 NAT 后，不想装 Tailscale、不想开端口 | 只出站，连 relay |

并列的代价是两条加密栈、两种 key；后者由下面的「一把 key」消掉。

### Condr 自建 relay 与 DNS，不接 n0 基础设施

- **relay**：`relay.condr.dev`，上游 `iroh-relay` 二进制原样部署在一台 VM 上（长连接 WebSocket + UDP 的 QAD，Cloudflare Pages/Workers 接不了），TLS 用其自带 ACME 或前置 Caddy。**全开**，`client_rx_bytes_per_second` 限速，先看流量；被其他 iroh 应用白蹭再把 access token 编进 Condr。relay 也是打洞的 rendezvous，它宕机则 p2p Device 之间连不上，TCP/SSH 不受影响；起步一个区域，选址要实测国内 UDP 与 WebSocket 出境的稳定性。relay 协议 0.91 后冻结，1.x 内兼容，Condr 同时发客户端与 relay，可控。
- **DNS/pkarr**：`dns.condr.dev`，同一台 VM 跑 `iroh-dns-server`，做该 zone 的权威服务器（Cloudflare 加 NS 委派，开 53/UDP+TCP）并提供 pkarr 的 HTTPS publish。pkarr（Public Key Addressable Resource Records）是节点用私钥签的小 DNS 报文，「域名」就是公钥的 z-base-32，任何人拿 id 可查且可验签。有了它地址缩成 `p2p://<id>`，relay 不再是地址的一部分，换域名、加区域都不影响已保存的 Device；Device 的身份与地址合一。`iroh-dns` 已在依赖树里，客户端不多 crate。
- **不做**：用户自建 relay/DNS（现阶段不考虑，没有配置键）；mDNS 局域网发现（独立功能，等需求）；账号、白名单、多租户计费；把 relay/DNS 嵌进 `condr server`；不接 n0 的任何服务，连兜底也不接。
- **对项目的意义**：Condr 从「没有任何在线服务」变成「有一个可选的在线服务」。要写进 SECURITY.md 与网站：relay 看不到终端内容，但看得到谁在什么时候从哪个 IP 连了谁；iroh 默认只向 DNS 发布 relay URL，不发布任何 IP，拿到 id 的人只能查到它挂在哪个 relay；不落盘、不记账；宕机时 p2p 连接不可用。

### 一台 Device 一把 key；本机 Server 是这台机器唯一的 p2p Endpoint

**事实**（iroh 1.2 源码，`iroh-relay/src/server/clients.rs` 的 `register`）：同一个 EndpointId 被两个 Endpoint 同时绑定时，relay 让新连接接管路由，旧连接的 socket 保持打开但再收不到任何包，只在旧端打一条 WARN，不拒绝、不断开。而 iroh 里发起连接的一方也必须是带 key 的 Endpoint（TLS 双向认证）。于是一台机器上 Server（被连）、GUI（拨号）、每次 `condr --device`（拨号）三类进程若各绑一个 Endpoint：共用一把 key 会互相顶掉，各发一把 key 则一台机器在对端授权表里出现多条。TCP/Noise 没有这个问题：每条连接独立握手，一把 key 多少条连接都行。

**决定**：

- 每台机器只有一把 `device-key`（ed25519 seed；`server-key` / `client-key` 合并，pre-release 直接 break，不迁移）。它的公钥是 iroh 的 EndpointId，也是唯一的 fingerprint：`condr server clients`、Settings 的已配对列表、`RevokeDevice`、roadmap 方向 4 的首连指纹确认都只认这一个值；`revoke` 一次撤掉整台机器。
- **本机 Server 持有这台机器唯一的 iroh Endpoint。** GUI 与 CLI 要连远端 p2p Device 时，连本地 socket，第一帧发 `Tunnel { id, invite }`，之后这条本地连接就是到远端的原样字节管道，Server 两头 splice。客户端侧的 `EndpointStream::P2p` 因此就是一条本地 socket，reader/writer 线程模型不变；`condr --device` 对 p2p Device 用与 GUI 相同的 `ensure_local_server` 保证本机 Server 在跑。这是 Tailscale 的 daemon 模型。它顺带消掉了待决第 5 条：Endpoint 与 relay 连接常驻在 Server，CLI 每次调用不再重新 bind 与连 relay。
- 出站 Endpoint 的生命周期：`[server.p2p] enabled = true` 或有 `Tunnel` 请求时绑定，最后一条 p2p 连接关闭后保留 60 秒再释放。一台从未用过 p2p 的机器不常驻到 relay 的连接；「能连别人」不要求「让别人能连我」，出站不绑在 `enabled` 上。
- TCP 与 SSH 保持 GUI/CLI 直连，不经本机 Server：绕道只增加「本机 Server 必须在跑」的依赖，换不到任何东西。TCP 跟着变的只有 key：GUI/CLI 拨 TCP 用 `device-key` 派生的 X25519 做 Noise initiator，本机 Server 用同一把做 responder（IK 模式一把 key 双向是标准用法，WireGuard 亦然）。派生方式：私钥按 libsodium `crypto_sign_ed25519_sk_to_curve25519`（SHA-512 前 32 字节 clamp，即 ed25519 自己的标量），公钥 `EdwardsPoint::to_montgomery`（`curve25519-dalek` 已在依赖里）。`authorized-clients` 存 ed25519 key，Noise 握手时 Server 把每条授权 key `to_montgomery` 后比对，反方向从不需要。`StaticKey` 变成从 seed 派生的视图。
- 于是**配对一次，两种传输都认**：`tcp://<id>.<invite>@host:port` 与 `p2p://<id>.<invite>` 里的 `<id>` 是同一个字符串，invite 不分 scheme。

**否决**：每机两把 key（GUI 与 CLI 共用 client key，CLI 一跑就打断 GUI 的 relay 路径，已证实）；每进程一把 key 加机器 key 签证书（为避开「经本机 Server」而发明的证书链）。

**ADR 0022 的修订**：它写「no Server proxies another」并以「远端凭据进入长期进程」为由否决联邦。本决定让本机 Server 为 GUI/CLI 转发 p2p 字节，边界收窄为：Server 不持有远端凭据、不拥有远端 Session；作为本机的 p2p Endpoint 转发字节不算 proxy，授权语义与今天 `client-key` 可被本地用户读取完全等价。在新 ADR 里写这一段，0022 正文只加一行指针，不改历史。

UX 后果：id 既是身份也是 p2p 地址，Connect Remote Device 对 p2p 只要一个 `p2p://<id>.<invite>`；TCP 仍要 host:port。

### 对外表述

- **名字**：scheme `p2p://`（暂定，改名成本只是一个字符串常量与文档），UI 与文档统一叫「Peer-to-peer」；不叫 iroh（用户不必先知道它是什么），不叫 Relay（多数时候不走 relay，且暗示内容经过我们），不起品牌名（`condr://` 留给将来的深链接，且品牌名会让它显得高于并列的 SSH/TCP）。托管的基础设施朴素地叫「Condr relay」，只出现在文档与 SECURITY.md。网站 How it works 里一句「built on iroh」并链接，技术用户会去查，明写比被发现更可信。**除这一处署名外，用户接触到的一切都只说 p2p / Peer-to-peer**：配置键（`[server.p2p]`）、CLI 子命令与 flag、`server status --json` 字段、日志的 transport 名、错误文案、Settings、文档。代码里模块与类型也叫 `p2p`（`p2p.rs`、`P2pEndpoint`、`EndpointStream::P2p`），`iroh` 只作为 crate 名出现在 Cargo.toml 与该模块的 `use`，避免代码库里也并存两套词。
- **一句话**：三种方式并列呈现：SSH（你已有 SSH）、TCP（机器有固定地址）、Peer-to-peer（两台都在 NAT 后）。Peer-to-peer 的承诺只有两句：「No port to open, no VPN to install. Paste one link.」与「End-to-end encrypted; Condr's relay never sees your terminals.」其余都是解释。
- **与谁比**：对 Herdr 的差异点从「Server 自带加密配对」升级为「自带穿透与中继」。不主动与 Tailscale、厂商 Remote Control 比：只说「不需要 VPN」，不说「替代 VPN」。
- **诚实项**：Condr 的第一个在线服务，必须主动写出：可选、只在开启 Peer-to-peer 时使用；relay 看不到内容但看得到连接元数据；DNS 上能查到开启了它的 Server 挂在哪个 relay（不含 IP）；不落盘、无账号；宕机只影响 Peer-to-peer。放 SECURITY.md 的 scope 与网站「How peer-to-peer works」小节，README 一行链接。
- **roadmap 改两处**：「环境判断」中「SSH/TCP 够用，Relay 放远期」一句；Relay 从远期表提到近期方向，写明为什么现在做：NAT 后的两台机器今天没有答案；「capability 前置」在 iroh 下不成立，relay 不参与授权，授权与 TCP 完全相同。

### ADR 与术语

两篇 ADR：0025「一台 Device 一把 key」（合并 key 文件、ed25519 派生 X25519、Server 是机器的 p2p Endpoint、0022 的修订），0026「Peer-to-peer 基于 iroh」（scheme、Condr relay/DNS、Credential 帧、与 TCP/SSH 并列、诚实项）。身份决定没有 p2p 也成立且先落地。

CONTEXT.md 在落地时改四个词条（按项目惯例随实现一起改，不提前）：

- **Device**：一台运行 Condr 的机器，由它唯一一把持久 key 标识；这把 key 既是它被连接时的身份，也是它连接别人时的身份，一台机器在任何授权列表里只出现一次。它的 Server 是这台机器的 Peer-to-peer 端点。
- **Invite**：一次性、十分钟有效的秘密，与连接方式无关；随 `tcp://` 或 `p2p://` 链接粘贴，让一台未知 Device 完成一次配对后两种方式都被认可。
- **Peer-to-peer**（新）：一种连接方式。两台都在 NAT 后的 Device 经 Condr relay 协助打洞后直连，打不通时由 relay 转发密文。只需对方的 id，不需开端口。
- **Relay**（新）：Condr 托管的服务，协助 Peer-to-peer 打洞并在直连失败时转发端到端加密的字节；看不到内容，不落盘，宕机只影响 Peer-to-peer。

### Review 后补充的决定（2026-09-21）

- **TCP 配对要拿到 Ed25519 key**：Noise IK 应答方只学到对端 X25519，Montgomery 形式丢了符号位，反推不出 Ed25519。所以 Client 在 `Hello` 里带自己的 Ed25519 公钥，Server 仅当其 `to_montgomery` 等于握手认证的静态 key 时接受并写入 `authorized-clients`。匹配或验证之后，连接的远端身份就是 Ed25519 key，`last-seen`、活跃 peer 表、`RevokeDevice` 全在一个 key 空间比较。无法解压成合法点的授权行视为不匹配，不当错误。
- **旧存储**：Server 首次创建 `device-key` 时丢弃 `authorized-clients`（旧行永不可能匹配）；`[[client.servers]]` 里的 `tcp://<旧 key>@…` 需要重新添加。文件名与行格式不变。
- **`Tunnel` 细节**：只接受来自本地连接的 `Tunnel`，其他 peer 发送得到错误；每个 `Tunnel` 一条 QUIC 连接；本机 Server 负责向远端发 `Credential` 帧；拨号失败回一帧 `Error` 后关闭，成功则 Client 读到的下一帧是远端 `Welcome`；Client 等 `Welcome` 用 SSH 那档不活动超时；CLI 在 Pane 内时把 `Tunnel` 发给自己 Pane 的 Server，否则 `ensure_local_server`。
- **生命周期修正**：`enabled = true` 时 iroh endpoint 常驻并发布 DNS；`enabled = false` 时仅按 `Tunnel` 需要绑定、最后一条连接关闭 60 秒后释放、**不发布任何 DNS 记录**。
- **`invite` / `status`**：`condr server invite` 每个已启用的传输打印一条链接，都未启用则提示；`status --json` 与 Settings 报 `p2p`。
- **密钥复用**：一把 key 同时用于 Ed25519 签名与 X25519 DH 是有意的跨协议复用（Thormarker 2021 分析安全，libsodium 为此暴露转换）；`sha2` 已在依赖树。
- 措辞：ADR 里用「iroh endpoint」区分代码里的 `Endpoint` 枚举；id 编码沿用 hex。
- ADR 0011、0013、0022 各加一行指针指向新 ADR。

## 待决问题

1. **依赖体量**：已接受（2026-09-21）：+157 crate、+3 分钟编译，GUI 与 CLI 都要链，不用 feature 门控拆分；在 ADR 里写成已付的代价。
2. **relay 与 DNS 的选址与运维**：已定自建（见上）；机器区域、ACME、NS 委派、监控待落地。
3. **身份**：已定一把 `device-key`，Server 为机器的 p2p Endpoint（见上）。
4. **iroh 与 TCP 的关系**：已定并列（见上）。
5. **CLI 每次建连成本**：已由「本机 Server 持有 Endpoint」消掉。
6. **Windows** 未实测（iroh 支持 Windows；关掉 `portmapper` 后的打洞表现要看）。
7. **生态绑定**：`noq` 是 n0 的 QUIC fork，`ed25519-dalek` 预发布，n0 是单一上游。

## 来源

- [iroh CHANGELOG](https://github.com/n0-computer/iroh/blob/main/CHANGELOG.md)、[Releases](https://github.com/n0-computer/iroh/releases)、[docs.rs iroh 1.2.0](https://docs.rs/crate/iroh/latest)
- [iroh 1.0.0-rc.0](https://www.iroh.computer/blog/iroh-1-0-0-rc-0)、[iroh 1.0 roadmap](https://www.iroh.computer/roadmap)
- [Relays 概念文档](https://docs.iroh.computer/concepts/relays)、[Shared relays](https://www.iroh.computer/blog/shared-relays)、[iroh 0.91 relay 标准化](https://www.iroh.computer/blog/iroh-0-91-0-the-last-relay-break)、[iroh-relay crate](https://crates.io/crates/iroh-relay)
- 本机探针：`/tmp/iroh-probe`（编译时长与依赖计数）
