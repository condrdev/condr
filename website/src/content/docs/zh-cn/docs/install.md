---
title: 安装
description: 安装桌面应用或 Headless Server，并了解升级和卸载方法。
---

为每台机器选对安装包，完成安装，并按需要升级或卸载 Condr。

:::caution
Condr 0.1 是公开预览版，版本之间仍可能有变化。窗口和 Server 请一起更新。
:::

## 选择安装内容

| 这台机器 | 装什么 | 包含 |
| --- | --- | --- |
| 你工作用的电脑 | 桌面应用 | 窗口和 Server |
| 只跑 Agent、不需要窗口的机器 | Headless Server | 只有 `condr` 命令 |

桌面应用已经包含 Server，本机使用不需要再装 Headless Server。要从别的电脑连过来，把这台电脑当作远程 Device。见[连接远程设备](/zh-cn/docs/remote/)。

## 安装桌面应用

从[下载页](/download/)下载对应平台的安装包：

| 平台 | 安装包 |
| --- | --- |
| Linux x86_64 / arm64 | AppImage |
| macOS x86_64 / arm64 | `.dmg` |
| Windows x86_64 | `.exe` |

Windows 安装器装到 `%LOCALAPPDATA%\Programs\Condr`，不需要管理员权限，需要 Windows 10 1809 或更新。macOS 应用和 Linux AppImage 第一次启动时把 `condr` 命令复制到 `~/.local/opt/condr`，窗口用这份启动 Server。

### macOS 提示无法验证

应用还没有经过 Apple 公证，首次启动会提示无法验证。你可以这样打开：

- 点 **完成**，打开 **系统设置 › 隐私与安全性**，滚到底部，点 **仍要打开**。
- 或者在终端里清除一次下载标记：

  ```sh
  xattr -cr /Applications/Condr.app
  ```

### Windows 显示 SmartScreen

预览期的安装包没有签名。SmartScreen 拦截时，点 **更多信息**，再点 **仍要运行**。

### Linux AppImage 打不开

AppImage 需要 FUSE。安装发行版的 `libfuse2` 包，或者用 `--appimage-extract` 解开后运行。

## 安装 Headless Server

在要运行 Agent 的机器上，运行对应平台的命令：

| 平台 | 安装命令 |
| --- | --- |
| Linux x86_64 / arm64 | `curl -fsSL https://condr.dev/install.sh \| sh` |
| macOS x86_64 / arm64 | `curl -fsSL https://condr.dev/install.sh \| sh` |
| Windows x86_64 | `irm https://condr.dev/install.ps1 \| iex` |

脚本下载 headless 版本，用 `SHA256SUMS` 校验，运行 `condr server install`，再把 `condr` 加进 PATH。命令装到 `~/.local/opt/condr`，Windows 上装到 `%LOCALAPPDATA%\Programs\Condr`。

脚本把 PATH 加到 macOS 的 `~/.zprofile` 或 Linux 的 `~/.profile`。Windows 写进用户的环境变量。要确认安装成功，打开新终端运行：

```sh
condr --version
```

SSH 的非交互 shell 通常不读这些文件。从别的机器用 SSH 连接时报 `condr: not found`，在链接里指明路径。见[连接远程设备](/zh-cn/docs/remote/#用-ssh-连接)。

### 选择安装脚本选项

已经是最新版本时，再次运行脚本不做任何事。用下面的命令安装指定版本、安装后启动 Server，或强制重装：

```sh
CONDR_VERSION=nightly curl -fsSL https://condr.dev/install.sh | sh   # nightly（默认是最新 release）
CONDR_VERSION=v0.1.0 curl -fsSL https://condr.dev/install.sh | sh    # 某个版本
curl -fsSL https://condr.dev/install.sh | sh -s -- --start            # 同时启动 Server
curl -fsSL https://condr.dev/install.sh | sh -s -- --force            # 已是最新也重装
```

```powershell
$env:CONDR_VERSION = 'nightly'; irm https://condr.dev/install.ps1 | iex
$env:CONDR_VERSION = 'v0.1.0'; irm https://condr.dev/install.ps1 | iex
$env:CONDR_INSTALL_ARGS = '--start'; irm https://condr.dev/install.ps1 | iex
$env:CONDR_INSTALL_ARGS = '--force'; irm https://condr.dev/install.ps1 | iex
```

要装到其他位置，设置 `CONDR_INSTALL_DIR`。

## 选择 Release 或 Nightly

| 渠道 | 是什么 | 适合谁 |
| --- | --- | --- |
| Release | 带版本号的正式构建 | 大多数人 |
| Nightly | 每天从 `main` 构建一次 | 想用最新改动、能接受偶尔出错 |

窗口按你安装的渠道检查更新，启动 5 秒后一次，之后每 5 小时一次。有新版本时 **Settings › About** 显示提示，侧栏设置图标带一个点。Condr 只提示，不自动安装。渠道可以在 **Settings › About** 里改。

## 同时升级两部分

窗口和 Server 一起更新。协议兼容时，不同版本也能连接，侧栏只显示黄色三角。协议不兼容时拒绝连接。

- **桌面应用**：下载新安装包覆盖安装。窗口重新启动后，本机 Server 还是旧版本，运行 `condr server restart`，或在 **Settings › Device › Daemon** 里点 **Restart Condr**，让新版本接管。
- **Headless Server**：再次运行安装脚本。Server 正在运行时，脚本会问要不要重启。不重启时，连着它的窗口会显示版本不同，直到你重启。

重启 Server 会结束所有 Pane 里的程序，然后按快照恢复结构。装了 hook 的 Agent 会自动恢复会话。详见[Workspace、Tab 与 Pane](/zh-cn/docs/workspaces/#server-重启后剩下什么)。

## 卸载 Condr

Headless Server：

```sh
condr server uninstall
```

这会停止 Server，删除安装的 `condr`、符号链接和 PATH 项，保留配置、状态和日志目录，并告诉你位置。要连这些目录也删掉，见[配置与设置](/zh-cn/docs/configuration/#文件在哪)。

桌面应用：macOS 把 `Condr.app` 拖到废纸篓，Windows 在 **应用和功能** 中卸载，Linux 删除 AppImage。要删除复制到 `~/.local/opt/condr` 的 `condr` 命令，运行上面的 `uninstall`。
