---
title: 连接远程 Device
description: 选择 SSH、TCP 或 Peer-to-peer，连接并管理另一台机器。
---

把另一台机器加入侧栏，按情况选择连接方式，并处理断开或版本不一致。信任规则见[安全模型](/zh-cn/docs/security/)，连接检查见[故障排查](/zh-cn/docs/troubleshooting/)。

**Device key** 用来标识一台 Device，共 43 个字符。**invite** 是一次性邀请，要保密，10 分钟后过期。

## 选择连接方式

先在远程 Device 上安装 `condr`，见[安装](/zh-cn/docs/install/)。然后按你的情况选择：

| 你的情况 | 选择 | 链接形式 |
| --- | --- | --- |
| 已经能用 SSH 登录这台 Device | SSH | `ssh://user@host` |
| Device 有可访问的 IP，例如网络中的服务器 | TCP | `tcp://<device key>.<invite>@host:port` |
| 两台机器都在路由器后，没有公网地址 | Peer-to-peer | `p2p://<device key>.<invite>` |

三种方式都要在侧栏点 **Connect Remote Device**，再粘贴链接。连接后，远程 Device 和本机一样使用同一个窗口、Workspace 和 Agent 状态。

## 用 SSH 连接

```text
ssh://user@build-box
```

Condr 将地址原样交给本机 OpenSSH。因此 `~/.ssh/config` 中的别名、端口、密钥和跳板都会生效。要指定端口，使用 `ssh://user@host:2222`。

连接时，Condr 在远端运行 `condr server bridge`。远端没有运行中的 Server 时，Condr 会启动它。

先准备好：

- **免密登录。** Condr 用 `BatchMode` 连接，不会提示密码。先在终端配置密钥、ssh-agent 和主机信任。
- **远端 PATH 中有 `condr`。** SSH 非交互 shell 通常不读取 `~/.profile` 或 `~/.zprofile`，而安装脚本正是在这些文件中加入 PATH。连接报告 `condr: not found` 时，在链接中写出路径：

  ```text
  ssh://user@host?bin=/home/user/.local/bin/condr
  ```

- **需要二次验证的主机。** 先在终端手动登录一次并保持会话，Condr 会复用它。

macOS 和 Linux 上，Condr 自动用 `ControlMaster` 复用 SSH 连接，后续调用不用重新认证。如果 SSH 配置已有 `ControlPath`，Condr 会使用你的设置，不再添加。Windows 不支持连接复用，每次调用都会新建 SSH 连接。暂不支持把 Windows 机器作为远程 Device。

## 用 TCP 连接

```text
tcp://<device key>.<invite>@<host>:<port>
```

Server 默认只监听本机。要接受网络连接，在远程 Device 上：

1. 用监听地址启动 Server。地址必须是 IP 和端口，不能是主机名。Condr 会把地址写入配置，之后启动会自动使用。如果 Server 已运行，把 `start` 换成 `restart`：

   ```sh
   condr server start --listen 0.0.0.0:2637
   ```

2. 创建 invite：

   ```sh
   condr server invite
   ```

   命令打印 TCP 链接，其中的 `<host>` 是占位符。把它换成这台 Device 的 IP。

3. 在 10 分钟内打开工作电脑上的 Condr，点 **Connect Remote Device**，粘贴链接。过期后重复第 2 步。

invite 只能使用一次。第一台完成配对的 Device 会用掉它。重新生成会替换旧 invite。配对后，Condr 只保存 Device key 和地址，不保存 invite。

## 用 Peer-to-peer 连接

```text
p2p://<device key>.<invite>
```

适用于两台都在路由器后的机器，例如家里的台式机和工作笔记本。不需要固定 IP 或 VPN。

1. 开启 Peer-to-peer 后启动 Server。Condr 也会把这个设置写入配置。Server 已运行时使用 `restart`：

   ```sh
   condr server start --p2p
   ```

2. 运行 `condr server invite`。它打印没有占位符的 Peer-to-peer 链接。
3. 在 10 分钟内把链接粘贴到 Condr。

两台 Device 能直连时会直连，否则通过 Condr 中继。流量由两台 Device 的密钥端到端加密，中继无法读取。中继或 DNS 服务不可用时，只有 Peer-to-peer 连接失败，SSH 和 TCP 不受影响。中继能看到什么，见[安全模型](/zh-cn/docs/security/)。

Peer-to-peer Device 在侧栏的默认名称是 `p2p` 加 Device key 的前 8 个字符。右键 Device，选 **Edit Remote Device** 重命名。

## 在 Device 之间切换

侧栏按 Device 分组显示 Workspace。点 Workspace 即可切换。

Device 标题上的标记表示连接状态：

- 转圈：正在连接。
- 红色警告：无法连接。悬停查看原因，再点击重连。
- 黄色三角：已连接，但两边 Condr 构建不同。悬停查看应更新哪边，再点击关闭提示。

连接断开后，Condr 等待 2 秒才显示提示。之后在 Workspace 上方显示「Reconnecting to …」，每半秒重试一次，持续 45 秒。随后停止重试并显示 **Disconnected**。点 **Connect** 手动重连。电脑从睡眠唤醒时，Condr 会探测每个 Device，10 秒没有回应就开始重连。

Condr 启动时首次连接失败的 Device 不会自动重试。请在该 Device 页面点 **Connect**。

## 管理已配对的 Device

Server 会记录通过 TCP 和 Peer-to-peer 配对的 Device。在 Server 所在机器上查看或撤销：

```sh
condr server clients
condr server revoke <fingerprint 或它的前缀>
```

`clients` 列出 Device 名称、最近连接时间和 Device key。`revoke` 立即断开该 Device，它需要新的 invite 才能再次配对。前缀区分大小写，且必须只匹配一个 Device。

在 Condr 中，打开 **Settings › Device › Paired devices** 执行同样操作。**Generate invite** 会生成并复制链接，列表每行都有 **Revoke**。

只有本机和 SSH 连接可以管理 Server，包括生成 invite、开关监听、开关 Peer-to-peer、撤销 Device 和重启 Server。通过 TCP 或 Peer-to-peer 连接的窗口可以使用 Workspace 和 Agent，但这些设置只读。

## 处理版本不一致

连接时，Condr 会交换协议版本和构建号：

- **协议兼容，构建不同。** 功能照常，侧栏显示黄色三角。两边更新到同一版本并重启 Server 后，标记消失。
- **协议不兼容。** 连接被拒绝，窗口提示「speak different protocol versions」。两边更新到同一版本后重新连接。

更新远程 Device 后，要重启它的 Server。运行 `condr server restart`，或在 **Settings › Device › Daemon** 中点 **Restart Condr**。

## 连不上

| 提示 | 多半原因 |
| --- | --- |
| SSH could not reach the device | SSH 本身无法连接，或远端没有 `condr` |
| SSH reached the device, but Condr is not running there | 远端 Server 未运行且无法启动 |
| this device is not authorized | invite 过期、已使用或该 Device 被撤销 |
| speak different protocol versions | 两边版本差距过大 |

每条提示的检查方法见[故障排查](/zh-cn/docs/troubleshooting/)。

## 在命令行访问远程 Device

Condr 保存的 Device 也能从命令行使用：

```sh
condr device list
condr --device build-box workspace list
condr workspace list --all-devices
```

`--device` 后写侧栏中的 Device 名称。用 `CONDR_DEVICE` 设置默认值。`server` 和 `agent hooks` 只作用于本机，不接受 `--device`。详情见 [Agent 驱动 Condr](/zh-cn/docs/automation/)。
