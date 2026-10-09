---
title: 配置与设置
description: Condr 的配置体系详解：UI 与 config.toml 设置、完整配置项、环境变量与各平台存储路径。
---

Condr 采用分层配置架构：
* **客户端设置**：控制前端外观与交互逻辑，仅作用于当前设备。
* **Server 设置**：控制终端实例、Worktree 及远程连接策略，作用于运行 Server 的宿主设备。
* **整机设置**：整台机器共用一组配置，本机上的所有 Condr 进程均会读取，例如代理设置。

所有配置均保存在同一份 `config.toml` 中，可在图形界面中调整，也可直接编辑文件。

---

## 图形设置界面

* **快捷键**：macOS 为 `Cmd + ,`，Windows / Linux 为 `Ctrl + ,`。
* **自动保存**：设置项变更会立即同步至 `config.toml`。文本输入框失焦、回车或点击 **Save** 时触发写入。

设置窗口一次只配置一台设备。顶部选择器可切换设备（默认选中本机），并显示连接方式与在线状态。本机显示所有页面；远程设备仅显示下表中标有 **设备** 的页面；未连接的设备在连上之前不会显示页面。

| 页面 | 作用于 | 说明 |
| :--- | :--- | :--- |
| **General** | 设备 | **Status**：版本、运行时间、会话指标（Workspace 和 Pane 数量）以及最近错误日志。**Terminal**：新 Pane 启动的默认 Shell（留空则继承系统环境） |
| **Appearance** | 本机 | 外观主题（跟随系统、浅色、深色）、终端字体与配色，以及 Preview 和 Diff 的语法高亮与字号 |
| **Notifications** | 本机 | 系统通知总开关及测试通知按钮 |
| **Power** | 本机 | 保持屏幕常亮（与侧边栏底部的咖啡杯图标联动） |
| **Shortcuts** | 本机 | 快捷键列表（只读） |
| **Network** | 设备 | 该设备用于 Peer-to-peer 连接和检查更新的代理：**System**、**None**，或选择 **Manual** 并填写 URL |
| **Remote access** | 设备 | TCP 监听开关与地址、Peer-to-peer 开关，以及重启 Server 的 **Restart Condr** 按钮。修改需要重启才能生效，修改后按钮会变为 **Restart to apply** |
| **Paired devices** | 设备 | 点击 **Generate invite** 生成单次配对邀请（包含 P2P 和 TCP 两种链接）。列表优先显示在线设备，点击每行的 **Revoke** 可解除配对 |
| **Agent integrations** | 设备 | 查看各 Agent 的 Hook 状态，支持一键安装、更新或卸载 |
| **Developer** | 本机 | 帧率监控开关，以及快速打开应用、配置、数据、状态和日志目录的按钮 |
| **Licenses** | 本机 | 第三方开源依赖及开源协议 |
| **About** | 本机 | 版本信息、更新通道选择、手动检查更新及自动检查开关 |

> **权限限制**：通过 TCP 或 P2P 连接时，决定其他设备如何连入这台设备的设置（`Network` 中的代理，以及 `Remote access` 中的所有项，包括 **Restart Condr**）会置灰禁用，以防止改动导致当前连接中断；悬停可查看说明。如需修改，请在该设备上操作或通过 SSH 连接修改。其余设置均可正常调整，包括 **Generate invite**，以及对当前所用设备之外的任意设备执行 **Revoke**。

---

## 配置文件 `config.toml`

