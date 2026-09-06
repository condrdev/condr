# Development Build

Condr 在私有 GitHub 仓库中维护一个滚动的 `Development Build` Pre-release。它只供当前开发者自举使用，不是版本化发布。

## 发布模型

- `dev` 是可变 tag，指向当前选定的 `main` commit。
- 普通 commit 和 push 不发布；只有移动并推送 `dev` tag 才触发 GitHub Actions。
- 每次发布同时生成 Windows x86_64 bundle（`condr-gui` + `condr`）、Linux x86_64 和 Linux aarch64 的 `condr`（Server + CLI）。
- 三个 artifacts 必须来自同一 commit。文件名、release notes 和包内 `BUILD-COMMIT` 都记录该 SHA。
- Client 和 Server 没有跨开发版本兼容承诺，必须一起更新。

## 数据位置

Condr 按数据用途遵循 XDG 和各平台目录规范：

| 用途 | Linux | Windows | macOS |
| --- | --- | --- | --- |
| `config.toml` | `$XDG_CONFIG_HOME/condr`，默认 `~/.config/condr` | `%APPDATA%\condr` | `~/Library/Application Support/condr` |
| Managed Worktree | `$XDG_DATA_HOME/condr/worktrees`，默认 `~/.local/share/condr/worktrees` | `%LOCALAPPDATA%\condr\worktrees` | `~/Library/Application Support/condr/worktrees` |
| Snapshot | `$XDG_STATE_HOME/condr`，默认 `~/.local/state/condr` | `%LOCALAPPDATA%\condr` | `~/Library/Application Support/condr` |
| Log | `$XDG_STATE_HOME/condr`，默认 `~/.local/state/condr` | `%LOCALAPPDATA%\condr` | `~/Library/Logs/condr` |
| 本地 endpoint 与 GUI 实例锁 | `$XDG_RUNTIME_DIR/condr` | `%LOCALAPPDATA%\condr\runtime` | `$TMPDIR/condr` |

Linux 未提供 `XDG_RUNTIME_DIR` 时，本地 endpoint 回退到 data 目录下的 `runtime/`。`CONDR_SOCKET_PATH` 和 `CONDR_SNAPSHOT_PATH` 仍可覆盖 Server 的默认路径。Development Build 是普通归档，不会修改 PATH；全局命令注册需要单独的显式安装步骤。

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

在 Condr 外部终端执行 `condr server restart`，先停止当前 Server，等待 PTY 关闭、快照写回和 endpoint 释放，再启动后台 Server；尚未运行时直接启动。支持 `start` 同款 `--listen`、`--snapshot`，未指定时读取配置和环境；此前通过命令行指定的自定义快照路径需再次提供，或设置 `CONDR_SNAPSHOT_PATH`。关闭超过 30 秒时报错，不启动替代进程。重启恢复布局和 cwd，agent 进程不会恢复。在 Condr Pane 内执行会提前拒绝，因为关闭 Server 会终止发起重启的 CLI。

CLI 查询结构时不会请求全部终端画面；`pane read --lines N` 只读取目标 Pane 的文本。创建 Workspace、Tab 或拆分 Pane 时，Server 在同一次操作中应用名称与 `--focus`，并返回实际创建的 ID；不带 `--focus` 时保留当前选择。`pane send-keys` 支持 F1–F20，整组键在发送前校验，F21–F24 会直接拒绝。

### Pane 内的环境变量

Server 启动每个 Pane 的 shell 时注入 `CONDR_ENV=1`、`CONDR_PANE_ID=<id>`、`CONDR_SOCKET_PATH=<endpoint>`（本地 socket / named pipe 路径；Server 与 Pane 同机，即使 Server 也监听 TCP，Pane 内的 CLI 仍走本地 socket），并把 `condr` 所在目录前置到 `PATH`。Pane 内的程序（后续的 `condr` CLI、agent hook）靠这三个变量找到自己的 Server 和 Pane；便携版和 `cargo run` 下不需要安装步骤就能在 Pane 内直接执行 `condr`。

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

`condr-core::agent_discovery` 独立于 Pane 的进程/状态检测，按 Server 当前 PATH 查找 23 种已知 agent 的原生 CLI，返回类型和可执行文件的绝对路径；`AgentKind::executable()` 统一处理 `cursor-agent`、`kiro-cli` 等命令名。Unix 检查文件执行位，Windows 查找 `.exe` / `.cmd` / `.bat` / `.ps1`；保留 symlink/shim 路径。每次查询重新扫描，不运行 CLI、不调用 `--version`、不安装 hook，也不搜索 PATH 外的安装目录或 shell alias。通过 TCP 连接时探测的仍是目标 Server。后续 hook 集成可直接复用此模块；找到文件不代表已登录或 hook 已配置。

`start` 只使用已有 Pane 的空闲 shell，保留 cwd 和环境，以探测到的绝对路径启动。支持 sh/bash/dash/zsh/ksh/mksh/fish 和 PowerShell；其它 shell 暂不支持自动启动。参数按实际 shell 引用，编码后的整条启动命令最多 4094 字节，拒绝控制字符；Windows batch shim 的参数另外拒绝 shell 元字符。命令和回车一次入队，不会拆成两次 client 请求。Shell 尚在初始化时 CLI 最多重试 2 秒，只重试 Server 确认未发送输入的请求。

名字遵循 `[a-z][a-z0-9_-]{0,31}`，Server 内唯一；启动中即保留，GUI/CLI 断开不会释放，进程退出、Terminal 替换或 Server 重启时释放，不进入 Session Snapshot。`prompt` / `wait` 的目标是名字或数字 Pane id，不把 agent 类型当名字解析。`prompt` 只接受 idle agent；`--wait` 必须观察到提交后的状态变化，不能用提交前的 idle 立即成功。

