---
title: 安全模型
description: Condr 默认对外开放什么、每种连接方式信任谁、配对设备能做什么，以及 Condr 会访问哪些网络服务。
---

本文档用于评估特定设备是否适合部署 Condr，并说明不同连接方式所授予的具体权限范围。

---

## 默认只对本机开放

* **本机 socket**：新安装的 Server 仅监听本机的私有 socket。在 macOS 和 Linux 上，socket 权限设为 `0600`，仅允许当前用户连接；在 Windows 上，命名管道仅允许当前用户和 SYSTEM 访问。
* **TCP 端口**：在显式配置 `[server] listen` 之前，Server 不打开任何 TCP 端口。该配置可通过执行 `condr server start --listen` 或在 **Settings › Device › Remote access** 中写入。
* **Peer-to-peer 访问**：在手动开启 Peer-to-peer 之前，其他设备无法通过 P2P 方式连接当前 Server。
* **本机连接权限边界**：以当前用户身份运行的任意程序均可连接本机 socket 并执行全部操作（包括管理 Server）。Pane 内运行的程序和 Agent 同样具备该权限，且能够使用当前设备的设备密钥连接已配对的远程设备。

---

## Condr 访问的网络服务

Condr 不设用户账号体系，亦不收集遥测数据。运行期间仅访问以下三个外部服务：

| 服务地址 | 触发条件 | 用途说明 |
| :--- | :--- | :--- |
| `api.github.com` | 窗口启动 5 秒后首次请求，其后每 5 小时轮询一次 | 检查版本更新。可在 **Settings › Application › About** 中关闭自动检查，或设置 `[client.updates] auto_check = false` |
| `relay.condr.dev` | Server 启用了 Peer-to-peer，或本机主动发起 `p2p://` 连接 | 辅助 NAT 打洞；在直连建立失败时中继加密流量 |
| `dns.condr.dev` | Server 启用了 Peer-to-peer，或本机主动发起 `p2p://` 连接 | 发布并查询各设备当前使用的中继地址 |

---

## 设备密钥

* **单设备唯一密钥**：每台运行 Condr 的设备持有一对唯一的 Ed25519 密钥，存储于数据目录下的 `device-key` 文件中。
* **入站与出站共用**：Server 接收外部连接，以及本地窗口和命令行向外发起连接，均共用此单一密钥。因此每台设备仅具备一个指纹标识，在对端设备的授权列表中只对应单行记录。
* **指纹格式**：指纹为 43 个字符的 base64url 格式字符串。在连接链接中显示为 Device key，在 `condr server status` 命令输出中显示为 Fingerprint。
* **Linux 与 macOS 上的权限要求**：文件权限必须为 `0600`。若其他用户具备访问权限，Condr 将拒绝加载该密钥并报错 `… is readable by other users (mode …); make it 0600 or delete it`，此时 TCP 与 Peer-to-peer 连接均无法启动。
* **Windows 上的权限要求**：文件权限仅限文件所有者和 SYSTEM 访问，且不继承父目录权限。Condr 每次执行读取时均会重新强制校验并设置该 ACL。
* **本地同用户隔离限制**：密钥以纯文本文件存储，未集成进操作系统钥匙串。以当前用户身份执行的本地进程均具备读取该文件的物理权限。

### 更换密钥

停止 Server 进程，同时删除数据目录中的 `device-key` 和 `authorized-clients` 文件，随后重启 Server。Condr 将重新生成全新密钥，此前建立的所有配对关系均需重新配置。

若仅单独删除 `device-key`，窗口可能先于 Server 生成新密钥，导致旧的授权列表残留。

---

## SSH 连接