GUI 客户端、后台 Server 与 `condr` CLI 共用位于配置目录下的 `config.toml`（见[存储目录与文件系统](#存储目录与文件系统)）。所有字段均可选，缺省时载入系统内置默认值。

* **写入机制**：Condr 写入配置时使用文件锁规避并发竞争，并完整保留文件内的自定义注释与排版结构。
* **生效规则**：
  * 在图形界面修改的设置通常立即生效，但 `Remote access` 页面的网络设置和 Peer-to-peer 代理除外。
  * 手动编辑 `[client]` 及其子表后，重启 GUI 窗口生效。
  * 手动编辑 `[server]` 及其子表后，需在终端运行 `condr server restart` 重启 Server。
  * 手动编辑 `[network]` 后，上述两项操作均需执行。
* **容错降级**：若配置文件出现语法或解析错误，Condr 会在窗口报错警示，并回退至全套默认参数运行。在修复语法错误前，UI 界面的改动将无法持久化。

---

## 客户端配置项

### `[client]` 基础偏好

```toml
[client]
appearance = "dark"
notifications = true
keep_awake = false
fps_monitor = false
```

| 键 | 类型 / 可选值 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `appearance` | `system` / `light` / `dark` | `system` | 界面色彩模式。非法值自动降级为 `system` |
| `notifications` | bool | `true` | Agent 任务完成或需介入确认时的系统通知 |
| `keep_awake` | bool | `false` | 保持屏幕常亮，并阻止系统因空闲而休眠 |
| `fps_monitor` | bool | `false` | 在界面叠加显示实时帧率与资源占用 |
| `editor` | string | 空 | 上次通过 **Open in** 调用的编辑器名称，亦作为全局默认值（由 Condr 内部维护写入） |

### `[client.terminal]` 终端显示

```toml
[client.terminal]
font_family = "JetBrains Mono"
font_size = 13
color_scheme = "Dracula"
```

| 键 | 类型 / 可选值 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `font_family` | string | 界面主题等宽字体 | 终端渲染字体。留空时使用界面内置默认字体 |
| `font_size` | 6 ~ 72 | 界面等宽基准字号 | 像素单位。浮点数自动四舍五入，溢出范围截断至边界值 |
| `color_scheme` | string | 空 | 内置 iTerm2 主题标识，需与 **Appearance** 下拉项严格一致。缺省或无法识别时使用默认调色板 |

### `[client.code]` 代码视图（Preview / Diff）

```toml
[client.code]
theme = "Dracula"
font_size = 14
```

| 键 | 类型 / 可选值 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `theme` | string | 空 | 语法高亮主题，需匹配 **Appearance › Syntax** 选项。缺省或无法识别时，浅色界面使用 GitHub Light，深色界面使用 GitHub Dark |
| `font_size` | 6 ~ 72 | 界面等宽基准字号 | 代码视图字号，边界与取整逻辑同终端配置 |

### `[client.updates]` 软件更新

```toml
[client.updates]
auto_check = true
channel = "stable"
```

| 键 | 类型 / 可选值 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `auto_check` | bool | `true` | 自动检查更新（客户端启动 5 秒后首次运行，随后以 5 小时为周期轮询）。仅推送更新通知，不静默安装 |
| `channel` | `stable` / `nightly` | 当前版本通道 | 目标更新流。通道特性详见[版本发布说明](/zh-cn/docs/start/install/#版本发布通道) |

### `[[client.servers]]` 远程设备列表

```toml
[[client.servers]]
name = "build-box"
address = "ssh://rocky@build-box"

[[client.servers]]
name = "home"
address = "tcp://<device key>@192.168.1.20:2637"
```

通过数组表管理远程设备：

* `name`：设备在侧边栏中的名称，同时作为 `condr --device <name>` 的路由参数。
* `address`：连接 URI，格式需以 `ssh://`、`tcp://` 或 `p2p://` 开头（不含一次性 Invite 密钥）。各协议格式要求参见[远程连接规范](/zh-cn/docs/using/remote/)。

> 该列表由 GUI 自动持久化管理；CLI 仅具备只读权限。若某个设备的 URI 解析失败，Condr 会跳过该项并提示错误，其余设备照常连接。

### `[[client.editors]]` 外部编辑器注册

```toml
[[client.editors]]
name = "Helix"
command = ["hx"]
```

向右键 **Open in** 级联菜单注册外部可执行程序：

* `name`：菜单中渲染的项目展示名。
* `command`：可执行程序及前置参数。Condr 在拉起进程时会将目标文件/目录路径作为末尾参数传入。

手工编辑此块需重启客户端生效。Condr 针对各项目的选定记录存放于 `[[client.workspace_editors]]`（此表供运行时内部存储，不建议手动改动）。详细示例见[首选项配置](/zh-cn/docs/using/preferences/#自定义外部编辑器-open-in-editor)。

---

## Server 端配置项

Server 配置仅在守护进程冷启动时装载。手动编辑后，必须通过 `condr server restart` 重新加载。

### `[server]` 网络监听

```toml
[server]
listen = "0.0.0.0:2637"
```

| 键 | 类型 / 格式 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `listen` | `IP:PORT` | 空 | 附加 TCP 监听地址（必须为标准 IP 地址，不支持主机名域名解析）。未设置时不监听 TCP，仅接受本机 socket 连接。执行 `condr server start --listen` 会自动回写该项 |

### `[server]` Worktree 存放位置

```toml
[server]
worktree_root = "~/worktrees"
```

| 键 | 类型 / 格式 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `worktree_root` | path | 空 | 统一托管 Git Worktree 的物理根路径 |

`worktree_root` 的路径风格直接决定 Worktree 的磁盘存放结构：

| 模式 | 解析规则 | 目录结构示例 |
| --- | --- | --- |
| **缺省** | 依附各代码仓库同级 | `<repo>.worktrees/<branch>` |
| **绝对路径 / `~` 开头** | 统一收拢至指定根目录 | `<root>/<repo>/<branch>` |
| **相对路径** | 挂载于各仓库根目录内部 | `<repo>/<root>/<branch>` |

*注：分支路径中除字母、数字、`.`、`_`、`-` 外的特殊字符统一转义为 `-`。例如按上方示例配置，仓库 `my-app` 的分支 `feat/login` 将映射至 `~/worktrees/my-app/feat-login`。*

### `[server.p2p]` 点对点连接

```toml
[server.p2p]
enabled = true
```

| 键 | 类型 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `enabled` | bool | `false` | 启用 P2P 穿透服务。执行 `condr server start --p2p` 时自动置为 `true` |

### `[server.terminal]` 终端环境

```toml
[server.terminal]
shell = "/opt/homebrew/bin/fish"
```

| 键 | 类型 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `shell` | path | 空 | 新 Pane 派生进程的可执行路径。缺省状态下，Unix-like 系统读取 `$SHELL`，Windows 依序探测 `pwsh.exe`、`powershell.exe` 与 `%ComSpec%` |

若在 **Settings › General** 界面中修改 Shell，系统将直接热更新：后续新建的 Pane 立即生效，无须重启 Server。

---

## 整机配置项

不带 `client` 或 `server` 前缀的配置表由该机器上的所有 Condr 进程读取。在 GUI 中，它们与 Server 设置一样通过该机器的 Server 进行修改。

### `[network.proxy]` 代理

```toml
[network.proxy]
mode = "manual"
url = "http://user:password@proxy:8080"
```

| 键 | 类型 / 取值 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `mode` | `system` / `none` / `manual` | `system` | `system` 优先读取 `HTTPS_PROXY`（或 `ALL_PROXY`）与 `NO_PROXY`，再读取 Windows 或 macOS 系统代理。`none` 完全不使用代理。`manual` 使用 `url` 的配置 |
| `url` | `http://` 或 `https://` URL | 空 | `manual` 模式使用的代理地址，若需认证可包含 `user:password@`。切换到其他模式时保留该值。在 `manual` 模式下留空表示不使用代理 |

代理设置作用于 Server 连接 Condr relay 的 Peer-to-peer 网络连接，以及窗口的检查更新。Peer-to-peer 在 Server 重启后应用新代理；检查更新则立即生效。

---

## 环境变量参考

| 变量 | 描述 |
| --- | --- |
| `CONDR_LOG` | 基于 `tracing` 的 EnvFilter 日志过滤规则（例如 `debug` 或 `condr_server=trace`）。默认：`warn,condr_core=info,condr_server=info,condr_gui=info` |
| `CONDR_LOG_DIR` | 自定义日志落盘目录 |
| `CONDR_CONFIG_DIR` | 自定义配置文件根目录 |
| `CONDR_DATA_DIR` | 自定义应用持久化数据目录 |
| `CONDR_SOCKET_PATH` | 自定义本机 Server 的 IPC 路径（Unix domain socket 或 Windows 命名管道） |
| `CONDR_DEVICE` | 指定 `--device` 参数的全局缺省目标 |
| `CONDR_VERSION` | 安装脚本抓取的软件版本（例如 `nightly` 或 `v0.1.0`） |
| `CONDR_INSTALL_DIR` | 安装脚本与 `condr server install` 部署二进制文件的目标目录 |
| `CONDR_INSTALL_ARGS` | Windows 安装脚本透传给 `condr server install` 的扩展入参（如 `--force`） |

> **进程上下文继承**：Server 会完整继承启动它的进程环境，不会自行读取 Shell 配置文件。例如在终端执行 `CONDR_LOG=debug condr server restart` 即可重启 Server 并开启 Debug 日志。修改 `.zshrc` / `.bashrc` 后，需在已加载新配置的终端中执行 `condr server restart`；设置界面的 **Restart Condr** 会沿用旧 Server 的环境变量。

每个 Pane 会话自动挂载的环境变量见 [Agent 自动化指南](/zh-cn/docs/using/automation/)。

---

## 存储目录与文件系统

### 跨平台目录映射

| 分类 | Linux | macOS | Windows |
| --- | --- | --- | --- |
| **配置** (`config.toml`) | `~/.config/condr` | `~/Library/Application Support/condr` | `%APPDATA%\condr` |
| **数据** (设备公私钥、配对表) | `~/.local/share/condr` | `~/Library/Application Support/condr` | `%LOCALAPPDATA%\condr` |
| **状态** (Session 拓扑快照) | `~/.local/state/condr` | `~/Library/Application Support/condr` | `%LOCALAPPDATA%\condr` |
| **日志** | `~/.local/state/condr` | `~/Library/Logs/condr` | `%LOCALAPPDATA%\condr` |
| **运行时** (Socket / 临时文件) | `$XDG_RUNTIME_DIR/condr` | `$TMPDIR/condr` | `%LOCALAPPDATA%\condr\runtime` |
| **二进制** (`condr` 核心程序) | `~/.local/opt/condr` | `~/.local/opt/condr` | `%LOCALAPPDATA%\Programs\Condr` |

* **XDG 兼容**：Linux 平台严格遵循 `XDG_CONFIG_HOME`、`XDG_DATA_HOME` 与 `XDG_STATE_HOME` 规范。未指定 `XDG_RUNTIME_DIR` 时自动回退至 `~/.local/share/condr/runtime`。
* **快速访问**：UI 界面可在 **Settings › Developer › Locations** 点击对应按钮直达目标路径。

### 核心运行时文件

| 文件名 | 所属范畴 | 功能定义 | 移除影响 |
| --- | --- | --- | --- |
| `config.toml` | 配置 | 全局窗口与 Server 核心配置 | 所有参数回退为默认值，已保存的远程连接列表清空 |
| `device-key` | 数据 | 设备非对称加密密钥 | 下次启动重新生成公私钥对并清空配对表；既有对端连接需重新授权 |
| `authorized-clients` | 数据 | 配对白名单列表（每行一台设备） | 已配对设备下次连接时被拒绝，需重新配对 |
| `pending-invite` | 数据 | 当前生效的一次性配对票据 | 对应邀请链接立即失效 |
| `condr-server-<id>.snapshot` | 状态 | Session 结构快照（Workspace、Tab 与 Pane 布局） | *需停机操作*。下次冷启动时丢弃所有会话上下文（磁盘已检出 Worktree 不受损） |
| `condr-gui.state` | 状态 | 前端窗口几何尺寸、分栏折叠与各 Workspace 打开的 Tab 状态 | 窗口重置为初始居中尺寸与默认布局 |
| `condr-server-<id>.<date>.log` | 日志 | Server 运行流水（按日轮转，仅滚动保留近 7 份） | 无直接影响 |
| `condr-gui.<date>.log` | 日志 | 前端渲染流水（轮转规则同上） | 无直接影响 |
| `condr-server-<id>.stderr` | 日志 | 后台崩溃或未捕获的 Panic 堆栈输出 | 无直接影响 |

> **安全与权限约束**：密钥与 Invite 属于敏感鉴权文件。在 Linux / macOS 下，权限必须限制为 `0600`（所有者读写），任何组读或其他可读权限将导致 Condr 拒绝加载；Windows 环境下程序会自动配置 ACL，将其严格收敛至当前用户上下文。

:::note[目录演进兼容性]
0.1.6 及更早版本将 `device-key`、`authorized-clients` 及 `pending-invite` 存放在配置目录下。新版本初次启动时，会自动将上述状态迁移至数据目录，密钥及配对拓扑保持兼容无损。
:::

---

## 延伸阅读

* [快捷键与偏好设置](/zh-cn/docs/using/preferences/)：按键映射、外部编辑器挂载与默认终端 Shell 调整
* [远程连接与网络拓扑](/zh-cn/docs/using/remote/)：配置 TCP 直连与 P2P 穿透的端到端安全实践
* [CLI 参考手册](/zh-cn/docs/reference/cli/)：命令行语法与 Server 后台服务生命周期管理
* [故障排查指南](/zh-cn/docs/help/troubleshooting/)：常见连接中断、权限被拒与崩溃日志分析
