---
title: 快捷键速查
description: Condr 在 macOS、Windows 与 Linux 平台的全量按键映射，以及终端复制粘贴、滚动与鼠标交互行为规范。
---

Condr 的快捷键分为两层逻辑：
* **窗口级快捷键**：用于调度 Window、Workspace、Tab 与 Pane。由客户端直接拦截消费，不向终端前台进程透传。
* **终端级快捷键**：覆盖文本/媒体剪贴板、缓冲区滚动及鼠标交互，仅在当前终端 Pane 处于聚焦状态时生效。

未在本页声明的按键组合将全部透传至当前 Pane 内运行的子进程。

> **macOS 平台说明**：macOS 环境下 `Option` 键产生特殊字符（如 `Option + S` 输出 `ß`），不映射为终端 `Meta` 键。因此 Shell 中基于单词步进的快捷键（如 `Option + B`、`Option + F`）不生效。

---

## 窗口与侧边栏

| 操作 | macOS | Windows / Linux |
| :--- | :--- | :--- |
| 打开设置 | `Cmd + ,` | `Ctrl + ,` |
| 关闭设置 | `Esc` | `Esc` |
| 切换左侧栏显示状态 | `Cmd + B` | `Ctrl + Shift + B` |
| 切换右侧栏（Changes & Files） | `Cmd + Option + B` | `Ctrl + Alt + B` |

---

## Workspace

| 操作 | macOS | Windows / Linux |
| :--- | :--- | :--- |
| 切换至上一个 Workspace | `Cmd + Shift + ↑` | `Ctrl + Shift + ↑` |
| 切换至下一个 Workspace | `Cmd + Shift + ↓` | `Ctrl + Shift + ↓` |

* **轮转逻辑**：遵循侧边栏顺序，跨设备遍历所有 Workspace，到达边界后自动循环。
* **初始行为**：当前未显示任何 Workspace 时，`↓` 跳到首个 Workspace，`↑` 跳到最后一个。

---

## Tab

| 操作 | macOS | Windows / Linux |
| :--- | :--- | :--- |
| 新建 Tab | `Cmd + T` | `Ctrl + Shift + T` |
| 切换至下一个 Tab | `Cmd + }` 或 `Ctrl + Tab` | `Ctrl + Tab` |
| 切换至上一个 Tab | `Cmd + {` 或 `Ctrl + Shift + Tab` | `Ctrl + Shift + Tab` |
| 直达指定 Tab（1~9） | `Cmd + 1` ~ `Cmd + 9` | `Alt + 1` ~ `Alt + 9` |

* **物理按键**：`Cmd + }` / `Cmd + {` 对应 `Cmd + Shift + ]` / `Cmd + Shift + [`。
* **轮转逻辑**：Tab 切换到达边界后自动循环。

---

## Pane 分屏与调度

| 操作 | macOS | Windows / Linux |
| :--- | :--- | :--- |
| 水平拆分（向右新建） | `Cmd + D` | `Alt + Shift + =` |
| 垂直拆分（向下新建） | `Cmd + Shift + D` | `Alt + Shift + -` |
| 关闭当前 Pane | `Cmd + W` | `Ctrl + Shift + W` |
| 最大化 / 还原焦点 Pane | `Cmd + Shift + Enter` | `Alt + Shift + Enter` |
| 切换焦点方向 | `Cmd + Option + 方向键` | `Alt + 方向键` |
| 调整分屏占比 | `Cmd + Ctrl + 方向键` | `Alt + Shift + 方向键` |

* **微调步进**：快捷键每次调整分屏占比 5%。
* **拖拽调度**：
  * **交换位置**：拖拽 Pane 标题栏并释放于目标中央。
  * **拆分并置**：拖拽至目标边缘，将目标 Pane 朝对应方向拆分并嵌入。

---

## 终端：文本复制与粘贴

