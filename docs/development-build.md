# Development Build

Condr 在私有 GitHub 仓库中维护一个滚动的 `Development Build` Pre-release。它只供当前开发者自举使用，不是版本化发布。

## 发布模型

- `dev` 是可变 tag，指向当前选定的 `main` commit。
- 普通 commit 和 push 不发布；只有移动并推送 `dev` tag 才触发 GitHub Actions。
- 每次发布同时生成 Windows x86_64 GUI/Server bundle、Linux x86_64 Server 和 Linux aarch64 Server。
- 三个 artifacts 必须来自同一 commit。文件名、release notes 和包内 `BUILD-COMMIT` 都记录该 SHA。
- Client 和 Server 没有跨开发版本兼容承诺，必须一起更新。

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

进入解压出的 commit 目录并运行 `condr-gui.exe`。`condr-server.exe` 必须保留在同一目录；GUI 会发现已有本地 Server，或者从该目录启动一个新的 Server。

## Linux Server

根据机器架构下载一个 Server artifact：

```bash
arch=$(uname -m)
mkdir -p condr-download
gh release download dev --repo condrdev/condr --dir condr-download \
  --pattern "condr-server-linux-${arch}-*.tar.gz" --clobber
archive=$(find "$PWD/condr-download" -name "condr-server-linux-${arch}-*.tar.gz" -print -quit)
tar -C condr-download -xzf "$archive"
```

使用 loopback listener 启动远端 Server，避免暴露未鉴权端口：

```bash
mkdir -p "$HOME/.config/condr"
./condr-download/condr-server-linux-${arch}-*/condr-server \
  --listen 127.0.0.1:4242 \
  --snapshot "$HOME/.config/condr/remote.snapshot"
```

在 Windows 建立 SSH tunnel：

```powershell
ssh -N -L 4242:127.0.0.1:4242 <linux-host>
```

然后在 Condr 中添加 TCP Server `127.0.0.1:4242`。停止 Server 时，在 Linux 的另一个 shell 运行：

```bash
./condr-download/condr-server-linux-${arch}-*/condr-server --listen 127.0.0.1:4242 --stop
```

## 更新检查

解压后核对三个位置中的 commit SHA：release notes、artifact 文件名和包内 `BUILD-COMMIT`。更新滚动 release 后同时替换 Windows bundle 与 Linux Server，不要混用两个 SHA。
