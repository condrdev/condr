---
title: 故障排查
description: 先查看 Server 状态和日志，再按症状排查启动、连接、Agent 状态与配置问题，并准备提交 Issue 所需的信息。
---

排查问题时，建议先检查 Server 运行状态与相关日志，随后根据具体症状参考对应小节进行处理。

---

## 查看 Server 状态

在部署并运行 Server 的设备上执行以下命令：

```sh
condr server status
```

* **输出指标**：包含运行状态、socket 路径、设备指纹、程序版本与协议号、持续运行时长、TCP 监听地址、Peer-to-peer 状态，以及当前维护的 Workspace、Tab、Pane、Agent、窗口和 TCP 设备总数。
* **Pending 状态**：若已持久化的监听地址或 Peer-to-peer 配置与当前内存运行态不一致，输出中会包含一行 `Pending`，提示需重启 Server 使修改生效。
* **Recent errors**：按时间正序列出当前 Server 启动后产生的最近 20 条警告与错误记录。
* **JSON 输出格式**：追加 `--json` 参数可直接输出结构化 JSON 数据，便于随 Issue 提交反馈。
* **退出状态码**：若 Server 处于未运行状态，命令将以退出码 1 终止并打印失败原因。

如需查看远程设备状态，请前往 **Settings › Device › General**。

---

## 查看日志

| 日志类别 | 文件名模式 | 轮转与生命周期说明 |
| :--- | :--- | :--- |
| Server 日志 | `condr-server-<id>.<date>.log` | 按天切分，自动保留最近 7 个日志文件 |
| 窗口日志 | `condr-gui.<date>.log` | 按天切分，自动保留最近 7 个日志文件 |
| Server 标准错误输出 | `condr-server-<id>.stderr` | 捕获后台 Server 进程的 stdout 与 stderr。用于诊断 Server 启动失败或异常崩溃问题，该文件不执行轮转 |

默认日志存储目录请查阅[配置与设置](/zh-cn/docs/reference/configuration/)。亦可在界面通过 **Settings › Application › Developer › Locations**，定位到 Logs 一栏并点击 **Open** 直达对应目录。

### 提高日志详细程度

请在独立于 Condr 之外的外部终端中执行以下命令：

```sh
CONDR_LOG=condr_server=debug,condr_core=debug condr server restart
```

* **执行环境要求**：不可在 Condr 内部的 Pane 中执行 `condr server restart`，该操作会直接终止所有活跃 Pane，命令调用会被拦截拒绝。
* **配置语法**：`CONDR_LOG` 环境变量采用 `tracing` 框架的 EnvFilter 语法，传入后会覆盖系统默认的过滤策略。若单独设置为 `debug`，将一并启用所有底层依赖库的调试日志，产生大量输出。
* **语法异常回退**：若给定的过滤规则解析失败，Condr 将自动回退至默认过滤规则，并在 Recent errors 中追加一条警告记录。
* **窗口日志级别**：窗口客户端同样支持读取 `CONDR_LOG` 变量。若需提升窗口自身日志级别，需先在外部终端配置该环境变量后再行启动窗口。由该窗口衍生启动的 Server 实例将自动继承当前环境下的环境变量配置。

---

## 窗口无法启动本机 Server

当窗口尝试连接本机且未发现处于运行态的 Server 时，将自动拉起新实例并等待就绪握手。若拉起失败，连接面板将呈现如下提示：

```text
Condr's own server could not start on this device. Connect tries again.
```

面板伴随的具体错误诊断中会包含 `condr-server did not become ready`、对应 `.stderr` 文件的路径，以及该文件中本次启动以来的输出。常见故障诱因包括：

* **TCP 监听地址绑定失败**：`.stderr` 出现 `failed to listen on tcp://…`，导致 Server 初始化流程阻断。解决方案为变更端口，或直接移除 `config.toml` 内的 `[server] listen` 字段。
* **设备密钥文件权限过宽**：启用 TCP 监听时 Server 需校验并读取本机设备密钥。在 Linux 及 macOS 操作系统下，若密钥文件具备组或其他用户读取权限，`.stderr` 将记录 `… is readable by other users (mode …); make it 0600 or delete it`。依照提示执行权限变更，将其重置为 `0600`。
* **缺少 `condr` 可执行程序**：报错明确指出 `condr is not installed beside condr-gui`。发行版桌面应用已默认打包 `condr`。此问题通常发生在自行编译了窗口客户端但未配套编译 `condr` CLI 的场景。

若 `[server] listen` 配置的格式解析异常，Server 不会报错阻断，而是将其降级视为未配置监听。Peer-to-peer 组件启动异常亦不会导致整个 Server 崩溃，对应事件将以 `p2p endpoint not bound` 写入日志。

---

## 停止 Server 后又自动启动

在窗口保持打开的前提下于外部执行 `condr server stop`，窗口客户端会在约 0.5 秒的重连周期后检测到服务脱机，并立刻自动派生拉起新的 Server 进程。如需彻底关闭 Server，必须先退出窗口程序，再行执行 `condr server stop`。

---

## 两端版本不一致