| 操作 | macOS | Windows | Linux |
| :--- | :--- | :--- | :--- |
| 复制选区 | `Cmd + C` | `Ctrl + C` / `Ctrl + Shift + C` | `Ctrl + Shift + C` |
| 粘贴文本 | `Cmd + V` | `Ctrl + Shift + V` / `Ctrl + V` | `Ctrl + Shift + V` |
| 复制选区（通用） | `Ctrl + Insert` | `Ctrl + Insert` | `Ctrl + Insert` |
| 粘贴文本（通用） | `Shift + Insert` | `Shift + Insert` | `Shift + Insert` |

### 按键透传机制

* **选区生命周期**：复制动作触发后自动清空当前选中高亮。
* **复制提示**：经过 Condr 的复制会显示“Copied to clipboard”。来源可以是 Condr 自己的选区，也可以是程序（如 Claude Code）复制的内容，后者会注明来自哪个 Pane。程序自行调用系统剪贴板的复制不经过 Condr，不会有提示；例如 Codex 会复制到它所在机器的剪贴板，在远程 Device 上请改用 `Shift + 拖动` 让 Condr 来复制。
* **macOS 无选区时的 `Cmd + C`**：按键会交给前台程序。支持的程序（如 Claude Code）会复制自己的选区，Shell 等其他程序收不到任何输入。在 Windows Device 上，只要 Condr 已识别出 Pane 里的 Claude Code，`Cmd + C` 同样会送达它。
* **Windows 平台的冲突处理**：
  * 无选区时按 `Ctrl + C`，按键透传至前台应用，通常用于中断当前命令。
  * `Ctrl + V` 始终由 Condr 拦截并执行粘贴，不向子进程发送原始按键。
* **Linux 平台**：`Ctrl + C` 与 `Ctrl + V` 始终无条件透传至当前子进程。

---

## 终端：远程设备图片投递

| 操作 | macOS | Windows / Linux |
| :--- | :--- | :--- |
| 粘贴剪贴板图片 | `Option + V` | `Alt + V` |

* **投递逻辑**：仅针对运行在远程设备的 Pane 生效。Condr 会将本地剪贴板图片异步上传至该设备的临时目录，并将远程绝对路径自动回填至命令行光标处，供 CLI Agent 等程序读取。
* **回退行为**：若剪贴板无图像资产，或当前 Pane 位于本机设备，该按键直接透传至前台程序。
* **配额与生命周期**：
  * 单张图片最大支持 **16 MiB**。
  * 远程暂存文件在当前客户端断开连接或远程 Server 进程终止时自动销毁。

---

## 终端：滚动与缓冲区浏览

| 操作 | macOS | Windows / Linux |
| :--- | :--- | :--- |
| 缓冲区向上翻页 | `Shift + PageUp` | `Shift + PageUp` |
| 缓冲区向下翻页 | `Shift + PageDown` | `Shift + PageDown` |
| 逐行滚动 | 鼠标滚轮 | 鼠标滚轮 |

* **自动沉底**：浏览历史缓冲区时，向终端输入任意按键均会立即复位滚动条至最新行。
* **Alt Screen 模式（vim / less 等）**：`Shift + PageUp / PageDown` 透传至 TUI 程序内部处理；鼠标滚轮将自动转义为 `↑` / `↓` 方向键事件分发。
* **鼠标报告模式（Mouse Reporting）**：前台程序显式开启鼠标追踪时，滚轮事件由其内部事件循环接管。

---

## 终端：鼠标交互

| 操作 | macOS | Windows / Linux |
| :--- | :--- | :--- |
| 访问 URL 链接 | `Cmd + 点击` | `Ctrl + 点击` |
| 强制选择文本（程序接管鼠标时） | `Shift + 拖动` | `Shift + 拖动` |

---

## 延伸阅读

* [快捷键与偏好设置](/zh-cn/docs/using/preferences/)：常用快捷键与外部编辑器绑定
* [Workspace、Tab 与 Pane](/zh-cn/docs/using/workspaces/)：分屏布局与多任务工作流管理
* [配置与设置](/zh-cn/docs/reference/configuration/)：设置界面参数及 `config.toml` 配置规范
