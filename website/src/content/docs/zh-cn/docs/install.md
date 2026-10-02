---
title: 安装
description: 安装桌面应用或 Headless Server，启动 Server，并了解升级和卸载方法。
---

Condr 目前提供桌面应用和 Headless 两种安装方式：

| 场景 | 安装 | 包含 |
| --- | --- | --- |
| 日常工作 | 桌面应用 | GUI 和 Server |
| 服务器、开发机和其他不需要窗口的机器 | Headless Server | Server |

:::caution
Condr 0.1 是公开预览版，版本之间仍可能有变化。
:::

## 桌面应用（推荐）

从[下载页](/download/)下载对应平台的安装包，打开即可：

| 平台 | 安装包 |
| --- | --- |
| macOS x86_64 / arm64 | `.dmg` |
| Windows x86_64 | `.exe` |
| Linux x86_64 / arm64 | AppImage |

桌面应用自带 Server 并自动启动，无须单独安装 Headless Server。

要从别的电脑连接这台电脑，见[连接远程 Device](/zh-cn/docs/remote/)。

### Windows

安装包还没有签名。遇到 SmartScreen 拦截时，点 **更多信息**，再点 **仍要运行**。

### Linux

给下载的 AppImage 加上执行权限，再打开它。把文件名换成你下载的那个：

```sh
chmod +x condr-0.1.2-linux-x86_64.AppImage
./condr-0.1.2-linux-x86_64.AppImage
```

AppImage 需要系统支持 FUSE。如果它报 FUSE 错误，加上 `--appimage-extract-and-run`，不经过 FUSE 运行：

```sh
./condr-0.1.2-linux-x86_64.AppImage --appimage-extract-and-run
```

## Headless Server

在服务器、开发机和其他不需要窗口的机器上，只需要安装 `condr` 命令。

### 安装

Linux 或 macOS：

```sh
curl -fsSL https://condr.dev/install.sh | sh
```

Windows：

```powershell
irm https://condr.dev/install.ps1 | iex
```

脚本会下载 Headless 版本并用 `SHA256SUMS` 校验，然后把 `condr` 装到 `~/.local/opt/condr`，Windows 上装到 `%LOCALAPPDATA%\Programs\Condr`。

它还会把这个目录加进 PATH：macOS 写入 `~/.zprofile`，Linux 写入 `~/.profile`，Windows 写入用户环境变量。

打开一个新的终端，确认安装成功：

```sh
condr --version
```

### 启动 Server

```sh
condr server start
```

Server 在后台运行，关掉终端也不会停止。目前它不会开机自启，所以机器重启后需要手动启动。

用 SSH 连接时，Condr 会自动启动 Server。要从你的电脑连接这台机器，见[连接远程 Device](/zh-cn/docs/remote/)。

### 选择安装脚本选项

已经是最新版本时，再次运行脚本不会做任何事。用下面的写法安装指定版本或强制重装。

Linux 或 macOS：

```sh
CONDR_VERSION=nightly curl -fsSL https://condr.dev/install.sh | sh   # nightly（默认使用 release）
CONDR_VERSION=v0.1.0 curl -fsSL https://condr.dev/install.sh | sh    # 某个版本
curl -fsSL https://condr.dev/install.sh | sh -s -- --force            # 强制重装
```

Windows：

```powershell
$env:CONDR_VERSION = 'nightly'; irm https://condr.dev/install.ps1 | iex
$env:CONDR_VERSION = 'v0.1.0'; irm https://condr.dev/install.ps1 | iex
$env:CONDR_INSTALL_ARGS = '--force'; irm https://condr.dev/install.ps1 | iex
```

## 选择 Release 或 Nightly

| 渠道 | 是什么 | 适合谁 |
| --- | --- | --- |
| Release | 带版本号的正式构建 | 大多数人 |
| Nightly | 每天从 `main` 构建一次 | 想用最新改动、能接受偶尔出错 |

Condr 会按照安装的渠道，每 5 小时一次自动检查更新。有新版本时，会在 **Settings › Application › About** 显示提示。渠道可以在 **Settings › Application › About** 里修改。

## 升级

窗口和 Server 请一起升级。

- **桌面应用**：下载新安装包覆盖安装，再打开 Condr。Windows 安装器会先停止旧的 Server，窗口打开时直接启动新版本。macOS 和 Linux 上旧的 Server 会继续运行，窗口打开时会询问是否重启它。选 **Later** 的话，之后可以在 **Settings › Device › Daemon** 里点 **Restart Condr**。
- **Headless Server**：再次运行安装脚本。如果 Server 正在运行，脚本会询问是否需要重启。不重启的话，连着它的窗口会一直显示版本不同，直到你重启。

重启 Server 会结束所有 Pane 里的程序，然后按快照恢复结构。装了 hook 的 Agent 会自动恢复会话。详见 [Workspace、Tab 与 Pane](/zh-cn/docs/workspaces/#server-重启后剩下什么)。

## 卸载

- **Headless Server**：运行下面的命令。它会停止 Server，删除安装的 `condr` 和 PATH 项，但保留配置、状态和日志目录，并告诉你它们在哪。要把这些目录也删掉，见[配置与设置](/zh-cn/docs/configuration/#文件在哪)。

  ```sh
  condr server uninstall
  ```

- **桌面应用**：macOS 把 `Condr.app` 拖到废纸篓，Windows 在 **应用和功能** 中卸载，Linux 删除 AppImage。macOS 和 Linux 会留下复制到 `~/.local/opt/condr` 的 `condr` 命令，运行上面的 `condr server uninstall` 删除它。
