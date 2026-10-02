---
title: 连接远程 Device
description: 用 SSH、TCP 或 Peer-to-peer 连接另一台机器上的 Server。
---

你电脑上的 Condr 可以连接其他机器上的 Server。连上后，那台机器会作为一个 Device 出现在侧栏，Workspace 和 Agent 的用法和本机一样。

Condr 提供三种连接方式：

| 你的情况 | 连接方式 |
| --- | --- |
| 已经能用 SSH 登录那台机器 | [SSH](#用-ssh-连接) |
| 那台机器有你能访问的 IP，例如局域网或 VPN 中的服务器 | [TCP](#用-tcp-连接) |
| 两台机器都在路由器后，没有公网地址 | [Peer-to-peer](#用-peer-to-peer-连接) |

开始之前，先在那台机器上安装 Condr。服务器和开发机装 Headless Server，你的另一台电脑装桌面应用。见[安装](/zh-cn/docs/install/)。

## 用 SSH 连接

Condr 用你电脑上的 OpenSSH 连接那台机器，并自动启动那台机器上的 Server。

连接之前：

1. 在那台机器上安装 Headless Server。
2. 在终端运行 `ssh user@host`，确认不需要输入密码。Condr 使用非交互 SSH，不会提示密码。请先配置好密钥或 ssh-agent，并信任主机密钥。

连接：

1. 点侧栏底部的 **Connect Remote Device → SSH**。
2. 输入你平时 SSH 登录用的地址，例如 `user@build-box`。要指定端口，写 `user@build-box:2222`。

Condr 把地址原样交给 OpenSSH，所以 `~/.ssh/config` 中的别名、端口、密钥和跳板都会生效。

macOS 和 Linux 上，Condr 会复用 SSH 连接，之后不用重新认证。如果你的 SSH 配置已经设置了 `ControlPath`，Condr 改用你的设置。需要二次验证的主机，可以在 `~/.ssh/config` 中为它设置 `ControlMaster auto` 和 `ControlPath`，先在终端登录一次并保持会话，Condr 就会复用这条连接。Windows 上不复用，每次都新建 SSH 连接。

暂不支持用 SSH 连接 Windows 机器，请改用 TCP 或 Peer-to-peer。

## 用 TCP 连接

Server 默认只接受本机连接。要用 TCP 连接，先让那台机器上的 Server 监听一个网络地址，再用一次性 invite 配对。

在那台机器上：

1. 用监听地址启动 Server。地址必须是 IP 加端口，不能是主机名。Condr 会把地址写入配置，以后启动时自动使用。Server 已经在运行时，把 `start` 换成 `restart`：

   ```sh
   condr server start --listen 0.0.0.0:2637
   ```

2. 创建 invite：

   ```sh
   condr server invite
   ```

   命令会打印一个 TCP 链接：

   ```text
   tcp://<device key>.<invite>@<host>:2637
   ```

   把其中的 `<host>` 换成这台机器的 IP。

在你的电脑上：

1. 点侧栏底部的 **Connect Remote Device** → **TCP**。
2. 在 10 分钟内粘贴链接。过期后，回到那台机器重新运行 `condr server invite`。

那台机器装的是桌面应用时，也可以在它的 Condr 里操作：**Settings › Device › Remote access** → 打开 **TCP listener** 并填写 **Listen address** → **General** → **Restart to apply** → **Paired devices** → **Generate invite**。Server 重启前无法生成 invite。

## 用 Peer-to-peer 连接

适合两台都在路由器后的机器，例如家里的台式机和工作用的笔记本。不需要固定 IP、端口转发或 VPN。

在那台机器上：

1. 开启 Peer-to-peer 并启动 Server。Condr 会把这个设置写入配置。Server 已经在运行时，把 `start` 换成 `restart`：

   ```sh
   condr server start --p2p
   ```

2. 运行 `condr server invite`。它会打印一个可以直接使用的链接：

   ```text
   p2p://<device key>.<invite>
   ```

在你的电脑上：

1. 点侧栏底部的 **Connect Remote Device** → **Peer-to-peer**。
2. 在 10 分钟内粘贴链接。

那台机器装的是桌面应用时，也可以在它的 Condr 里操作：**Settings › Device › Remote access** → 打开 **Peer-to-peer** → **General** → **Restart to apply** → **Paired devices** → **Generate invite**。

两台机器能直连时直接连接，否则通过 Condr 中继。流量用两台 Device 的密钥端到端加密，中继无法读取。中继或 DNS 服务不可用时，只有 Peer-to-peer 连接会失败，SSH 和 TCP 不受影响。中继能看到什么，见[安全模型](/zh-cn/docs/security/)。

Peer-to-peer Device 在侧栏的默认名称是 `p2p` 加 Device key 的前 8 个字符。要重命名，右键 Device → **Edit Remote Device**。

## 了解 invite

- **Device key** 标识一台 Device，共 43 个字符。
- **invite** 是一次性的配对凭据，10 分钟后过期。请像密码一样保密。
- 第一台用它完成配对的 Device 会用掉它。重新生成会替换旧的 invite。
- 配对后，Condr 只保存 Device key 和地址，不保存 invite。

## 在 Device 之间切换

侧栏按 Device 分组显示 Workspace。点 Workspace 会切换到它，同时展开或收起它下面的 Agent 列表。

Device 标题上的标记表示连接状态：

- **转圈**：正在连接。
- **红色警告**：无法连接。悬停查看原因，点击重新连接。
- **黄色三角**：已连接，但两边的 Condr 构建不同。悬停查看该更新哪一边，点击关闭提示。

连接断开 2 秒后，Workspace 上方会显示「Reconnecting to …」。Condr 每半秒重试一次，持续 45 秒，之后停止重试并显示 **Disconnected**。点 **Connect** 可以手动重连。电脑从睡眠中唤醒时，Condr 会探测每个 Device，10 秒没有回应就开始重连。

Condr 启动时第一次就连不上的 Device 不会自动重试。请在那个 Device 的页面点 **Connect**。

## 管理已配对的 Device

Server 会记录通过 TCP 和 Peer-to-peer 配对的 Device。在 Server 所在的机器上查看或撤销：

```sh
condr server clients
condr server revoke <fingerprint 或它的前缀>
```

`clients` 列出每个 Device 的名称、最近连接时间和 Device key。`revoke` 会立即断开那个 Device，它要用新的 invite 才能再次配对。前缀区分大小写，而且只能匹配一个 Device。

在 Condr 中，**Settings › Device › Paired devices** 可以做同样的事：**Generate invite** 生成一个 invite 并复制到剪贴板，页面按 Peer-to-peer 和 TCP 各列出一条链接，每条都有 **Copy**。已配对的 Device 每行显示名称和指纹，已连接的排在前面并带绿点，行尾有 **Revoke**。

只有本机和 SSH 连接可以管理 Server，包括生成 invite、开关监听、开关 Peer-to-peer、撤销 Device 和重启 Server。通过 TCP 或 Peer-to-peer 连接时，你可以使用 Workspace 和 Agent，但 Device 标签顶部会显示「Viewing only」，这些设置不能修改。

## 处理版本不一致

连接时，Condr 会交换两边的协议版本和构建号：

- **协议兼容，构建不同**：功能照常，侧栏显示黄色三角。把两边升级到同一版本并重启 Server 后，标记会消失。
- **协议不兼容**：连接被拒绝，窗口提示「speak different protocol versions」。把两边升级到同一版本后重新连接。

升级远程 Device 后，要重启它的 Server。运行 `condr server restart`，或 **Settings › Device › General** → **Restart Condr**。

## 在命令行访问远程 Device

Condr 保存的 Device 也能在命令行中使用：

```sh
condr device list
condr --device build-box workspace list
condr workspace list --all-devices
```

`--device` 后面写侧栏中的 Device 名称。用 `CONDR_DEVICE` 设置默认值。`server` 和 `agent hooks` 只作用于本机，不接受 `--device`。详见 [Agent 驱动 Condr](/zh-cn/docs/automation/)。

## 连不上时

- **SSH could not reach the device**：在终端运行 `ssh user@host`，在那里解决密钥、ssh-agent、主机密钥或 `~/.ssh/config` 的问题。Condr 不会提示输入密码。
- **报 `condr: not found`**：SSH 的非交互 shell 通常不读 `~/.profile` 或 `~/.zprofile`，而安装脚本正是在这些文件里加入 PATH。在地址里写出 `condr` 的完整路径：

  ```text
  user@build-box?bin=/home/user/.local/bin/condr
  ```

- **SSH reached the device, but Condr is not running there**：那台机器上的 Server 没有运行，也无法启动。在那台机器上运行 `condr server start`，看它报什么错。
- **this device is not authorized**：invite 已过期、已被使用，或者这个 Device 已被撤销。在那台机器上重新运行 `condr server invite`。
- **speak different protocol versions**：两边版本差距过大。见[处理版本不一致](#处理版本不一致)。

更多检查方法见[故障排查](/zh-cn/docs/troubleshooting/)。
