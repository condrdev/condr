---
title: Workspace、Tab 与 Pane
description: 打开项目、用 Git Worktree 为 Agent 分配隔离分支，以及排布终端布局。
---

Workspace（工作区）是 Condr 管理项目的基本单位。一个工作区对应一个项目文件夹，你可以在其中通过分屏和多标签页运行多个终端或 Agent。

## 打开项目（Workspace）

- **界面打开**：在侧边栏点击 **New Workspace**，从三种方式里选一种；欢迎页也列出同样三项。
  - **Open Project**：打开已有的文件夹。本地直接选择，远程服务器上输入文件夹的绝对路径。
  - **Clone Repository**：粘贴仓库 URL，确认放在哪、文件夹叫什么，点 **Clone**。Condr 建好文件夹、打开工作区，等 Shell 就绪后在第一个终端里输入 `git clone`。克隆就在那里用那台机器自己的 git、密钥和凭据运行，密码、主机指纹等提问直接在终端里回答，按 Ctrl+C 可以中止。
  - **New Directory**：在工作区所在的机器上新建一个空文件夹。
- **命令行打开**：在终端运行 `condr workspace create --cwd ~/code/my-app`，或用 `condr workspace clone <url> [<目录>]` 克隆到一个新文件夹。

### 识别项目状态

侧边栏的项目列表会直接显示状态：

- **项目名称**：默认显示文件夹名字，右键 → **Rename Workspace** 可重命名。
- **Git 状态**：第二行显示当前分支，以及相对上游领先和落后的提交数，例如 `main ↑1 ↓2`。不在 Git 仓库里时显示 `no git`。
- **折叠设备**：鼠标移到设备标题上，点名字前面的箭头，就能收起它下面的项目。折叠后，名字旁会按状态统计其中的 Agent；重启后仍然保持折叠；当前正在看的项目在它里面时，标题会高亮。

## 划分与整理终端界面

### 多标签页（Tabs）

- **新建**：点击顶部 **+** 号新建标签页，每个标签页继承当前的工作目录。
- **整理**：右键标签页 → **Rename Tab** 可重命名，按住标签页可**拖拽排序**。

### 分屏布局（Panes）

点击 Pane 右上角的菜单：

- **分屏**：选择 **Split Right**（左右分屏）或 **Split Down**（上下分屏）。
- **放大专注（Zoom）**：选择 **Toggle Zoom**，将当前分屏临时铺满整个窗口，再按一次恢复原样（适合查看长日志）。
- **调整位置**：直接按住分屏的标题栏**拖拽**，即可与旁边分屏交换位置或重新组合。

## 高效使用技巧

### 向远程 Agent 发送截图

当你连接远程服务器跑 Agent 时，本地复制一张截图，直接在远程终端按下和本机给 Agent 粘贴图片相同的键。按键看你正在用的这台电脑，与远程 Device 无关：

- macOS / Linux：Ctrl+V
- Windows：Alt+V

Condr 会自动将图片传到服务器，并将远程图片路径粘贴到命令行中，直接供 Agent 读取。

### 打开终端里的超链接

按住按键并点击终端里的网址，即可用默认浏览器打开：

- **macOS**：`Cmd + 鼠标左键`
- **Windows / Linux**：`Ctrl + 鼠标左键`

### 强制选择文本

在运行 `vim` 或其他占用鼠标的终端程序时，按住 `Shift` 键再用鼠标拖拽，即可强行选中并复制终端文本。

### 终端里的图片

用 kitty 图形协议画图的程序，图片会直接显示在 Pane 里：yazi 的预览、`kitten icat`、viu、OpenCode 的图片预览、image.nvim、snacks.nvim、presenterm、timg，以及基于 ratatui-image 的程序。图片跟着文字滚动，动画会播放，远程 Device 和手机 App 上也一样能看到。

少数程序只看环境变量，需要手动告诉它们：

- **chafa**：`chafa -f kitty`
- **Codex**：目前在 Condr 里不画图，它只认环境变量里的 kitty、Ghostty 或 WezTerm。

不支持 Sixel 和 iTerm2 的图片协议。Windows 上程序运行在系统的控制台层（ConPTY）之后，它不会把协议的应答传回给程序，所以大多数程序会判断不支持，退回文字。