* **构建版本差量**：界面中设备标签旁将显示黄色警示三角。将光标悬停其上可对比两端具体版本号并获取更新建议。点击警示三角可暂时收起提示，该状态保持至该设备版本再次变动。
* **本机 Server 滞后于窗口**：完成 Condr 升级后，窗口客户端将弹出提示询问是否重启本机 Server。执行重启将销毁所有终端运行进程，随后重新载入当前的 Workspace 拓扑与 Agent 对话状态。
* **协议底层不兼容**：连接将被远端阻断，连接面板呈现 `Condr there and here speak different protocol versions`，CLI 端同步抛出 `protocol_mismatch` 错误代码。该状态不可通过重试恢复，必须将两端程序版本升级至匹配状态。

在两端均更新至匹配版本后，请转至目标设备并在外部终端执行 `condr server restart`，或于 GUI 界面访问 **Settings › Device › Remote access** 点击 **Restart Condr**。该操作按钮仅在本地会话或底层通过 SSH 建立连接时可用。

---

## Agent 状态一直是 Unknown

Agent 状态变更完全依赖 Hook 上报机制。请按序核验以下环节：

1. **检查 Hook 安装状态**：执行 `condr agent hooks status <agent>`（例如 `condr agent hooks status claude`）。返回 `missing` 时执行安装；返回 `outdated` 时执行重新安装；返回 `unsupported` 则代表该 Agent 尚未受支持。针对远程设备，需进入 **Settings › Device › Agent integrations** 查看与配置，命令行 `agent hooks` 指令作用域仅限本机环境。
2. **确认运行环境位于 Condr Pane 内部**：Hook 仅在环境变量预注入 `CONDR_ENV=1` 的终端会话中触发上报。Condr 无法监视外部独立终端中运行的 Agent 进程。
3. **识别特定 Agent 的初始上报行为**：Codex、Copilot 及 Antigravity 在收到首次 prompt 输入前不发送状态；Cursor 在执行会话恢复（Resume）期间亦不触发上报，此阶段状态将始终显示为 Unknown。此外，Kimi 当前尚未支持状态上报机制。
4. **验证 Codex 的 Hook 信任授权**：Codex 要求必须开启 hooks 特性开关并显式授予信任。执行 Condr Hook 安装流程时会尝试自动配置；若配置失败，需手动在 Codex 的 `config.toml` 内追加 `[features] hooks = true`，随后进入 Codex 终端执行 `/hooks` 命令完成信任授权确认。
5. **激活 Antigravity 插件配置**：完成 Hook 文件部署后，需在 Antigravity CLI 交互环境中手动启用 condr 插件。
6. **核对 Hook 写入路径一致性**：安装阶段 Condr 根据当前上下文的环境变量（例如 `CLAUDE_CONFIG_DIR`、`CODEX_HOME`）决定 Hook 部署路径。CLI 操作依据当前终端所处环境变量，而图形设置界面基于 Server 进程的环境变量。若 Agent 运行期所寻址的环境变量与 Hook 部署期不一致，将无法加载 Hook 脚本。

---

## Pane 中找不到 Agent

* **Server 进程的 PATH 继承源**：Condr 依赖 Server 自身的 PATH 检索系统中的 Agent，该变量由拉起 Server 的父进程传递。在 macOS 系统下，若直接通过 Dock 或 Finder 启动 GUI，由此派生启动的 Server 仅包含操作系统预置的默认 PATH 集合。可执行 `condr agent available` 打印 Server 实际可感知的 Agent 列表。
* **PATH 作用域修正策略**：在 PATH 已包含该 Agent 的外部终端中执行 `condr server restart`，随后窗口将重新接入该 Server 实例。或选择将目标 Agent 的可执行文件软链/安装至系统全局标准路径（如 `/usr/local/bin`）。
* **Pane 内 Shell 类型的配置文件差异**：macOS 默认的 zsh 会话仅加载 `~/.zshenv` 与 `~/.zshrc`，跳过 `~/.zprofile`；Linux 默认的非登录 bash 会话仅读取 `~/.bashrc`。配置于 `~/.zprofile` 或 `~/.profile` 内的 PATH 变更不会在 Pane 内部会话生效。

---

## 远程设备无法连接

请打开目标设备专属页面，或点击 Workspace 顶部断网横幅中的 **Details**，对照下表定位具体建议及对应原因：

| 诊断提示 | 故障原因与排查步骤 |
| :--- | :--- |
| `SSH could not reach the device.` | 在终端执行 `ssh user@host`，确认基础登录可用后，进一步运行 `ssh user@host condr --version`。若报告找不到程序，请参考下文在连接串中显式声明绝对路径 |
| `SSH reached the device, but Condr is not running there.` | Condr 已在远端触发启动 Server 尝试但未成功拉起通信。需直接登入该设备执行 `condr server start`，捕获终端报错输出 |
| `The device refused this connection.` | 目标设备未配置互信、认证已作废，或远端设备私钥已变更。在目标设备重新生成 `condr server invite`，使用全新链接重新配对 |
| `Nothing answers at this address.` | 远端设备上的 Server 尚未启动，或本地配置的网络地址、通信端口存在拼写错误 |
| `The device did not answer.` | 目标设备处于休眠挂起、断网状态，或网络流量遭到防火墙阻断。TCP、SSH 及 Peer-to-peer 机制均在持续超时 10 秒后终止握手 |
| `This device cannot reach that address.` | 本地网络路由异常或 VPN 阻断了通信，请核查本机网络与 VPN 设置 |
| `The device closed the connection.` | 目标设备上的 Server 进程已停止、意外终止或正在重启 |