`start` 默认等待 30 秒，复用检测器的 3 秒启动宽限，确认预期类型的进程仍存活且进入 idle 才成功。blocked 返回 `agent_not_ready` 并保留名字；超时返回 `agent_timeout` 并释放尚在启动中的名字，进程继续留在 Pane 内供人工处理。启动超时范围为 `(3000, 300000]` 毫秒，普通等待范围为 `[0, 300000]`。等待登记在 Server，由检测事件和期限驱动；连接读取线程继续服务 Ping/Detach，断开取消等待，关闭 Pane 或停止 Server 会结束等待。GUI 的退出完成提示不算仍在运行的 idle agent。

参考 Herdr 本地 commit `5158adab10b6dcfea9370782043392f80fa0643c` 的 `src/detect/mod.rs`、`src/integration/registry.rs`、`src/app/agents.rs` 和 `src/platform/windows.rs`；逐文件未发现额外许可证或 notice，适用 Apache-2.0。此轮协议变更仍保持开发版本号 1，更新二进制后须重启旧 Server。

### GUI 是单实例

GUI 启动时在 runtime 目录取一把 `condr.lock` 独占文件锁；锁已被占用时第二个实例打印一行说明后直接退出，不会打开窗口。它保证每个用户只有一个 GUI 实例；配置文件另有跨进程事务锁。锁在进程退出时释放，包括崩溃。

Server 不受此限制 —— 每个 endpoint 本来就由 socket/named pipe 的绑定天然互斥。

发布一个已经测试并推送到 `main` 的 commit：

```bash
git tag -f dev <commit>
git push origin refs/tags/dev --force
gh run list --workflow development-build.yml --limit 1
```

`dev` 是唯一允许 force push 的 tag。正式版本 tag 必须保持不可变。

## Windows

在已登录私有仓库的 PowerShell 中下载并解压最新 bundle：

```powershell
New-Item -ItemType Directory -Force .\condr-download | Out-Null
gh release download dev --repo condrdev/condr --dir .\condr-download --pattern 'condr-windows-x86_64-*.zip' --clobber
$archive = Get-ChildItem .\condr-download\condr-windows-x86_64-*.zip | Select-Object -First 1
Expand-Archive -LiteralPath $archive.FullName -DestinationPath .\condr-dev -Force
```

运行 `condr-dev\condr\condr-gui.exe` 启动 GUI。`condr.exe` 必须保留在同一目录；GUI 会发现已有本地 Server，或者用它启动一个新的 Server。需要显式管理 Server 时，使用 `condr-dev\condr\condr.exe server start|restart|status|stop|run`。

更新前先运行 `condr.exe server stop`，再将新版覆盖解压到同一个 `condr-dev`。运行数据位于平台目录，替换二进制不会影响它们。删除 bundle 只卸载程序；需要清空 Condr 时，再删除上表中对应平台的 config、data、state、log 和 runtime 目录。

## Linux Server

Linux artifacts are built on Ubuntu 22.04 and require glibc 2.35 or newer.

根据机器架构下载一个 Server artifact：

```bash
arch=$(uname -m)
mkdir -p condr-download
gh release download dev --repo condrdev/condr --dir condr-download \
  --pattern "condr-linux-${arch}-*.tar.gz" --clobber
archive=$(find "$PWD/condr-download" -name "condr-linux-${arch}-*.tar.gz" -print -quit)
tar -C condr-download -xzf "$archive"
```

后台启动远端 Server 并让它额外监听一个 TCP 地址。一台机器只有一个 Server，它始终在本地 socket 上服务本机 GUI 与 CLI；`--listen` 把地址写进 Server 的 `config.toml`，之后不带参数的 `start` 也会按它监听，其他子命令都不需要再指定。每个 TCP 连接都经过 WireGuard 式的双向密钥认证与加密，因此地址可以是 loopback 配合 SSH tunnel，也可以直接是局域网地址。首次启动会在配置目录生成 `server-key`；默认 snapshot 和 log 会写入 XDG state 目录：

```bash
./condr-download/condr/condr server start --listen 127.0.0.1:4242
```

调试或交给外部服务管理器时，使用 `server run` 在前台运行，它同样读取 `config.toml` 里的监听地址。

在 Windows 建立 SSH tunnel（直接监听局域网地址时可省略）：

```powershell
ssh -N -L 4242:127.0.0.1:4242 <linux-host>
```

在 Linux 上为这台 Windows 设备生成一次性 invite，它 10 分钟内有效，只能使用一次：

```bash
./condr-download/condr/condr server invite
```

把它打印的 `<server key>.<invite>@<host>:4242` 中的 `<host>` 换成 `127.0.0.1`，粘贴到 Condr 的 Add Server 对话框。第一次连接成功后，设备的公钥就记录在 Server 的 `authorized-clients` 里，之后重连不再需要 invite。`condr server clients` 列出已配对设备，`condr server revoke <key 前缀>` 撤销一台设备：先改写名单文件，再连上运行中的 Server 断开该设备的存活连接。停止 Server 时，在 Linux 的另一个 shell 运行：

```bash
./condr-download/condr/condr server stop
```

## 更新检查

解压后核对三个位置中的 commit SHA：release notes、artifact 文件名和包内 `BUILD-COMMIT`。更新滚动 release 后同时替换 Windows bundle 与 Linux Server，不要混用两个 SHA。
