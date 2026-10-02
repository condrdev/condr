---
title: 故障排查
description: 找到最先该查的地方，修复常见问题，并准备 Issue 信息。
---

先检查 Server 状态和日志，再按问题类型查看对应步骤。

## 先查状态和日志

**Server 状态。** 在运行 Server 的机器上执行：

```sh
condr server status
```

命令显示 Server 是否运行、版本、监听地址、Workspace 和 Agent 数量、连接客户端数，以及最近 20 条警告和错误。保存的监听地址或 Peer-to-peer 设置还没生效时，会多一行 Pending。加上 `--json` 可得到一个对象，用于 Issue。远程 Device 的状态也在 **Settings › Device › General** 中显示。

**日志。** 日志每天滚动，保留 7 天。位置见[配置与设置](/zh-cn/docs/reference/configuration/)。**Settings › Developer › Locations** → **Open** 会打开日志目录。Server 日志是 `condr-server-<id>.<date>.log`，窗口日志是 `condr-gui.<date>.log`。后台 Server 崩溃时，输出在旁边的 `.stderr` 文件中。

要获取更多细节，用 `CONDR_LOG` 重启 Server：

```sh
CONDR_LOG=debug condr server restart
```

`CONDR_LOG` 使用 `tracing` 过滤语法，例如 `condr_server=trace`。窗口也读取这个变量，请从终端启动窗口。

## 窗口无法连接本机 Server

每次本机连接时，如果没有运行中的 Server，窗口会启动一个并等待 1.5 秒。超时后会提示「condr-server did not become ready」，并显示 `.stderr` 文件的最后几行。

检查这些原因：

- **端口被占用。** 配置中的 `listen` 地址无法绑定时，Server 会整体启动失败。日志会显示「failed to listen on tcp://…」。改端口，或从 `config.toml` 删除 `listen`。
- **找不到 `condr`。** 提示是「condr is not installed beside condr-gui」。桌面应用自带 `condr`，这通常表示自行构建时没有构建 `condr`。
- **版本不兼容。** 窗口和已运行的 Server 使用不同协议，提示「speak different protocol versions」。运行 `condr server restart`，让新版本接管。

窗口打开时停止本机 Server，窗口会在 45 秒内启动新的 Server。

## Agent 状态保持 Unknown

状态只来自 Agent hook。按顺序检查：

1. **安装 hook 了吗？** 用你的 Agent 运行 `condr agent hooks status claude`。`missing` 要安装，`outdated` 要重装。远程 Device 在 **Settings › Device › Agent integrations** 中查看。
2. **Agent 在 Condr Pane 中运行吗？** hook 只在设置了 `CONDR_ENV=1` 的 shell 中生效。Condr 看不到其他终端中的 Agent。
3. **Agent 会报告状态吗？** Codex、Copilot、Cursor 和 Antigravity 在第一次提示前不报告。Kimi 不支持。见 [Agent 集成](/zh-cn/docs/using/agents/) 的支持表。
4. **Codex 信任 hook 了吗？** 安装后在 Codex 中运行 `/hooks`。
5. **hook 写到了其他目录吗？** hook 在安装时根据环境变量寻找 Agent 配置目录，例如 `CLAUDE_CONFIG_DIR`。窗口与终端的环境可能不同。

## Pane 中找不到 Agent

Server 使用自己的 PATH 查找 Agent，这个 PATH 来自启动 Server 的程序。从 Dock 或开始菜单启动的窗口获得的是系统最小 PATH，也不会读取 `.zshrc`。

尝试以下方法：

- 从终端运行 `condr server restart` 启动一次 Server。它会继承终端 PATH，之后窗口会连接到它。
- 把 Agent 安装到系统 PATH，例如 `/usr/local/bin`。

Pane 中的 shell 是非登录 shell。macOS 不读取 `~/.zprofile`，只读取 `~/.zshrc`。

## 远程 Device 显示版本不同

Device 标题上的黄色三角表示两边 Condr 构建不同。悬停查看应更新哪边。两边安装同一版本后，在远程 Device 上运行 `condr server restart`，或 **Settings › Device › General** → **Restart Condr**。

红色警告「speak different protocol versions」表示版本差距太大。连接前必须把两边更新到同一版本。

## 远程 Device 无法连接

先看 Device 标题提示，或打开 Device 后点 **Details**。

**SSH could not reach the device.** SSH 未连接成功，或远端没有 `condr`。在终端运行 `ssh user@host`，登录成功后运行 `ssh user@host condr --version`。如果提示 `command not found`，安装脚本加入的 PATH 对非交互 shell 不生效，请在链接中指定路径：

```text
ssh://user@host?bin=/home/user/.local/bin/condr
```

Condr 不会提示输入密码。先在终端配置密钥、ssh-agent 和主机信任。

**SSH reached the device, but Condr is not running there.** 远端 Server 未运行，自动启动也失败。请在远端运行 `condr server start`，查看输出。

**this device is not authorized.** invite 已过期、已使用或该 Device 被撤销。在远端重新运行 `condr server invite`，10 分钟内粘贴链接。

**Peer-to-peer 无法连接，但 TCP 和 SSH 正常。** 远端没有启用 `--p2p`，或中继和 DNS 服务暂时不可用。在远端运行 `condr server status`，确认 Peer-to-peer 已开启。

**Connecting 转圈超过 10 秒。** TCP 超时为 10 秒，SSH 超时为 15 秒。检查地址、端口和防火墙。

Condr 启动时第一次失败的连接不会自动重试。请在 Device 页面点 **Connect**。

## Condr 无法安装或打开

- **Windows SmartScreen 拦截。** 点 **更多信息** → **仍要运行**。预览安装包没有签名。
- **Linux AppImage 打不开。** 需要 FUSE。安装发行版的 `libfuse2` 包，或用 `--appimage-extract` 解开后运行。
- **另一个 Condr 已在运行。** 窗口只允许一个实例，第二个会立即退出。

## 配置修改没有生效

- 手改 `config.toml` 的 `[client]` 键要重启窗口，`[server]` 键要重启 Server。界面修改会立即生效。
- 文件格式错误时所有键使用默认值。窗口启动时提示「Failed to load」，并禁用 Device 列表编辑。
- Windows 上编辑器锁定文件时，Condr 会原地覆盖，编辑器可能提示文件已变化。

## 提 Issue 时附上这些信息

在 [GitHub Issues](https://github.com/condrdev/condr/issues) 提 Issue，并附上：

1. Condr 版本和平台。窗口版本在 **Settings › About**，Server 版本运行 `condr --version` 查看。
2. `condr server status --json` 的输出。
3. 问题前后的日志片段，尽量用 `CONDR_LOG=debug` 复现一次。
4. 简短的复现步骤。
5. 涉及远程 Device 时，两边版本和连接类型。

不要公开报告安全问题。见[安全模型](/zh-cn/docs/help/security/#报告安全问题)。
