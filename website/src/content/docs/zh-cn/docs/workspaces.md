---
title: Workspace、Tab 与 Pane
description: 打开项目、给每个 Agent 分配独立分支，并整理终端布局。
---

按下面的步骤打开项目，让多个 Agent 使用各自的分支，并了解关闭或重启后的结果。

## 打开项目

点侧栏 Device 旁的 **New Workspace**，或在欢迎页点 **Open Project**，再选一个文件夹。本机 Device 打开系统文件夹选择器。远程 Device 要求你输入那台机器上的绝对路径，并列出下面的目录供你确认。

Condr 打开 Workspace，在第一个 Tab 里于该文件夹启动 shell。Workspace 默认使用文件夹名。右键它 → **Rename Workspace** 可改名。

Workspace 由一个文件夹和其中的终端组成。在 shell 里运行 `cd` 不会改变 Workspace 根目录。侧栏里每个 Workspace 占两行。第一行是名称，第二行是当前分支，以及相对上游领先和落后的提交数。不在 Git 仓库里时，第二行显示「no git」。

你也可以在终端里创建：

```sh
condr workspace create --cwd ~/code/app --label app
```

## 给每个 Agent 分配独立分支

两个 Agent 改同一个工作目录时可能互相覆盖。Git worktree 为每个 Agent 提供同一仓库的另一个目录和独立分支。

右键 Git 仓库的 Workspace → **Create Worktree**，输入分支名。默认名称是 `worktree/` 加 Workspace 名。分支不存在时，Condr 从当前 HEAD 创建它。分支存在时，Condr 直接检出。

Condr 把新目录放到仓库旁的 `<repo>.worktrees/<branch>`，并在侧栏添加一个 Workspace。要换位置，在 Server 配置中设置 `[server] worktree_root`。见[配置与设置](/zh-cn/docs/configuration/)。

要使用已有的 worktree，选 **Open Existing Worktree**。Condr 不会删除用这种方式打开的 worktree。

要删除 Condr 创建的 worktree，右键它 → **Remove Worktree**。Condr 删除工作目录，但保留分支。目录有未提交或未跟踪文件时，Condr 会拒绝删除。先清理目录。

命令行目前不能创建 worktree。运行 `condr workspace list` 可查看每个 Workspace 对应哪个 worktree。

## 用 Tab 分开工作

Tab 是 Workspace 中的一屏。点 Tab 栏的 **+** 新建 Tab。新 shell 在你当前查看的 Pane 目录中启动。拖动 Tab 可排序。Tab 放不下时，Tab 栏会横向滚动，新激活的 Tab 会滚进可见范围。右键 Tab 可选 **Rename Tab** 或 **Close Tab**。未命名的 Tab 只显示序号。

关闭最后一个 Tab 会关闭 Workspace，所以 Condr 会先请求确认。

## 分割和整理 Pane

Pane 是一个终端。打开它的 **⋮** 菜单即可选择：

- **Split Right** 和 **Split Down**：在同一目录中以 50/50 比例打开 shell。
- **Swap**：与指定方向的邻居交换位置。
- **Toggle Zoom**：让该 Pane 填满 Tab，再点一次还原。标题中也有放大按钮。
- **Close Pane**。

把 Pane 标题拖到另一个 Pane 上。放在边缘会移到该侧，放在中间会交换两个 Pane。拖动 Pane 之间的分隔线可调整大小。Pane 放大时不能执行这些操作。

当前 Pane 有蓝色边框。快捷键见[键盘快捷键](/zh-cn/docs/keybindings/)。

## 使用终端功能

每个 Pane 都是基于 alacritty 核心的真实终端。它使用 `TERM=xterm-256color`，支持真彩色，滚动历史为 10,000 行。

**选择和复制。** 拖动可选择文字，双击选词，三击选行。程序启用鼠标模式时，按住 Shift 拖动。Server 保存选区，所以选区会随输出滚动，调整窗口大小或切换屏幕后清除。复制会结束选区。Condr 不会在选中时自动复制。

**打开链接。** macOS 按住 Cmd，其他平台按住 Ctrl，再点击 URL。URL 会在浏览器打开。悬停可查看完整地址。

**粘贴图片。** 连接远程 Device 时，按 Alt+V，macOS 按 Option+V，把剪贴板图片上传到那台机器。再把文件路径粘贴到终端，就能交给远程 Agent。支持 PNG、JPEG、GIF、WebP 和 BMP，最大 16 MiB。图片存放在 Server 的运行时目录，断开连接时删除。本机 Device 不需要这个功能，直接把文件拖进终端。

**设置终端属性。** 程序可以设置窗口标题，用 OSC 52 写剪贴板，发送 OSC 8 超链接，修改前景色、背景色和调色板。Condr 会把 OSC 52 的剪贴板内容发送到每个已连接的窗口。

**暂不支持。** 终端搜索、块选择、kitty keyboard 协议、Sixel、kitty 图形，以及通过 OSC 52 读取剪贴板。

## 关闭时会发生什么

关闭 Pane 会结束它启动的所有程序。Condr 依次发送 SIGHUP、SIGTERM 和 SIGKILL。Windows 上每个 Pane 有自己的 Job Object，关闭时会结束整个 Job。Unix 上用 `setsid` 或 `nohup` 启动的进程，以及 Windows 上用 `CREATE_BREAKAWAY_FROM_JOB` 启动的进程，会因主动脱离而继续运行。

关闭 Tab 的最后一个 Pane 会关闭 Tab。关闭 Workspace 的最后一个 Tab 会关闭 Workspace。只有关闭操作会连带关闭 Workspace 时，Condr 才会请求确认。关闭 Workspace 会停止终端，但不会删除文件。

shell 自行退出时，Pane 保留最后画面并标记为已退出。它启动的其他程序也会按同样方式结束。

## 让两个窗口互不影响

每个窗口独立选择 Workspace 和 Tab。同一台 Server 上的两个窗口，一个窗口切换不会改变另一个窗口。

命令 `condr workspace focus` 和 `tab focus` 是一次性的「请看这里」操作。它们会切换所有已连接窗口，但不会产生共享状态。

Server 一次只接受一个窗口的控制。第二个窗口连上后，Tab 栏显示「Viewing only」。它可以查看和复制，但不能输入。第一个窗口断开后，第二个窗口接管控制。

## Server 重启后剩下什么

Server 每次改动后都会把结构保存到快照文件。重启时恢复：

- **保留**：每个 Workspace、Tab 和 Pane，以及每个 Pane 的目录、分割比例、焦点和 worktree 关系。
- **不保留**：终端内容和滚动历史、正在运行的程序、Agent 状态。

重启会结束每个 Pane 中的程序。Condr 随后在每个 Pane 原来的目录启动新 shell。目录不存在时使用 Workspace 根目录。根目录也不存在时，Condr 移除该 Pane。

装有 hook 的 Agent 会恢复。Condr 记录原生会话号，重启后在同一 Pane 中输入 Agent 自己的恢复命令，例如 `claude --resume <id>`。对话会从中断处继续。恢复失败时，命令会留在终端中，供你重试。

要重启 Server，请在 Condr 外的终端运行 `condr server restart`，或 **Settings › Device › General** → **Restart Condr**。窗口重启后也会恢复窗口位置、侧栏宽度，以及每个 Workspace 正在显示的 Tab。