* **认证与加密实现**：认证与传输加密完全由本地 OpenSSH 实现，Condr 本身不参与底层安全逻辑。
* **非交互运行模式**：Condr 显式指定 `BatchMode=yes` 参数调用系统 `ssh` 命令，执行期间不会提示输入交互密码，亦不会要求手动确认未知的主机公钥指纹。主机密钥验证严格遵循本地既有的 OpenSSH 配置。
* **隧道转发机制**：Condr 在远端设备执行 `condr server bridge` 命令，将远端 Server 的私有 socket 隧道转发至本机。若远端 Server 尚未启动，`bridge` 命令会自动拉起远端实例。
* **权限等价性**：远端 Server 将通过 SSH 接入的连接等同视为主机本地连接，通过 SSH 连接的窗口可调用本地窗口拥有的全部权限（包括管理 Server 进程）。拥有 SSH 登录权限即意味着拥有目标设备的完整命令执行能力，Condr 未引入额外的越权风险。
* **连接复用机制**：在 macOS 和 Linux 上，Condr 默认启用 OpenSSH 连接复用特性，将生成的控制 socket 存放在运行时目录中，并在连接闲置 60 秒后自动关闭。若系统 SSH 配置已显式声明 `ControlPath`，Condr 保持既有配置不变。

---

## TCP 连接

* **握手安全协议**：TCP 传输基于 `Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s` 协议完成握手（与 WireGuard 属于同一类握手模式）。通信两端使用静态密钥进行双向身份认证，连接全程强制加密。
* **信任链建立方式**：连接字符串内包含 Server 端的设备公钥，客户端仅向该公钥发起握手，因此无需在首次连接时人工确认指纹。
* **已配对设备行为**：直接完成握手并建立加密信道。
* **未配对设备行为**：必须提供有效的 invite 凭证。invite 充当握手过程中的预共享密钥（PSK）；若缺失 invite 则无法解密 Server 返回的握手包，Server 将直接终止该连接。

---

## Invite 与配对

* **Invite 规范**：由 32 字节随机数构成，有效期为 10 分钟，且仅限单次消费。系统在同一时刻仅允许存在一个有效 invite，生成新 invite 将立即使历史凭证失效。该凭证保存在数据目录下的 `pending-invite` 文件中，访问权限配置与设备密钥一致。
* **使用限制**：首台成功完成握手接入的设备将直接消耗该 invite。任何持有该 invite 的主体均可完成配对，必须仅通过受信渠道分发。
* **配对记录格式**：配对成功后，Server 在 `authorized-clients` 文件中追加一行，记录对端的设备公钥、配对时间戳、最后活跃时间以及对端主机名。
* **客户端本地持久化**：客户端仅在 `config.toml` 中存储已连接 Server 的设备密钥与网络地址，不留存 invite 凭据。

---

## Peer-to-peer 连接

* **共享凭据与鉴权体系**：Peer-to-peer 与 TCP 共享同一套设备密钥、invite 机制及已授权客户端列表。撤销某台设备的授权将同步作用于上述两种连接。
* **通信加密**：两端通过 `iroh` 建立 QUIC 链路，使用各自的设备密钥完成 TLS 1.3 双向身份认证，数据链路保持端到端加密。invite 凭证仅在底层加密信道就绪后才会传输。
* **握手公开性**：任何获取当前设备公钥的主体均可完成基础 QUIC 握手流程，随后才会因未进入授权名单而被阻断拦截。被拦截方在断开前可读取 Server 的程序构建版本号。目前未设置连接频次限制。
* **中继服务 `relay.condr.dev`**：用于辅助对端建立 NAT 打洞通道；直连失败时中继端到端加密数据，直连打通后退出传输路径。中继服务能获取通信两端的设备公钥标识、连接建立时间以及源 IP 地址，但无法解密传输内容。该中继由 Condr 官方维护，运行未作二次修改的开源 `iroh-relay` 服务，不执行磁盘持久化写入，亦不维护用户账户体系。
* **DNS 服务 `dns.condr.dev`**：启用 Peer-to-peer 的 Server 每隔 5 分钟使用自身设备私钥对当前所选中继地址签名并发布一条解析记录（记录仅包含中继地址信息，不包含设备实际 IP）。持有对应设备密钥的主体均可查询该条解析记录。
* **服务通信触发时机**：启用 Peer-to-peer 的 Server 将与中继服务器维持长连接。仅向外发起 `p2p://` 连接请求的设备不向 DNS 发布解析记录，并在最后一个活跃 P2P 连接断开 60 秒后关闭与中继的连接。

---

## 撤销设备

执行 `condr server revoke <指纹或前缀>`，或在界面打开 **Settings › Device › Paired devices** 点击 **Revoke**。

