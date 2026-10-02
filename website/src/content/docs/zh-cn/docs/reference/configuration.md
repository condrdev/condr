---
title: 配置与设置
description: 修改窗口和 Server 设置，并查找各平台上的 Condr 文件。
---

用这一页找到设置、确认何时生效，并定位 Condr 保存的文件。

## 在界面里修改设置

在 macOS 上按 Cmd+,，其他平台按 Ctrl+,，打开设置。窗口有两个标签。**Application** 包含窗口自己的设置，**Device** 包含某台 Device 的 Server 设置。Device 标签的页面上方有一栏，显示当前编辑的是哪台 Device、它的连接方式和连接状态，在那里切换 Device。

每项修改立即生效并写入配置文件。文本框会在你离开、按 Enter 或点 Save 时保存。

**Application**

| 页面 | 设置 |
| --- | --- |
| Appearance | 主题（跟随系统、浅色、深色），终端字体、字号、配色方案，Preview 和 Diff 的高亮主题和字号 |
| Notifications | 开关系统通知，发送测试通知 |
| Power | 保持屏幕唤醒。侧栏底部的咖啡杯使用同一个开关 |
| Shortcuts | 快捷键列表，只读 |
| Developer | 帧率监视器，打开应用、配置、状态和日志目录的按钮 |
| Licenses | Condr 用到的第三方组件及其许可证 |
| About | 版本、更新渠道、自动检查更新、立即检查，有新版本时 Updates 组显示版本号和 **View** 按钮 |

**Device**

| 页面 | 设置 |
| --- | --- |
| General | Status 组显示连接方式、版本、运行时长、计数、最近错误，以及 Server 实际绑定的监听地址和 Peer-to-peer 状态，并有 **Restart Condr** 按钮。Terminal 组设置新 Pane 使用的 shell，留空则使用系统默认值 |
| Remote access | TCP listener 开关和 Listen address，Peer-to-peer 开关。改动要重启 Server 才生效，General 页的按钮会变成 **Restart to apply** |
| Paired devices | **Generate invite** 生成一次性 invite，并按 Peer-to-peer 和 TCP 各列出一条链接供复制。已配对 Device 列表，已连接的排在前面并带绿点，每行有 **Revoke** |
| Agent integrations | 每个 Agent 的 hook 状态，以及安装、更新和卸载 |

Remote access 和 Paired devices 页面只能通过本机或 SSH 连接修改。通过 TCP 或 Peer-to-peer 连接时，Device 标签顶部显示「Viewing only」，这些控件不可用。

## 文件在哪

| 内容 | Linux | macOS | Windows |
| --- | --- | --- | --- |
| 配置：`config.toml`、密钥 | `~/.config/condr` | `~/Library/Application Support/condr` | `%APPDATA%\condr` |
| 状态：快照、窗口状态 | `~/.local/state/condr` | `~/Library/Application Support/condr` | `%LOCALAPPDATA%\condr` |
| 日志 | `~/.local/state/condr` | `~/Library/Logs/condr` | `%LOCALAPPDATA%\condr` |
| 运行时：socket、临时文件 | `$XDG_RUNTIME_DIR/condr` | `$TMPDIR/condr` | `%LOCALAPPDATA%\condr\runtime` |
| 安装的 `condr` 命令 | `~/.local/opt/condr` | `~/.local/opt/condr` | `%LOCALAPPDATA%\Programs\Condr` |

Linux 上 `XDG_CONFIG_HOME` 和 `XDG_STATE_HOME` 照常生效。**Settings › Developer › Locations** 提供打开这些目录的按钮。

窗口和命令行共用 `config.toml`，你可以手动编辑。Condr 写入时保留注释，并用锁避免两个进程同时写入。文件格式错误时，所有键都会回退为默认值。

手改 `[client]` 键后要重启窗口，手改 `[server]` 键后要重启 Server。通过界面修改会立即生效。

## 配置 `[server]`

```toml
[server]
listen = "0.0.0.0:2637"
worktree_root = "~/worktrees"
```

| 键 | 含义 | 生效 |
| --- | --- | --- |
| `listen` | 额外监听的 TCP 地址，只能是 IP 和端口。不设置就不监听。`condr server start --listen` 会写入它。 | 重启 Server |
| `worktree_root` | Condr 创建的 worktree 存放位置。不设置时放在仓库旁的 `<repo>.worktrees/`。绝对路径或 `~`：`<root>/<repo>/<branch>`。相对路径：`<repo>/<root>/<branch>`。 | 重启 Server |

## 配置 `[server.p2p]`

```toml
[server.p2p]
enabled = true
```

`enabled` 让 Server 接受 Peer-to-peer 连接，默认关闭。`condr server start --p2p` 会写入它，重启 Server 后生效。