### SSH 找不到 `condr`

安装脚本默认向 Shell profile 追加的 PATH 配置在非交互式 SSH 会话中无法被解析。此时需在 SSH 协议地址参数中显式写入 `condr` 完整绝对路径：

```text
ssh://user@host?bin=/home/user/.local/bin/condr
```

Condr 以纯非交互模式执行底层 `ssh` 进程，期间无法提供交互式密码输入框，亦不支持手动确认未知的主机密钥指纹。请预先在系统终端中配置免密认证（公钥互信或 ssh-agent），并预先将主机密钥计入 `known_hosts`。

### Peer-to-peer 连不上

* **远端功能未启用**：在远端设备执行 `condr server status`，确认 Peer-to-peer 标志位已置为开启。
* **邀请凭据失效**：诊断原因显示 `invite unknown, used or expired` 时，表明邀请码已过期或已被消耗，必须在服务端重新生成凭据。
* **本机中转 Server 未运行**：P2P 会话需借由本机 Server 发起通信路由。诊断日志若提示 `this machine's Server is unreachable`，请先确保本机的 Server 处于正常运行状态。

---

## 自动重连的规则

* **冷启动初次建连失败**：系统不触发自动重试机制。需在设备管理面板手动点击 **Connect**。
* **通信链路异常中断**：系统将启动每隔 0.5 秒一次的重连循环，重连上限持续 45 秒。断线告警 UI 会在重连持续 2 秒后对外呈现。
* **系统自休眠中唤醒**：若心跳探测连续 10 秒缺失应答，窗口客户端判定连接超时失效并切入自动重连流水线。
* **远端协议版本超前**：若对端设备持续向本机推送本地无法解析的新结构消息，窗口将主动断开链路并停止后续重试，上报错误：`this Device sends messages this build cannot read; update it`。

---

## 安装与启动

* **Windows SmartScreen 拦截提示**：点击弹窗中的 **更多信息** → **仍要运行**。预览版本安装二进制尚未集成机构代码签名证书。
* **Linux AppImage 缺少依赖无法启动**：AppImage 打包规范强依赖 FUSE 用户态文件系统支持。若运行环境缺失 FUSE，需追加 `--appimage-extract-and-run` 参数启动程序。
* **重复执行启动无任何窗口弹出**：Condr 强制实施单实例互斥锁，多余的派生实例将静默退出，不会唤醒当前已存在的窗口句柄。请检索系统任务栏或其余虚拟桌面确认主实例是否已在后台运行。
* **Windows Pane 运行异常报错 448**：例如 pnpm 工具链抛出 `untrusted mount point`。此问题系 Server 继承了其父进程的 Redirection Guard 隔离限制，并下发至内部所有子 Pane 进程。请妥善保存 Pane 中的当前改动，随后于外部标准系统终端中运行 `condr server restart`，使 Server 在不受限的上下文环境中重新生成。

---

## 配置修改没有生效

* **手工修改的重载流程**：手动编辑 `[client]` 及其子表配置后，需重启窗口生效；手动编辑 `[server]` 及其子表配置后，必须在终端执行 `condr server restart`。通过图形界面修改的设置会立即生效，Remote access 页除外。
* **配置文件语法损毁**：若 TOML 语法存在错误，窗口启动将拦截报错：`Failed to load … Device list changes are disabled; fix the file and restart Condr.`。此时系统回退至初始默认配置且禁止变更设备列表。修复格式后重启窗口即可恢复。
* **Windows 系统外部编辑器变动告警**：在第三方编辑器抢占 `config.toml` 文件句柄时，若 Condr 触发了全量覆盖写入，外部编辑器通常会触发外部文件改动同步警报。

---

## 提交 Issue

请统一在 [GitHub Issues](https://github.com/condrdev/condr/issues) 渠道跟踪反馈，工单需完整附带下列要素：

1. **版本信息**：窗口客户端版本号可在 **Settings › Application › About** 查询。当前运行态 Server 版本以 `condr server status` 报告的 Version 字段为准；执行 `condr --version` 仅反映当前可执行文件版本，可能与常驻运行态 Server 存在偏差。
2. **状态快照**：附带 `condr server status --json` 的控制台输出。
3. **关键日志**：附带故障发生节点前后的日志片段（推荐事先上调日志详细度复现异常后抓取）。
4. **稳定复现步骤**：清晰的操作路径说明。
5. **网络与设备拓扑**：凡涉及远程设备联动故障，务必注明通信两端具体的运行版本与当前生效的连接方式。

涉及潜在安全性漏洞的报告请勿使用公开 Issue 跟踪，具体流程遵循[安全模型](/zh-cn/docs/help/security/#报告安全问题)的私密指引。
