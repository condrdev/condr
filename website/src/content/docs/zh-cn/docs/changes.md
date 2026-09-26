---
title: 查看改动
description: 在 Condr 里查看 Agent 改过的文件、差异和项目文件。
---

打开右侧栏，就能在 Condr 内查看 Agent 改了哪些文件、具体差异和项目文件。

## 认识四个视图

| 入口 | 在哪 | 看什么 | 怎么打开 |
| --- | --- | --- | --- |
| **Changes** | 右侧栏 | 哪些文件改了 | 点标题栏的侧栏按钮或使用快捷键 |
| **Files** | 右侧栏 | 项目里有哪些文件 | 用同一按钮，再切到 Files 标签 |
| **Diff** Tab | Tab 栏 | 一个文件改了什么 | 在 Changes 中点文件 |
| **Preview** Tab | Tab 栏 | 一个文件的内容 | 在 Files 中点文件 |

四个视图都是只读的，远程 Workspace 也能使用。

点标题栏的 **Show Changes & Files** 打开右侧栏，或按 Cmd+Alt+B（macOS）或 Ctrl+Alt+B。Git 仓库默认打开 Changes，其他文件夹默认打开 Files。每个 Workspace 记住侧栏是否打开。拖动左边缘可调整宽度。

## 查看改了哪些文件

Changes 按工作目录相对 HEAD 的差异列出所有文件，分为 **Conflicts**、**Tracked** 和 **Untracked** 三组。已暂存和未暂存不分开，每个文件只显示一个状态。

文件显示为目录树。只有一个子目录的目录链会折叠成一行。每个文件显示状态及新增、删除行数。删除的文件会划线。标签上的数字是改动文件数，末尾显示新增和删除行的总数。

改动超过 1000 个时，只列出前 1000 个。通常这是因为构建输出未被忽略。请添加 `.gitignore`。

## 查看文件差异

在 Changes 中点文件，会在 Workspace 里打开 **Diff** Tab。再点其他文件时复用这个 Tab。Diff 与 HEAD 比较，使用终端字体并显示行号，也支持搜索。

标题栏的 **Show File** 会打开 Preview，并跳到光标所在行。删除的文件没有这个按钮。

文件任一侧超过 1 MiB 时不显示 Diff。二进制文件显示「Binary file」。

## 浏览和预览文件

Files 会在你展开目录时加载目录树。目录下有改动时显示一个点。被忽略的文件变灰，改动文件按状态着色。

点文件后，它会在 **Preview** Tab 中打开。Preview 只显示文本文件，按扩展名高亮，文件上限为 1 MiB。磁盘上的文件变化时，Preview 会刷新并保留滚动位置。图片和其他二进制文件显示「Binary file」。

在任一视图中右键文件，可选择：

- **Show File**：在 Changes 中提供，打开 Preview。
- **Copy Relative Path** 和 **Copy Path**：复制相对路径，或该 Device 上的绝对路径。
- **Open in …**：用外部编辑器打开。只对本机 Device 可用。
- **Insert Path into Terminal**：把相对路径和一个空格粘贴到该 Workspace 最近使用的终端，需要时自动加引号。

## 用外部编辑器打开文件

标题栏的 **Open in** 会打开 Workspace 根目录。Condr 会查找已安装的 Zed、VS Code、Cursor 和 IntelliJ IDEA，最后总是提供 Finder、资源管理器或文件管理器。每个仓库记住上次选择，它的 worktree 共用这个选择。

要添加编辑器，在配置文件中写入：

```toml
[[client.editors]]
name = "Helix"
command = ["hx"]
```

路径会追加为最后一个参数。见[配置与设置](/zh-cn/docs/configuration/)。

这个按钮只对本机 Device 可用。远程 Workspace 的文件留在远端机器上，本机编辑器无法打开。

## 了解限制

- Condr 不能编辑文件。使用外部编辑器，或让 Agent 修改。
- Diff 只能与 HEAD 比较，不能与其他分支比较。
- Condr 不能暂存、提交或丢弃改动。
- Preview 不能显示图片。
