# Development Build

Condr 在私有 GitHub 仓库中维护一个滚动的 `Development Build` Pre-release。它只供当前开发者自举使用，不是版本化发布。

## 发布模型

- `dev` 是可变 tag，指向当前选定的 `main` commit。
- 普通 commit 和 push 不发布；只有移动并推送 `dev` tag 才触发 GitHub Actions。
- 每次发布同时生成 Windows x86_64 GUI/Server bundle、Linux x86_64 Server 和 Linux aarch64 Server。
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

### GUI 是单实例

GUI 启动时在 runtime 目录取一把 `condr-gui.lock` 独占文件锁；锁已被占用时第二个实例打印一行说明后直接退出，不会打开窗口。这条约束的存在理由是 Client 独占 `config.toml` 的读-改-写：两个 GUI 并发保存会互相丢键。锁在进程退出时释放，包括崩溃。

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

运行 `condr-dev\condr\condr.exe`。它只启动 GUI，不承载命令行接口。`condr-server.exe` 必须保留在同一目录；GUI 会发现已有本地 Server，或者从该目录启动一个新的 Server。需要显式管理 Server 时，使用同目录下的 `condr-server.exe start|status|stop|run`。

更新前先运行 `condr-server.exe stop`，再将新版覆盖解压到同一个 `condr-dev`。运行数据位于平台目录，替换二进制不会影响它们。删除 bundle 只卸载程序；需要清空 Condr 时，再删除上表中对应平台的 config、data、state、log 和 runtime 目录。

## Linux Server

Linux artifacts are built on Ubuntu 22.04 and require glibc 2.35 or newer.

根据机器架构下载一个 Server artifact：

```bash
arch=$(uname -m)
mkdir -p condr-download
gh release download dev --repo condrdev/condr --dir condr-download \
  --pattern "condr-server-linux-${arch}-*.tar.gz" --clobber
archive=$(find "$PWD/condr-download" -name "condr-server-linux-${arch}-*.tar.gz" -print -quit)
tar -C condr-download -xzf "$archive"
```

使用 loopback listener 后台启动远端 Server，避免暴露未鉴权端口。默认 snapshot 和 log 会写入 XDG state 目录：

```bash
./condr-download/condr/condr-server start --listen 127.0.0.1:4242
```

调试或交给外部服务管理器时，使用 `run --listen 127.0.0.1:4242` 在前台运行。

在 Windows 建立 SSH tunnel：

```powershell
ssh -N -L 4242:127.0.0.1:4242 <linux-host>
```

然后在 Condr 中添加 TCP Server `127.0.0.1:4242`。停止 Server 时，在 Linux 的另一个 shell 运行：

```bash
./condr-download/condr/condr-server stop --listen 127.0.0.1:4242
```

## 更新检查

解压后核对三个位置中的 commit SHA：release notes、artifact 文件名和包内 `BUILD-COMMIT`。更新滚动 release 后同时替换 Windows bundle 与 Linux Server，不要混用两个 SHA。
