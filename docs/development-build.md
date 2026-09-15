# Development Build

发布流程(nightly 与正式版)见 [releases.md](releases.md)。本文记录包结构、数据目录、各平台打包脚本与运行时行为。

开发时的模块入口、测试归档约定与检查命令见 [代码组织与测试归档](code-organization.md)。

## 产物

- 每次发布生成所有平台的 GUI + CLI 安装包与 CLI-only 归档:Linux x86_64/arm64 AppImage 与 tar.gz,Windows x86_64 安装器与 ZIP,macOS x86_64/arm64 `.dmg` 与 tar.gz。
- 所有 artifacts 必须来自同一 commit。文件名、release notes 和包内 `BUILD-COMMIT` 都记录该 SHA。
- Client 和 Server 没有跨构建兼容承诺,必须一起更新。

产物命名统一为 `condr-{version}[-{short_sha}]-{platform}-{arch}.{suffix}`(GUI + CLI)和 `condr-cli-{version}[-{short_sha}]-{platform}-{arch}.{suffix}`(CLI-only)。`platform` 为 `linux`、`macos` 或 `windows`;`arch` 为 `x86_64` 或 `arm64`(Linux 的 `aarch64` 也输出为 `arm64`)。Nightly 包含 12 位短 SHA,例如 `condr-0.1.0-d7912d3f186e-macos-arm64.dmg`;正式版省略 SHA:Unix 打包脚本设置 `CONDR_RELEASE=1`,Windows 打包脚本传入 `-Release`,完整 commit 仍写入包内 `BUILD-COMMIT`。Windows 安装器使用 `.exe`,便携包使用 `.zip`。Release 附带 `SHA256SUMS`;安装脚本保留在仓库 `script/install-condr.sh` / `script/install-condr.ps1`,不作为 Release 附件发布。

## 数据位置

Condr 按数据用途遵循 XDG 和各平台目录规范：

| 用途 | Linux | Windows | macOS |
| --- | --- | --- | --- |
| `config.toml` | `$XDG_CONFIG_HOME/condr`，默认 `~/.config/condr` | `%APPDATA%\condr` | `~/Library/Application Support/condr` |
| Managed Worktree | `$XDG_DATA_HOME/condr/worktrees`，默认 `~/.local/share/condr/worktrees` | `%LOCALAPPDATA%\condr\worktrees` | `~/Library/Application Support/condr/worktrees` |
| Snapshot | `$XDG_STATE_HOME/condr`，默认 `~/.local/state/condr` | `%LOCALAPPDATA%\condr` | `~/Library/Application Support/condr` |
| Log | `$XDG_STATE_HOME/condr`，默认 `~/.local/state/condr` | `%LOCALAPPDATA%\condr` | `~/Library/Logs/condr` |
| 本地 endpoint 与 GUI 实例锁 | `$XDG_RUNTIME_DIR/condr` | `%LOCALAPPDATA%\condr\runtime` | `$TMPDIR/condr` |

Linux 未提供 `XDG_RUNTIME_DIR` 时，本地 endpoint 回退到 data 目录下的 `runtime/`。`CONDR_SOCKET_PATH` 和 `CONDR_SNAPSHOT_PATH` 仍可覆盖 Server 的默认路径。仅解压普通归档不会修改 PATH；使用对应平台的安装包或安装脚本注册全局命令。

### 安装 CLI

仓库内的安装脚本只用于安装 headless CLI/server（同一个 `condr` 二进制）；GUI + CLI 使用对应平台的安装包。Unix 脚本把 CLI 放到用户目录并只追加自己的 PATH 标记；不会修改 Condr 数据目录：

```bash
sh script/install-condr.sh --from ./condr-cli-<version>-<short_sha>-linux-x86_64.tar.gz
```

安装后重新打开终端即可执行 `condr`。脚本也可以省略 `--from`，从已登录的 GitHub CLI 下载 `nightly` release；服务器安装应使用对应架构的归档。GUI 的 Linux AppImage 安装会把同一版本的 `condr` 提取到稳定用户目录，再注册 `~/.local/bin/condr`，不能直接链接到 AppImage 的临时挂载目录。