* **生效时效**：Condr 会立即从 `authorized-clients` 中移除目标行，并同步切断该设备当前建立的所有活跃连接。
* **后续接入表现**：后续 TCP 连接请求将在 Noise 握手阶段阻断；Peer-to-peer 连接将在 QUIC 握手完成后予以拒绝。

---

## 配对设备能做什么

目前配对授权的作用范围覆盖整个 Session。通过 TCP 或 Peer-to-peer 连接的对端设备具备以下权限：

* 创建、修改和关闭 Workspace、Tab、Pane 以及关联的 Worktree。
* 在终端执行输入、粘贴与文本复制操作（即具备以 Server 所在运行用户权限执行任意命令的能力）。
* 读取各 Pane 内容、Git 状态变更及 Workspace 中的文件，并可列出这台设备上任意目录的子目录。
* 启动 Agent 进程，并通过图形配置界面安装 Agent Hook。
* 更改 Server 运行时的默认 Shell。
* 读取 Server 运行状态信息，包括网络监听配置、已连接设备指纹及最近的系统错误记录。
* 终止 Server 进程。虽然窗口界面和命令行未直接暴露远程停止按钮，但底层协议支持此操作。

以下敏感管理操作只接受本机本地连接及 SSH 会话调用：

* 开启或关闭 TCP 监听端口及 Peer-to-peer 开关。
* 签发 invite 凭证，查询及撤销已配对设备。
* 重启 Server 进程。
* 将当前设备用作 Peer-to-peer 连接的中转。

因此，请只将自己持有的设备与 Server 完成配对。当前系统尚未实现只读权限与控制权限的分权解耦，多用户协同共用单 Server 需等待该功能上线。

**Viewing only** 不是权限边界：该机制仅用于在多窗口并发接入场景下，将未取得控制的窗口在前端界面层置为只读视图。

---

## 终端中的程序能做什么

* **剪贴板写入行为**：Pane 内运行的程序可通过 OSC 52 转义序列静默覆写本机剪贴板，远程设备执行的程序同样具备该能力。所有当前接入的窗口客户端均会响应写入，期间不会出现交互确认弹窗。程序本身无法反向读取系统剪贴板内容。
* **超链接处理安全**：按住 Cmd 或 Ctrl 点击终端内链接时，Condr 直接调用操作系统默认程序打开目标 URL，不对协议头作白名单限制。运行程序可通过 OSC 8 转义序列伪造显示文本与实际落地跳转地址不一致的链接，点击前需人工核实目标 URL。
* **Agent 状态可伪造性**：终端 Pane 内部任意进程均可自行构造并伪造 Agent 状态通知事件。Agent 状态仅用于前端 UI 视觉呈现，不可用作鉴权或安全判断凭据。
* **远程粘贴图像文件的存储生命周期**：在远程设备执行图像粘贴时，文件写入远程设备的独立私有目录。在 Linux 和 macOS 上，该存储目录权限为 `0700`、文件权限为 `0600`。文件在窗口连接终止或 Server 进程退出时自动清除，残留历史文件在 24 小时后由清理策略自动删除。在 Windows 上，该临时目录权限直接继承父文件夹配置。

---

## 文件权限

| 文件名 | Linux 与 macOS 权限模型 | Windows 权限模型 |
| :--- | :--- | :--- |
| `device-key`、`pending-invite` | `0600`，权限过宽时拒绝加载运行 | 仅允许所有者及 SYSTEM 访问 |
| `config.toml`、Session 快照、Server 进程的 `.stderr` 文件 | `0600` | 继承父目录 ACL 权限 |
| `authorized-clients`、系统日志文件 | 遵循操作系统默认 umask 创建 | 继承父目录 ACL 权限 |

具体文件存储物理路径参见[配置与设置](/zh-cn/docs/reference/configuration/)。

---

## 报告安全问题

请勿通过公开 Issue 提交安全缺陷。请访问 GitHub 的 [Security › Report a vulnerability](https://github.com/condrdev/condr/security/advisories/new) 渠道进行私密漏洞披露，报告需包含：

* 具体的 Condr 版本标识（发行版 Release tag 或 Git commit hash）。
* 操作系统。
* 明确的漏洞复现步骤。
* 漏洞影响范围评估。

你会在 7 天内收到回复。具体披露指引参见 [SECURITY.md](https://github.com/condrdev/condr/blob/main/SECURITY.md)。
