---
title: 安全模型
description: 查看 Condr 默认暴露什么、每种连接信任谁，以及中继能看到什么。
---

用这一页判断一台机器是否适合安装 Condr，并了解每种连接方式允许什么。

## 保持默认关闭

新安装的 Server 只监听本机的私有 socket。macOS 和 Linux 上 socket 的权限是 0600，只有你的用户能连接。Windows 上的命名管道只允许你和 SYSTEM。

Server 不会上公网，不打开 TCP 端口，也不联系 Condr 服务。在运行 `--listen` 或 `--p2p` 前，它对网络不可见。

Condr 没有账号和遥测。它唯一的托管服务是 Peer-to-peer 中继，只有开启 Peer-to-peer 的 Device 会联系它。

## 每台 Device 使用一把密钥

每台运行 Condr 的机器有一把 Ed25519 密钥，保存在配置目录的 `device-key` 文件中，权限为 0600。其他用户能读取时，Condr 会拒绝启动并提示你修复权限。

同一把密钥用于这台机器的 Server 接受连接，也用于窗口和命令行向外连接。因此一台 Device 只有一个指纹，在授权列表中只出现一次。指纹是 43 个字符的 base64url，也是链接中的 Device key。

删除 `device-key` 会生成新密钥，并清空这台机器的已授权 Device 列表。所有配对都必须重做。

## SSH 连接信任谁

Condr 不负责 SSH 连接的认证和加密，这两项都交给本机 OpenSSH。Condr 在远端运行 `condr server bridge`，把远端 Server 的私有 socket 转发到本机。

远端 Server 把这条连接当作本机连接。因此，通过 SSH 连接的窗口可以执行本机窗口能做的一切，包括管理 Server。能 SSH 登录一台机器本来就能在那里做任何事，Condr 没有新增权限。

## TCP 连接信任谁

TCP 连接使用 `Noise_IKpsk2` 握手，属于和 WireGuard 相同的一类。双方用各自的静态密钥认证，连接全程加密。

信任从 invite 开始。链接中带有 Server 的 Device key，本机只向这把密钥加密第一条消息，所以不需要「首次连接确认指纹」步骤。已配对 Device 直接进入握手，未配对 Device 还必须出示有效 invite。

invite 包含 32 个随机字节，10 分钟后过期，只能使用一次。Server 把它保存在配置目录的 `pending-invite` 文件中，权限为 0600。第一台完成配对的 Device 会用掉它。持有 invite 的任何人都能配对，请只通过信任的渠道发送。

配对后：

- Server 在 `authorized-clients` 中添加一行，记录该 Device 的密钥、配对时间、最近连接时间和名称。
- 本机在配置文件中记录 Server 的 Device key 和地址，不保存 invite。

撤销 Device 会从 `authorized-clients` 删除对应行。现有连接立即关闭，下次握手失败。

## Peer-to-peer 连接信任谁

Peer-to-peer 使用和 TCP 相同的密钥、invite 和授权列表。撤销会同时作用于两种连接。

两台 Device 用自己的密钥端到端加密数据。两个 Condr 服务参与建立连接，但看不到内容：

- **中继（`relay.condr.dev`）** 帮 Device 打洞。直连失败时转发加密字节，直连成功后离开数据路径。它能看到哪台 Device 在什么时候从哪个 IP 连接哪台 Device，不在磁盘保存数据，也没有账号。
- **DNS（`dns.condr.dev`）** 让其他 Device 找到 Server 使用的中继。开启 Peer-to-peer 的 Server 会以自己的 Device key 发布签名记录。记录只有中继地址，不含 IP。持有 Device key 的人能查到它所在的中继。

Server 只在配置开启 Peer-to-peer，或本机正在拨出 `p2p://` 连接时联系这两个服务。最后一条拨出连接关闭一分钟后，它会断开。只拨出的 Device 不发布记录。

## 了解配对 Device 能做什么

目前配对会授予整个 Session。通过 TCP 或 Peer-to-peer 连接的 Device 可以：

- 创建、修改和关闭 Workspace、Tab、Pane。
- 在终端中输入、粘贴和复制。
- 读取 Pane 内容和 Git 改动，浏览 Server 机器上的任何目录或文件。
- 启动 Agent 并安装 Agent hook。
- 停止 Server。

它不能管理 Server，包括开关监听和 Peer-to-peer、生成 invite、列出和撤销 Device、重启 Server。这些操作只接受本机和 SSH 连接。

所以只把自己的 Device 配对到 Server。只读和控制权限还没有分开实现，与他人共用一台 Server 要等该功能。

## 报告安全问题

不要公开提 Issue。请在 GitHub 中通过 **Security › Report a vulnerability** 私下报告，并附上 Condr 版本、操作系统和复现步骤。你会在 7 天内收到回复。详情见 [SECURITY.md](https://github.com/condrdev/condr/blob/main/SECURITY.md)。