检测到已有 `condr` 时，脚本提示确认覆盖，默认取消；确认后只更新 CLI，保留已有 GUI 文件。自动化环境通过 Unix 的 `--yes` 或 PowerShell 的 `-Yes` 显式确认。两个脚本都支持 `CONDR_INSTALL_DIR` 覆盖安装目录；默认 Unix 使用 `~/.local/opt/condr`，Windows 与 GUI 共用 `%LOCALAPPDATA%\Programs\Condr`。安装不会自动启动 Server，需要时执行 `condr server start`。

### `config.toml` 可以手工编辑

`config.toml` 由 Client 和 Server 共享，允许手工编辑。GUI 写回时只改动它自己那个键，文件其余部分——注释、键顺序、空行和引号风格——原样保留，包括被改键上方和行尾的注释。

被改动的那个值本身会按标准格式重写（`appearance = 'dark'` 变成 `appearance = "dark"`）。Server 列表是从当前连接整体重新生成的，所以 `[[client.servers]]` 各条目内部的手写格式和注释不保留；其外的内容不受影响。

GUI、Server 和 CLI 共用配置事务：在解析后的真实文件旁取得 `config.toml.lock` 跨进程锁，锁覆盖读取、修改和写回，读取也使用同一把锁。已有符号链接保留，写入落到目标文件；通常原子替换，Windows 编辑器未共享删除权限时才在锁内原地写回。GUI 在事件循环启动前读取，后续保存按顺序放到后台，退出时等待未完成保存；Server 保存 Shell 时不会持有全局 Session 锁。手工编辑器不参与 Condr 的事务锁，仍应避免同时覆盖同一个设置。

### 终端 Shell

`[server.terminal] shell` 指定 Server 在新终端里启动的程序，例如 `"nu"`、`"zsh"` 或 `"C:\\Program Files\\Git\\bin\\bash.exe"`。留空或不写时使用系统默认：Unix 取 `$SHELL`（不可执行时回退 passwd 记录），Windows 依次找 PATH 上的 `pwsh.exe`、`powershell.exe`，最后 `%ComSpec%`。

这是每个 Server 各自的配置，存在该 Server 所在机器的 `config.toml` 里。Settings 的 Server 页先选 Server，再改 Shell；GUI 通过协议把值发给那台 Server，Server 写回自己的 `config.toml` 并广播新设置给所有 client。改动对下一个新终端生效，不必重启 Server；直接手改文件则需要重启 Server 才会读到。

cwd 上报只对已知 shell 注入：Linux 上的 bash（wrapper rcfile）和 Windows 上的 pwsh / powershell（prompt hook）。其他 shell 正常启动，Pane 的 cwd 来自进程探测或 shell 自己发送的 OSC 7；OSC 7 接受空主机名、localhost 和本机主机名，拒绝其他机器的路径。

### 两个二进制

- `condr`：Server 和命令行。`condr server start|restart|status|stop|run` 管理 Server；`condr workspace …`、`condr tab …`、`condr pane …` 从 Pane 内改 Session 结构、读终端文本（`pane read`，直接打纯文本）和向终端输入（`pane run` / `send-text` / `send-keys`）；`condr agent available|list|start|prompt|wait` 探测、启动和编排 agent。成功打 JSON 到 stdout，失败 `{"error":{"code","message"}}` 到 stderr 并退出 1，用法错误退出 2；`--help` 列出子命令。编排能力全部在 Server，所以 CLI 和 Server 是同一个二进制，无头 Linux 只需部署这一个文件。
- `condr-gui`：GUI，只是 Server 的一个 client。它发现已有的本地 Server，或者用同目录下的 `condr server run` 启动一个。

三个平台一致。Windows 上 `condr-gui.exe` 是 GUI 子系统程序，双击不出现控制台；`condr.exe` 是普通控制台程序。安装器和快捷方式负责把 GUI 以 Condr 的名字露给用户。

在 Condr 外部终端执行 `condr server restart`，先停止当前 Server，等待 PTY 关闭、快照写回和 endpoint 释放，再启动后台 Server；尚未运行时直接启动。支持 `start` 同款 `--listen`、`--snapshot`，未指定时读取配置和环境；此前通过命令行指定的自定义快照路径需再次提供，或设置 `CONDR_SNAPSHOT_PATH`。关闭超过 30 秒时报错，不启动替代进程。重启恢复布局和 cwd，启动新 shell，再用 hooks 已保存的原生会话 ID 重新打开 Agent 会话；不恢复旧进程或终端历史，详见 [ADR 0005](adr/0005-session-snapshots-restore-structure-only.md)。维护者于 2026-09-08 确认会话恢复已手工测试通过。在 Condr Pane 内执行会提前拒绝，因为关闭 Server 会终止发起重启的 CLI。

