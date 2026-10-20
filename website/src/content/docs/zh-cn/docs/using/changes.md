---
title: 改动与文件预览
description: 在 Condr 中快速查看 Agent 修改的文件、对比代码差异（Diff）以及预览项目文件。
---

当 Agent 帮你修改或重构代码时，你需要快速确认它改了哪些文件、代码改得对不对。Condr 内置了文件改动与预览视图，也能直接对文件做小改动，无需打开外部 IDE 即可完成审查。

## 打开改动面板 (Changes)

- **快捷键**：按下 `Cmd + Option + B` (macOS) 或 `Ctrl + Alt + B` (Windows/Linux)。
- **界面入口**：点击右侧栏标题栏的侧栏按钮打开。

改动视图会根据 Git 状态将修改的文件分为三组：
- **Conflicts**：存在合并冲突的文件。
- **Tracked**：已追踪并被修改的文件。
- **Untracked**：Agent 新建但尚未纳入 Git 追踪的文件。

每一个文件后都会标出新增和删除的行数，顶部会汇总累计变动行数。

## 对比 HEAD 或 base 分支

改动列表上方的按钮写着当前的比较对象，例如 `Against main`，点开菜单可以改选：

- **HEAD**：只列出尚未提交的改动。Agent 提交之后，这些文件就会从列表中消失。
- **base 分支**：菜单里显示分支名，例如 `main`。列表包含这条分支从分叉点以来的全部改动，已提交和未提交的都在内，base 分支后来新增的提交不会列入。

Condr 新建 Worktree 时，会把父 Workspace 当时所在的分支记为 base 分支。其他 Workspace 依次使用远端的默认分支（例如 `origin/main`）、本地的 `main` 和 `master`。当前分支本身就是 base 分支或它的上游时，只能对比 HEAD。

每个 Workspace 分别记住自己的选择。

## 查看代码差异 (Diff)

1. 在 **Changes** 列表中点击任意文件，中间区域会自动打开 **Diff** 标签页。
2. 页面会以高亮形式展示该文件相对于所选比较对象的具体改动，标题栏会标出比较对象，例如 `Against main`。
3. 点击标题栏的 **Show File** 可以直接跳转到该文件的完整内容预览。

> **提示**：极大的二进制文件或单文件超过 1 MiB 时，出于性能保护会自动隐藏明细对比。

## 浏览项目文件与预览 (Files & Preview)

- **切换至文件树**：在右侧栏顶部将标签切至 **Files**，即可像文件管理器一样展开和浏览项目文件夹。
- **改动标记**：有修改的文件夹旁会带有一个圆点标记，被 `.gitignore` 忽略的文件会自动变灰。
- **点击预览**：点击任意文本文件，即可在 **Preview** 标签页中打开，代码高亮与滚动位置在磁盘文件变动时会自动刷新。
- **Markdown 与图片渲染**：Markdown 与 SVG 文件默认以渲染后的效果打开，可在 Tab 顶部的 **Preview** 与 **Source** 间随时切换。常见格式图片（PNG、JPEG、GIF、WebP、BMP、ICO、TIFF，约 2 MiB 以内）会自动缩放适应窗口，动图可直接播放。Markdown 中的图片仅支持加载项目本地文件，网络图片则显示其替代文字。

## 编辑文件 (Preview)

Preview 标签页的 **Source** 视图就是编辑器，本地和远程 Device 都能用：

- **编辑与保存**：直接修改文件内容，然后按 `Cmd + S` (macOS) 或 `Ctrl + S` (Windows/Linux) 保存。Tab 顶部的圆点表示有未保存的改动。Condr 不会自动保存，因此你的编辑不会和 Agent 的写入相互抢先。
- **保留换行符**：使用 Windows 换行符（`CRLF`）的文件，新增的行也会沿用 `CRLF`。
- **文件在磁盘上被改动时**：如果你还有未保存的改动时 Agent 写了这个文件，你的文本保持不变，顶部会提示 **Changed on disk**。此时保存会询问是用你的文本 **Overwrite**（覆盖），还是 **Discard and Reload**（放弃改动并重新加载）。
- **未保存的改动不会被悄悄丢掉**：在切换到其他文件、关闭 Tab 或退出 Condr 之前，Condr 会先询问是否保存。如果是另一台设备切换或关闭了这个 Tab，你仍然可以选择保存、复制或放弃这些文本。
- **只读文件**：不是有效 UTF-8 的文本、二进制文件和超过 1 MiB 的文件只能查看、不能编辑，顶部会说明原因。

## 快捷右键菜单

在 Changes 或 Files 列表中的文件上右键，可选择以下常用操作：

- **Open in ...**：选择用本地已安装的 VS Code、Zed、Cursor 或 IntelliJ IDEA 打开当前文件或项目。
- **Insert Path into Terminal**：快速将文件的相对路径粘贴到最近使用的终端窗格中，方便传给命令行工具。
- **Copy Relative Path / Copy Path**：一键复制文件相对路径或绝对路径。
- **Reveal in Finder / Reveal in File Explorer / Open Containing Folder**：在系统文件管理器中显示该文件或文件夹（仅限本地 Device）。
- **New File… / New Folder…**：在 Files 视图中新建：在文件夹上新建到其中，在文件上新建到同一目录，在文件树下方的空白处则新建到项目根目录。新文件会在 Preview 标签页中打开，可以直接输入。
- **Rename…**：在原文件夹内重命名文件或文件夹，名字已被占用时会拒绝。
- **Delete…**：确认后删除文件，或删除文件夹及其中的全部内容。删除是永久性的，不进系统回收站。

Condr 不会改动 `.git` 里的任何内容。
