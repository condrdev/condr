---
title: 任务
description: 在一个视图里看到每台设备上的所有 Agent，不离开它就能回复，并为写下的工作启动 Agent。
---

同时推进好几个项目时，一个 Workspace 一个 Workspace 地切换很耗注意力。**Tasks** 模式把每台已连接设备上的所有 Agent 放进同一个窗口，按它们需要你做什么排好，点开任何一个就在旁边回复，然后接着处理下一个。

## Tasks 模式

点侧栏最上面的 **Tasks** 行、侧栏收起时最上面的图标，或按 `Cmd+0`（Windows 和 Linux 上是 `Alt+0`）进入。左上角的 **‹ Workspace** 或同样的快捷键会回到你之前看的 Tab。只有主窗口有 Tasks 模式。

Agent 按状态分列：**To do**（还没开始的任务）、**Needs you**（Bell、Blocked 或 Error）、**Done**（完成了还没看过）、**Working** 和 **Idle**。卡片不能拖，它们随 Agent 上报的状态移动。点列名可以把一列收成窄条；Needs you 来了新 Agent 时会自己展开。

- **看板或列表**：看板显示完整的卡片（Workspace、分支、设备、改动、Agent 在等什么）或紧凑的行；列表是一张表。
- **范围**：面包屑 `Tasks › MacBook › api-server` 可以只看一台设备或一个 Workspace；仓库的范围包括从它建出来的 worktree。
- **未连接的设备**：顶部的横幅列出范围内每台没连上的设备，带“重新连接”按钮。

### 回复 Agent

点一张卡片，它的终端就在卡片旁边打开（看板在下方，列表在右侧），直接打字，键盘已经交给了 Agent。`Cmd+]` 和 `Cmd+[`（`Alt+]` 和 `Alt+[`）切到下一个和上一个 Agent，不用鼠标就能挨个回复。再点一次这张卡片或点 ✕ 关掉终端。

点卡片区的空白处，键盘回到卡片区：这时方向键只预览每个 Agent，不会往里打字，也不会改变它的终端尺寸；按 `Enter` 把键盘交给它。**在 Workspace 中打开** 会离开 Tasks 模式，去这个 Agent 自己的 Tab，在那里可以分屏或调整 Pane。Tasks 模式下，布局和 Tab 的快捷键不起作用。

## 任务

**任务**是你在任何 Agent 动手之前先写下来的工作。任务保存在各台设备的 Server 上，所以每个窗口和 `condr task` 命令看到的都是同一批。

- **写任务**：点 To do 最上面的 **+ New task**，或在命令面板里选 **New Task**。填标题、纯文本描述、它属于哪台设备，仓库可选。按 **保存**（`Cmd+S`）之前什么都不会留下。
- **开始任务**：在表单右半边选择在哪里做：仓库自己的 Workspace、带新分支的 **New worktree**，或者（以前开始过的任务）接着在上次的地方做。选好 Agent，按 **开始**。Condr 会打开 Workspace、在里面启动 Agent，并让它去读 Server 写下的任务文件，内容是标题和描述。
- **进行中**：它的 Workspace 里的每个 Agent，卡片上都显示任务的标题。如果 Workspace 里已经没有 Agent，任务会以 “No agent running” 出现在 Idle 列，可以再次开始。
- **归档**：做完或放弃时归档。归档已开始的任务时会问你：要不要关闭它的 Workspace（会结束里面的 Agent），要不要删除它的 worktree（分支总会保留）。归档的任务放在 To do 最下面，可以恢复或删除。

一个 Workspace 同一时间只属于一个任务。启动 Agent 不要求安装 hook；不装的话，Server 重启后不会恢复它的对话。

命令行里管理同一批任务：[`condr task`](/zh-cn/docs/reference/cli/#task)。