CLI 查询结构时不会请求全部终端画面；`pane read --lines N` 只读取目标 Pane 的文本。创建 Workspace、Tab 或拆分 Pane 时，Server 在同一次操作中应用名称与 `--focus`，并返回实际创建的 ID；不带 `--focus` 时保留当前选择。`pane send-keys` 支持 F1–F20，整组键在发送前校验，F21–F24 会直接拒绝。

### Pane 内的环境变量

Server 启动每个 Pane 的 shell 时注入 `CONDR_ENV=1`、`CONDR_PANE_ID=<id>`、`CONDR_SOCKET_PATH=<endpoint>`（本地 socket / named pipe 路径；Server 与 Pane 同机，即使 Server 也监听 TCP，Pane 内的 CLI 仍走本地 socket），并把 `condr` 所在目录前置到 `PATH`。另注入 `CONDR_BIN_PATH=<condr 的绝对路径>`：agent 执行命令时的登录 shell 可能重设 PATH，此时用 `"$CONDR_BIN_PATH"`（PowerShell：`& $env:CONDR_BIN_PATH`）调用同一二进制，包括 `--skill`，无需额外安装。此做法参考 Herdr `5158adab10b6dcfea9370782043392f80fa0643c` 的 `src/integration/env.rs`，该文件适用 Apache-2.0，无额外 notice。