## 配置 `[server.terminal]`

```toml
[server.terminal]
shell = "/opt/homebrew/bin/fish"
```

`shell` 是新 Pane 启动的程序。留空时，Unix 使用 `$SHELL`，Windows 依次查找 `pwsh.exe`、`powershell.exe`、`%ComSpec%`。界面中修改后，下一个新 Pane 生效。手改需要重启 Server。

## 配置 `[client]`

```toml
[client]
appearance = "dark"
notifications = true
keep_awake = false
fps_monitor = false
editor = "zed"
```

| 键 | 含义 | 默认 |
| --- | --- | --- |
| `appearance` | `system`、`light` 或 `dark` | `system` |
| `notifications` | Agent 完成或需要你时发送系统通知 | `true` |
| `keep_awake` | 防止屏幕休眠 | `false` |
| `fps_monitor` | 显示帧率 | `false` |
| `editor` | 上次使用的「Open in」目标 | 无 |

## 配置 `[client.terminal]`

```toml
[client.terminal]
font_family = "JetBrains Mono"
font_size = 13
color_scheme = "Dracula"
```

`font_size` 的范围是 6 到 72。`color_scheme` 是内置 iTerm2 配色名。留空使用默认调色板。

## 配置 `[client.updates]`

```toml
[client.updates]
auto_check = true
channel = "stable"
```

`channel` 是 `stable` 或 `nightly`。不设置时跟随你安装的构建。自动检查在启动 5 秒后进行一次，之后每 5 小时一次。Condr 只提示，不自动安装。

## 在 `[[client.servers]]` 中保存远程 Device

```toml
[[client.servers]]
name = "build-box"
address = "ssh://rocky@build-box"

[[client.servers]]
name = "home"
address = "tcp://<device key>@192.168.1.20:2637"
```

这里保存远程 Device。`name` 是侧栏名称，也是 `--device` 的参数。`address` 是不含 invite 的 `ssh://`、`tcp://` 或 `p2p://` 地址。界面会在连接、编辑或删除 Device 时写入列表，命令行只读取它。

## 在 `[[client.editors]]` 中添加编辑器

```toml
[[client.editors]]
name = "Helix"
command = ["hx"]
```

这会把编辑器加入「Open in」。要打开的路径会追加为最后一个参数。手动修改后重启窗口。Condr 按仓库记住的选择写在 `[[client.workspace_editors]]` 中，不要手动修改。

## 设置环境变量

| 变量 | 作用 |
| --- | --- |
| `CONDR_LOG` | `tracing` EnvFilter 语法的日志过滤器，例如 `debug` 或 `condr_server=trace`。默认：`warn,condr_core=info,condr_server=info,condr_gui=info` |
| `CONDR_LOG_DIR` | 日志目录 |
| `CONDR_CONFIG_DIR` | 配置目录 |
| `CONDR_SOCKET_PATH` | 本机 Server 的 socket 路径 |
| `CONDR_DEVICE` | `--device` 的默认值 |
| `CONDR_VERSION` | 安装脚本安装的版本：`nightly` 或 `v0.1.0` |
| `CONDR_INSTALL_DIR` | 安装脚本和 `server install` 放置命令的位置 |
| `CONDR_INSTALL_ARGS` | Windows 安装脚本传给 `server install` 的参数 |

Server 继承启动它的进程环境。因此 `CONDR_LOG=debug condr server restart` 有效，但修改 `.zshrc` 后要重启 Server 才会生效。

Pane 中自动设置的变量见 [Agent 自动化与协同](/zh-cn/docs/using/automation/)。

## 查看 Server 的其他文件

| 文件 | 位置 | 作用 | 删除后 |
| --- | --- | --- | --- |
| `device-key` | 配置目录 | 这台 Device 的密钥 | 生成新密钥，清空授权列表，所有配对必须重做 |
| `authorized-clients` | 配置目录 | 已配对 Device，每行一个 | 每个已配对 Device 下次连接都会被拒绝 |
| `pending-invite` | 配置目录 | 当前有效的 invite | 该 invite 失效 |
| `condr-server-<id>.snapshot` | 状态目录 | Session 结构 | Server 停止时删除，下次启动为空。worktree 仍在磁盘上 |
| `condr-gui.state` | 状态目录 | 窗口位置、侧栏和每个 Workspace 显示的 Tab | 窗口用默认大小打开 |
| `condr-server-<id>.<date>.log` | 日志目录 | Server 日志，每天滚动并保留 7 天 | 无影响 |
| `condr-gui.<date>.log` | 日志目录 | 窗口日志 | 无影响 |

密钥和 invite 文件的权限必须是 0600。其他用户能读取时，Condr 会拒绝使用它们。
