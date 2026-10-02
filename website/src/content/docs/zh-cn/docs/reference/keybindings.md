---
title: 键盘快捷键
description: 查找 macOS、Windows 和 Linux 上的全部 Condr 快捷键。
---

:::caution[施工中]
这一页还在编写中，内容会陆续补齐。
:::

需要快捷键时查这页。Condr 也会在 **Settings › Shortcuts** 显示同一份列表。

## 了解哪些按键由 Condr 处理

Condr 只截获表格中的快捷键。其他按键都交给 Pane 中的程序，包括 macOS 上除 Ctrl+Tab 外的所有 Ctrl 组合。没有绑定的 Cmd 或 Win 组合会被丢弃，不会到达终端。对话框打开时没有快捷键生效。

Windows 和 Linux 上 Ctrl+B 留给 tmux 和 readline，因此侧栏使用 Ctrl+Shift+B。macOS 上 Option 加字母会输入该字符，Option 不作为 Meta 键。

## 操作窗口和侧栏

| 动作 | macOS | Windows / Linux |
| --- | --- | --- |
| 打开设置 | Cmd+, | Ctrl+, |
| 开关左侧栏 | Cmd+B | Ctrl+Shift+B |
| 开关右侧栏（Changes & Files） | Cmd+Alt+B | Ctrl+Alt+B |
| 关闭设置窗口 | Esc | Esc |

## 切换 Workspace 和 Tab

| 动作 | macOS | Windows / Linux |
| --- | --- | --- |
| 上一个 Workspace | Cmd+Shift+↑ | Ctrl+Shift+↑ |
| 下一个 Workspace | Cmd+Shift+↓ | Ctrl+Shift+↓ |
| 新建 Tab | Cmd+T | Ctrl+Shift+T |
| 下一个 Tab | Cmd+} 或 Ctrl+Tab | Ctrl+Tab |
| 上一个 Tab | Cmd+{ 或 Ctrl+Shift+Tab | Ctrl+Shift+Tab |
| 跳到第 1 到 9 个 Tab | Cmd+1 到 Cmd+9 | Alt+1 到 Alt+9 |

Workspace 按侧栏中每个 Device 的顺序切换，到末尾后回到开头。重命名或关闭 Workspace、关闭 Tab 时使用右键菜单，这些操作没有快捷键。

## 操作 Pane

| 动作 | macOS | Windows / Linux |
| --- | --- | --- |
| 向右分割 | Cmd+D | Alt+Shift+= |
| 向下分割 | Cmd+Shift+D | Alt+Shift+- |
| 关闭 Pane | Cmd+W | Ctrl+Shift+W |
| 放大或还原 | Cmd+Shift+Enter | Alt+Shift+Enter |
| 移动焦点 | Cmd+Alt+方向键 | Alt+方向键 |
| 调整大小 | Cmd+Ctrl+方向键 | Alt+Shift+方向键 |

每按一次调整 5%。交换 Pane 没有快捷键，请使用 Pane 菜单或拖动。

## 操作终端

| 动作 | macOS | Windows | Linux |
| --- | --- | --- | --- |
| 复制 | Cmd+C；没有选区时交给程序 | Ctrl+Shift+C，或有选区时 Ctrl+C | Ctrl+Shift+C |
| 粘贴 | Cmd+V | Ctrl+Shift+V 或 Ctrl+V | Ctrl+Shift+V |
| 粘贴图片到远程 Device | Option+V | Alt+V | Alt+V |
| 打开链接 | Cmd+点击 | Ctrl+点击 | Ctrl+点击 |
| 在鼠标模式下选择 | Shift+拖动 | Shift+拖动 | Shift+拖动 |
| 滚动历史 | Shift+PageUp / PageDown，或滚轮 | 同左 | 同左 |

Ctrl+Insert 在三个平台复制，Shift+Insert 粘贴。Linux 上 Ctrl+C 和 Ctrl+V 总是交给终端。Tab 和 Shift+Tab 也交给终端，不会移动窗口焦点。

复制会结束选区。macOS 上没有选区时，Cmd+C 会交给程序：使用 kitty 键盘协议的程序会收到 Cmd+C 本身，Claude Code 据此复制它自己的鼠标选区；shell 和其他程序什么都收不到。没有清屏快捷键，请在 shell 中使用 `clear` 或 Ctrl+L。