Agent 的命令沙箱需要允许访问上述 socket。Linux 实测 Codex 0.153.4 的 `workspace-write` 默认网络限制会使连接失败，返回 `Operation not permitted`。本轮验收通过单次启动参数 `-s workspace-write -c sandbox_workspace_write.network_access=true` 允许连接，保留文件写入限制；该选项也会允许出站网络，应按所需权限配置，详见 [Codex 配置参考](https://learn.chatgpt.com/docs/config-file/config-reference)。Condr 不修改 agent 的沙箱策略或全局配置。

Windows 原生 Codex 0.153.4 的 `workspace-write` 在本机实测中无法访问 Condr 私有命名管道，返回 `not_authorized` / `Access denied (os error 5)`，上述 Linux 网络选项不能解决。Windows 独立临时 Server 验收使用单次启动参数 `--no-alt-screen -s danger-full-access -a never`；该模式取消 Codex 命令沙箱限制，不能当作默认配置，Condr 不会自动注入。权限含义见 [Codex 审批与安全](https://learn.chatgpt.com/docs/agent-approvals-security)。

### Agent 状态检测规则

Server 用内嵌的 TOML manifest（来自 herdr，`crates/condr-core/src/agent/manifests/`）判断 agent 的 idle / working / blocked。某个 agent 的规则不合适时，把修改后的 manifest 放到 `<config dir>/agent-detection/<id>.toml`（`id` 如 `claude`、`codex`），Server 启动时会用它替换内嵌版本；文件不合法时忽略并在 stderr 说明。

### Agent 探测与启动

`condr --skill` 原样输出内嵌的精简 [Condr skill](../skills/condr/SKILL.md)，无需 Server 或 Pane 环境，也不会连接或启动 Server。文档只维护 `skills/condr/SKILL.md` 一份，修改后重新构建二进制即可更新；具体参数以各命令的 `--help` 为准。

```bash
condr agent available
condr agent start worker --kind codex --pane 2 -- --model gpt-5
condr agent list
condr agent prompt worker "检查当前改动" --wait --timeout 120000
condr agent wait worker --until idle --until blocked --timeout 120000
```

`condr-core::agent_discovery` 独立于 Pane 的进程/状态检测，按 Server 当前 PATH 查找 10 种已知 agent 的原生 CLI，返回类型和可执行文件的绝对路径；`AgentKind::executable()` 统一处理 `cursor-agent`、`agy` 等命令名。Unix 检查文件执行位，Windows 查找 `.exe` / `.cmd` / `.bat` / `.ps1`；保留 symlink/shim 路径。每次查询重新扫描，不运行 CLI、不调用 `--version`、不安装 hook，也不搜索 PATH 外的安装目录或 shell alias。通过 TCP 连接时探测的仍是目标 Server。找到文件不代表已登录或 hook 已配置。Pi、OMP、Antigravity CLI、官方 Grok Build、Cursor CLI、GitHub Copilot CLI 已有 hook 安装入口；Kimi 目前仅识别身份，hooks 显示 unavailable，原生事件无法区分主任务和子 agent 的完成。版本、配置与测试范围见 [新增 Agent hooks 调查](research/additional-agent-hooks.md)。

`start` 只使用已有 Pane 的空闲 shell，保留 cwd 和环境，以探测到的绝对路径启动。支持 sh/bash/dash/zsh/ksh/mksh/fish 和 PowerShell；其它 shell 暂不支持自动启动。参数按实际 shell 引用，编码后的整条启动命令最多 4094 字节，拒绝控制字符；Windows batch shim 的参数另外拒绝 shell 元字符。命令和回车一次入队，不会拆成两次 client 请求。Shell 尚在初始化时 CLI 最多重试 2 秒，只重试 Server 确认未发送输入的请求。

`start` / `prompt` 的文本与 Enter 共用一个有界输入队列任务。Writer 写完文本后，Unix 等待 300 ms、Windows 等待 1 秒再发 Enter，避免原生 CLI 把 Enter 吸收为粘贴内容；期间其它输入不能插入这次提交，停止 Terminal 会取消尚未发送的 Enter。普通键盘输入不增加等待。延迟提交思路参考 Herdr `9a2a7af5402f2bc67ab24c8b4c14c6dd20a43bb2` 的 `src/app/api/agents.rs`（Apache-2.0，文件无额外 notice）；Windows 的较长间隔来自 Condr 的原生 Codex 验证。

名字遵循 `[a-z][a-z0-9_-]{0,31}`，Server 内唯一；启动中即保留，GUI/CLI 断开不会释放，进程退出、Terminal 替换或 Server 重启时释放，不进入 Session Snapshot。`prompt` / `wait` 的目标是名字或数字 Pane id，不把 agent 类型当名字解析。`prompt` 只接受 idle agent；`--wait` 必须观察到提交后的状态变化，不能用提交前的 idle 立即成功。

`start` 默认等待 30 秒，复用检测器的 3 秒启动宽限，确认预期类型的进程仍存活且进入 idle 才成功。Codex、Copilot 在第一轮才报启动，Cursor 恢复时不报启动，Antigravity 没有已确认的启动 hook；这些 CLI 识别为 Unknown 后也可返回，首次发送前先完成原生 CLI 的登录/初始化并确认输入框可用。blocked 返回 `agent_not_ready` 并保留名字；超时返回 `agent_timeout` 并释放尚在启动中的名字，进程继续留在 Pane 内供人工处理。启动超时范围为 `(3000, 300000]` 毫秒，普通等待范围为 `[0, 300000]`。等待登记在 Server，由检测事件和期限驱动；连接读取线程继续服务 Ping/Detach，断开取消等待，关闭 Pane 或停止 Server 会结束等待。GUI 的退出完成提示不算仍在运行的 idle agent。

参考 Herdr 本地 commit `5158adab10b6dcfea9370782043392f80fa0643c` 的 `src/detect/mod.rs`、`src/integration/registry.rs`、`src/app/agents.rs` 和 `src/platform/windows.rs`；逐文件未发现额外许可证或 notice，适用 Apache-2.0。此轮协议变更仍保持开发版本号 1，更新二进制后须重启旧 Server。

### GUI 是单实例

GUI 启动时在 runtime 目录取一把 `condr.lock` 独占文件锁；锁已被占用时第二个实例打印一行说明后直接退出，不会打开窗口。它保证每个用户只有一个 GUI 实例；配置文件另有跨进程事务锁。锁在进程退出时释放，包括崩溃。

Server 不受此限制 —— 每个 endpoint 本来就由 socket/named pipe 的绑定天然互斥。

## Windows

在 PowerShell 中下载并解压最新 nightly bundle：

```powershell
New-Item -ItemType Directory -Force .\condr-download | Out-Null
gh release download nightly --repo condrdev/condr --dir .\condr-download --pattern 'condr-[0-9]*-windows-x86_64.zip' --clobber
$archive = Get-ChildItem .\condr-download\*.zip | Where-Object { $_.Name -like 'condr-[0-9]*-windows-x86_64.zip' } | Select-Object -First 1
Expand-Archive -LiteralPath $archive.FullName -DestinationPath .\condr-dev -Force
```

运行 `condr-dev\condr\condr-gui.exe` 启动 GUI。`condr.exe` 必须保留在同一目录；GUI 会发现已有本地 Server，或者用它启动一个新的 Server。需要显式管理 Server 时，使用 `condr-dev\condr\condr.exe server start|restart|status|stop|run`。

CLI-only 安装可直接运行仓库内脚本（PowerShell 会写入当前用户 PATH）：

```powershell
powershell -ExecutionPolicy Bypass -File script\install-condr.ps1 -From .\condr-cli-<version>-<short_sha>-windows-x86_64.zip
```

GUI + CLI 的安装器工程在 `packaging/condr.iss`，通过 `powershell -File script\package-windows.ps1 -Commit <commit>` 构建。安装器与 GUI ZIP 共用打包输入目录，默认安装两个相邻的 EXE，CLI-only 类型只安装 `condr.exe`；两种类型均附带 `LICENSE` 和 `BUILD-COMMIT`。

`powershell -File script\check-windows-package.ps1` 检查 `dist` 中的 GUI ZIP、CLI ZIP 和 `.exe`，实际安装 GUI 包，核对安装目录中的 `BUILD-COMMIT` 和 `LICENSE`，并验证 CLI 安装、覆盖确认、PATH 注册和 GUI 文件保留，最后卸载测试安装并还原 PATH。此检查应在独立测试用户或 CI runner 下运行。

更新前先运行 `condr.exe server stop`，再将新版覆盖解压到同一个 `condr-dev`。运行数据位于平台目录，替换二进制不会影响它们。删除 bundle 只卸载程序；需要清空 Condr 时，再删除上表中对应平台的 config、data、state、log 和 runtime 目录。

## Linux Server

Linux artifacts are built on Ubuntu 22.04 and require glibc 2.35 or newer.

GUI + CLI 的 Linux 包使用 AppImage。构建机需要 `linuxdeploy` 和 `appimagetool`：

```bash
CONDR_COMMIT=<commit> script/package-linux.sh
```

AppImage 需要先赋予执行权限；它只是桌面分发包，CLI-only 环境仍使用上面的安装脚本。AppImage 运行时使用临时挂载目录，因此 PATH 入口必须指向安装后的稳定用户目录。

macOS GUI + CLI 使用拖放安装的 `.dmg`（内含 `Condr.app` 与 `/Applications` 链接），构建机需要 Xcode Command Line Tools：

```bash
CONDR_COMMIT=<commit> script/package-macos.sh
```

DMG 没有安装脚本，所以 `Condr.app` 的启动器 `Contents/MacOS/condr-launcher`（不叫 `Condr`，否则在默认不区分大小写的 APFS 上会和同目录的 `condr` CLI 冲突） 每次启动先运行 bundle 内的 `condr server install`（ADR 0016，幂等）：把 CLI 复制到 `~/.local/opt/condr`，建立 `~/.local/bin/condr` 链接，并在 `~/.zprofile` 追加一次带标记的 PATH 行；随后 `exec` 成 `condr-gui`，并让 GUI 用安装后的稳定副本启动 Server，与 Linux AppImage 的 `AppRun` 一致。安装失败时 GUI 仍照常启动。重新打开 zsh 终端后即可运行 `condr --help`；bash 登录 shell 不注册，需要时手工把 `~/.local/bin` 加入 PATH。

`sh script/check-macos-package.sh <dmg> <cli tar.gz>` 只在 macOS 上运行：挂载镜像，核对 `/Applications` 链接、提交号、图标、启动器语法和二进制架构与 runner 一致，再把 app 复制到 `~/Applications`，连续执行两次 `condr server install` 验证 `~/.zprofile` 不重复追加，并在全新 zsh 登录 shell 中执行 `condr server --help`。

Linux/macOS 原生检查共用 `script/check-cli-package.sh`：归档必须只包含对应顶层目录、可执行 `condr`、`LICENSE` 和 `BUILD-COMMIT`；随后实际运行 CLI，验证安装、覆盖确认和已有 GUI 文件保留。提交号默认比对 `GITHUB_SHA`（本地为当前 HEAD），检查旧产物时用 `CONDR_COMMIT` 显式指定期望 SHA。

根据机器架构下载一个 Server artifact：

```bash
arch=$(uname -m)
case "$arch" in aarch64) arch=arm64 ;; esac
mkdir -p condr-download
gh release download nightly --repo condrdev/condr --dir condr-download \
  --pattern "condr-cli-*-linux-${arch}.tar.gz" --clobber
archive=$(find "$PWD/condr-download" -name "condr-cli-*-linux-${arch}.tar.gz" -print -quit)
tar -C condr-download -xzf "$archive"
```

远端需预装与 Client 同一构建的 `condr`，并提前启动 Server：

```bash
condr server start
```

### SSH 连接

先在 Client 机器的终端完成一次 `ssh <host>` 登录，确认主机密钥和密钥认证可用；GUI 使用系统 OpenSSH 与 ssh-agent，不弹出密码或主机信任提示。端口、IdentityFile、ProxyJump 等可以放在 SSH config 的 Host 条目里。Condr 会禁用该条目继承的端口转发，并覆盖 RemoteCommand、SessionType、StdinNull、ForkAfterAuthentication，保证 bridge 在前台使用协议 stdin/stdout。

在 Connect Remote Device 的 Address 中填写：

```text
ssh://user@host
ssh://build-box
ssh://user@host:2222?bin=/opt/example/condr
```

未指定 `bin` 时，远端非交互 shell 的 PATH 必须能找到 `condr`。`bin` 是绝对可执行文件路径；空格等字符可使用 URI 百分号编码，例如 `?bin=/opt/Condr%20Dev/condr`，`+` 保留为加号。只支持 `bin` 这一个 query 参数，路径中的 `&`、`#`、`%` 应分别编码为 `%26`、`%23`、`%25`。Server 必须用同一远端用户启动；非默认 socket 可由远端 `CONDR_SOCKET_PATH` 指定。

Client 执行 `ssh -T … 'condr server bridge'`（指定 `bin` 时替换程序路径）。bridge 仅双向转接远端私有 `.sock`，不需要 TCP listener、invite 或 Noise 配对；它不创建 Server、不安装或更新程序。SSH Client 的权限等同于远端登录用户，TCP 的 `server clients|revoke` 不管理 SSH 密钥。GUI 断开后远端 Server、Session 和终端继续运行，重连获取当前 Bootstrap。

SSH/Welcome 阶段允许 15 秒无数据；Welcome 后 Bootstrap 的超时按 4 秒无数据计算，持续传输不会因总耗时过长中断。Disconnect 可取消连接中的握手和堵塞的发送。

首版面向 Windows/Linux Client 连接 Linux/macOS Server（远端使用 POSIX shell）；维护者于 2026-09-08 确认 SSH 连接已手工测试通过，见 [M1 完成记录](roadmap.md#m1solo-daily-driver--desktop-daily-driver)。远端 Server 未运行时由 bridge 自动启动；远端自动安装将在后续 install 安装流程中实现。

### TCP 连接

远端额外监听 TCP 时，使用：

```bash
condr server start --listen 0.0.0.0:4242
condr server invite
```

已启动的 Server 需要 `condr server restart` 才会应用新的 listen 地址。将 invite 打印的 `tcp://<server key>.<invite>@<host>:4242` 中 `<host>` 换成远端可达地址，粘贴到 Connect Remote Device。TCP 每次连接仍使用 Noise 双向认证与加密；首次 invite 配对成功后，重连只使用设备密钥。`condr server clients` 列出配对设备，`condr server revoke <key 前缀>` 撤销并断开设备。

### 保存的 Server

Client 将远端连接保存到 `config.toml`，地址的 scheme 区分类型：

```toml
[[client.servers]]
name = "Build"
address = "ssh://user@host?bin=/opt/example/condr"

[[client.servers]]
name = "TCP Server"
address = "tcp://<server-public-key>@host:4242"
```

TCP 地址只保存公钥，不保存 invite。开发版不迁移旧的 `address = "host:port"` + `server_key` 配置；将其改成上面的单个 `tcp://<server-public-key>@host:port` 地址即可。

Server 列表加载失败时会保留原文件，并禁止添加、编辑、删除或写回列表；先修正错误再重启 GUI。其他外观和终端偏好仍可保存。

停止远端 Server 是显式操作，在远端终端执行 `condr server stop`。

## 更新检查

应用图标统一来自 `packaging/icons/condr.svg`。修改后运行
`cargo run --locked -p condr-gui --example generate_icons` 并提交生成资源；
各平台打包方式见 [应用图标说明](../packaging/icons/README.md)。

解压后核对三个位置中的 commit SHA：release notes、artifact 文件名和包内 `BUILD-COMMIT`。更新滚动 release 后同时替换 Windows bundle 与 Linux Server，不要混用两个 SHA。
