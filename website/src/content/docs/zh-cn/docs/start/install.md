---
title: 安装
description: 安装 Condr 桌面端或 Headless 服务端，掌握启动、多版本管理、平滑升级与卸载方法。
---

Condr 提供两种部署形态：

| 形态 | 适用场景 | 包含组件 |
| :--- | :--- | :--- |
| **桌面应用**（推荐） | 本地日常开发与交互 | GUI 界面 + 内置 Server |
| **Headless Server** | 远程服务器、云主机、无 GUI 容器/开发机 | 仅 `condr` CLI 与 Server |

:::caution
Condr 0.1 目前处于公开预览（Public Preview）阶段，功能与配置项可能在后续版本中持续调整。
:::

---

## 桌面应用

桌面端内置 Server 并在打开时自动拉起，无需单独配置后台服务。若需远程连接该设备，请参阅[远程连接](/zh-cn/docs/using/remote/)。

从[下载中心](/download/)获取对应平台的安装包：

| 平台 | 架构 | 安装包类型 |
| :--- | :--- | :--- |
| macOS | x86_64 / Apple Silicon | `.dmg` |
| Windows | x86_64 | `.exe` |
| Linux | x86_64 / arm64 | `.AppImage` |

### 平台注意事项

**Windows**
当前安装包尚未完成代码签名。如遇 SmartScreen 弹窗拦截，点击 **“更多信息” → “仍要运行”** 即可。

**Linux**
下载后赋予 AppImage 可执行权限并启动：

```sh
chmod +x condr-*-linux-*.AppImage
./condr-*-linux-*.AppImage
```

> **提示**：若运行环境缺少 FUSE 支持，可添加参数以解包模式直接运行：
> ```sh
> ./condr-*-linux-*.AppImage --appimage-extract-and-run
> ```

---

## Headless Server

在无图形界面的环境仅需安装 `condr` CLI 工具。

### 1. 一键安装

**Linux / macOS**：
```sh
curl -fsSL https://condr.dev/install.sh | sh
```

**Windows (PowerShell)**：
```powershell
irm https://condr.dev/install.ps1 | iex
```

安装脚本会将二进制文件安装至对应目录，并自动将其添加至系统环境变量 `PATH` 中：
- **Linux / macOS**: `~/.local/opt/condr`
- **Windows**: `%LOCALAPPDATA%\Programs\Condr`

重新打开终端，验证安装结果：
```sh
condr --version
```

### 2. 启动服务

```sh
condr server start
```

Server 运行于守护进程模式，关闭当前终端不影响其运行。

*注：目前尚未注册系统开机自启服务，主机重启后需手动执行该命令启动。*

### 3. 高级安装与环境配置

通过注入环境变量可切换发布通道、指定版本或强制覆盖安装：

**Linux / macOS**：
```sh
# 切换至每日 Nightly 构建
CONDR_VERSION=nightly curl -fsSL https://condr.dev/install.sh | sh

# 指定安装特定版本
CONDR_VERSION=v0.1.0 curl -fsSL https://condr.dev/install.sh | sh

# 强制重装当前版本
curl -fsSL https://condr.dev/install.sh | sh -s -- --force
```

**Windows (PowerShell)**：
```powershell
# 切换至每日 Nightly 构建
$env:CONDR_VERSION = 'nightly'; irm https://condr.dev/install.ps1 | iex

# 指定安装特定版本
$env:CONDR_VERSION = 'v0.1.0'; irm https://condr.dev/install.ps1 | iex

# 强制重装当前版本
$env:CONDR_INSTALL_ARGS = '--force'; irm https://condr.dev/install.ps1 | iex
```

---

## 版本发布通道

Condr 默认每 5 小时在后台自动检测一次通道更新。可在客户端通过 **Settings › About › Updates** 查看版本状态。

| 渠道 | 特性 | 适用受众 |
| :--- | :--- | :--- |
| **Stable** | 经过测试的标准稳定发行版（默认） | 绝大多数生产与日常场景 |
| **Nightly** | 每日由 `main` 分支自动构建，包含最新特性 | 尝鲜用户、功能测试与 Bug 验证 |

---

## 升级指南

> **兼容性说明**：客户端与 Server 端版本需保持匹配。若两端存在版本差但协议兼容，侧边栏将提示黄色警示图标；若协议不兼容，连接将被强制拒绝。

### 桌面应用升级

直接下载新版本覆盖安装。旧 Server 的迁移策略如下：

- **Windows**：安装程序自动停止旧版 Server，并在启动新版客户端时同步升级启动新 Server。
- **macOS / Linux**：安装后旧版 Server 仍维持运行，新版客户端启动时会提示重启服务；亦可稍后在 **Settings › Remote access** 中手动点击 **Restart Condr**。

### Headless Server 升级

重新执行对应安装脚本即可完成检测与覆盖更新。

若执行时检测到 Server 正在运行，脚本会提示是否立即重启：
- **立即重启**：直接生效新版本。
- **稍后重启**：继续维持当前进程，待手动重启后完成版本过渡。

> **关于重启影响**：重启 Server 将中断正在运行的所有 Pane 进程，随后系统会基于快照自动恢复工作区（Workspace）、标签页（Tab）与分屏布局（Pane）。配置了 Hook 的 Agent 将自动恢复会话，详见 [Workspace、Tab 与 Pane](/zh-cn/docs/using/workspaces/)。

---

## 卸载

卸载过程仅移除应用本体与可执行脚本，**不会删除你的工作配置、运行状态及本地日志**。如需彻底清除残留数据，请参考[配置与设置](/zh-cn/docs/reference/configuration/#存储目录与文件系统)手动清理对应目录。

### 步骤 1：清理主应用

| 安装形式 | 卸载方式 |
| :--- | :--- |
| **Windows 桌面端** | 进入系统 **“设置” › “应用” › “已安装的应用”** 卸载。安装程序会自动终止后台服务并清理系统变量。 |
| **macOS 桌面端** | 将 `Condr.app` 移至废纸篓，并继续执行清理命令。 |
| **Linux 桌面端** | 删除对应的 `.AppImage` 文件，并继续执行清理命令。 |
| **Headless Server** | 直接执行清理命令。 |

### 步骤 2：清理 CLI 与后台服务

在 **除 Condr 自带终端之外** 的常规系统终端中执行：

```sh
condr server uninstall
```

此命令将确认后主动停止运行中的 Server 进程，并删除 `condr` 二进制文件与关联的 PATH 环境变量。
*自动化脚本环境中可追加 `--yes` 参数跳过交互确认：`condr server uninstall --yes`*
