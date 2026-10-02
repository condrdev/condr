---
title: 快捷键与偏好设置
description: 常用键盘快捷键、自定义外部编辑器联动及终端偏好设置。
---

通过设置界面与键盘快捷键，你可以根据个人习惯提升 Condr 的操作效率与界面体验。

## 打开设置界面

- **快捷键入口**：在 macOS 上按下 `Cmd + ,`，在 Windows / Linux 上按下 `Ctrl + ,` 打开设置窗口。
- **设置页签功能**：
  - **Application**：客户端全局设置（包含外观主题、终端字号、系统通知开关及快捷键列表）。
  - **Device**：服务端与设备设置（包含默认 Shell 路径、连接状态及 Agent 集成控制）。

## 自定义外部编辑器 (Open in Editor)

在文件列表（Files）或改动列表（Changes）中右键点击文件，可以调用外部 IDE 或编辑器直接打开对应文件。标题栏的 **Open in** 按钮则打开整个 Workspace 根目录。两者都只对本机 Device 有效。

Condr 会自动找到已安装的 Zed、VS Code、Cursor 和 IntelliJ IDEA。如果默认列表中没有你常用的编辑器，可以在配置文件 `config.toml` 中添加 `[[client.editors]]` 配置：

```toml
[[client.editors]]
name = "Helix"
command = ["hx"]

[[client.editors]]
name = "VS Code Insiders"
command = ["code-insiders", "--reuse-window"]
```

### 参数说明
- `name`：显示在 Open in 菜单中的名称。
- `command`：程序和它的前置参数，Condr 会把要打开的路径追加为最后一个参数。
- 手改配置文件后要重启窗口才生效。Condr 会把每个仓库上次选的编辑器记在 `[[client.workspace_editors]]` 里，这一项不要手改。

## 常用快捷键参考

### 窗口与工作区

| 动作 | macOS | Windows / Linux |
| --- | --- | --- |
| 打开设置 | Cmd+, | Ctrl+, |
| 开关左侧栏 | Cmd+B | Ctrl+Shift+B |
| 开关右侧改动面板 | Cmd+Option+B | Ctrl+Alt+B |
| 上一个工作区 | Cmd+Shift+↑ | Ctrl+Shift+↑ |
| 下一个工作区 | Cmd+Shift+↓ | Ctrl+Shift+↓ |

### 标签页与分屏

| 动作 | macOS | Windows / Linux |
| --- | --- | --- |
| 新建标签页 | Cmd+T | Ctrl+Shift+T |
| 向右分屏 | Cmd+D | Alt+Shift+= |
| 向下分屏 | Cmd+Shift+D | Alt+Shift+- |
| 关闭当前分屏 | Cmd+W | Ctrl+Shift+W |
| 放大或还原当前分屏 | Cmd+Shift+Enter | Alt+Shift+Enter |

### 终端

| 动作 | macOS | Windows / Linux |
| --- | --- | --- |
| 粘贴图片到远程 Device | Option+V | Alt+V |
| 程序开启鼠标模式时选择文本 | Shift+拖动 | Shift+拖动 |
| 打开终端里的链接 | Cmd+点击 | Ctrl+点击 |

完整列表见[键盘快捷键](/zh-cn/docs/reference/keybindings/)。

## 设置终端默认 Shell

默认情况下，新建分屏会使用系统的默认 Shell。你可以通过以下方式修改：

### 在界面中修改
前往 **Settings › Device › General**，在 **Terminal** 区域的 Shell 框中输入目标 Shell 的路径（例如 `/bin/zsh` 或 `/opt/homebrew/bin/fish`）。

### 在配置文件中修改
在 `config.toml` 中写入：

```toml
[server.terminal]
shell = "/opt/homebrew/bin/fish"
```
