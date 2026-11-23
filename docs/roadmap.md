# Condr Roadmap

## 近期方向

按优先级排列。每项写清为什么、做到哪、什么时候停。

### 终端内搜索

**为什么**：这一项原本在摩擦记录里等证据。Agent 的输出很长，要在里面找一段报错或一个路径，现在只能靠滚动。对外宣布后，新用户会拿 Condr 和他们用惯的终端比较，而这些终端都能搜索。

**做到哪**：回滚内容只在 Server 的 VT 里，所以搜索是一个协议请求，由 Server 在 VT 上查找并返回匹配的位置，GUI 高亮并滚动过去。CLI 可以用同一个请求。

**停在哪**：只在当前 Pane 里搜索，不做跨 Pane 或跨 Workspace 的搜索，不做文件快速打开。

### 探测 Workspace 里的端口

**为什么**：转发要用户自己知道端口号。Server 已有按平台实现的进程表（Agent 检测用），加一步"该进程树在监听哪些端口"是同一条路；探测到的端口点一下就转发，不必再看终端里打出的地址。

**做到哪**（落地前先写 ADR）：Server 在进程表变化时查 Pane 进程树的监听端口（Linux `/proc/net/tcp*`，macOS `proc_pidfdinfo`，Windows `GetExtendedTcpTable`），作为 Pane 状态经事件下发；GUI 在 Pane 上显示端口徽标，Ports 页面列出探测到、尚未转发的端口，点一下即调用 ADR 0061 的转发；`condr pane|workspace` 的 JSON 携带端口。

**停在哪**：不自动转发探测到的端口（默认只列出，点了才转），不解析进程输出里的 URL。

### GitHub：PR 状态与检出 PR

**为什么**：收尾的最后一步通常是推分支、开 PR、等 CI。现在用户要切到浏览器，才知道这个 Workspace 的分支有没有 PR、检查过没过；要在本机看别人的 PR，也得自己建 worktree 再检出。把这些状态放到 Workspace 上，收尾就不用离开 Condr。

**做到哪**（按顺序，落地前先写 ADR）：

1. Server 识别 Workspace 当前分支对应的 GitHub PR，查询它的状态（打开、草稿、已合并、已关闭）和 CI 检查结果，作为 Workspace 状态经事件下发。侧栏和 Workspace 上显示 PR 编号与检查状态，点击在浏览器打开；`condr workspace` 的 JSON 带上这些字段。
2. 输入 PR 编号或链接，新建一个检出该 PR 的 Managed Worktree。
3. PR 合并后，Workspace 上提示可以 Remove Worktree。

ADR 要回答三个问题：

- 认证从哪来：复用 `gh` 的登录，还是读 `GITHUB_TOKEN`。无论哪种，token 只留在 Server 所在的机器上。
- 多久查一次。只查有 Workspace 打开的分支，并按 GitHub API 的限额安排。
- 检出 PR 要先 fetch，gix 的 fetch 和凭据处理够不够用。

**停在哪**：只做 GitHub，GitLab 和 Gitea 等真实需求。不在 Condr 里创建、合并或评审 PR，这些交给 Agent 和 `gh`。PR 合并后不自动删除 worktree，删除仍是用户的显式操作（ADR 0002）。不做从 Issue、PR 评论或聊天工具触发 Agent，那需要一个面向公网的服务。

### 新建的 worktree 拿来就能用

**为什么**：Managed Worktree 只检出已提交的文件。`.env` 这类被忽略的文件不在，依赖也没装，Agent 一启动就失败，用户得先手工准备。新建时只能从父 Workspace 的 HEAD 分叉，选不了 base 分支或远程分支；分支只以 `origin/x` 存在时，`create_worktree` 会从 HEAD 新建一个与远程无关的 `x`，而 `git worktree add` 这时会建一个跟踪 `origin/x` 的分支。`agent start|prompt|wait` 能在 CLI 里编排多个 Agent，CLI 却建不了 worktree，派出去的 Agent 只能共用一个目录。

**做到哪**（按顺序，落地前先写 ADR）：

1. 起点：分支只以 `origin/x` 存在时，像 git 一样建一个跟踪它的本地分支。新建时可以选 base 分支或远程分支，`CreateWorktree` 增加起点字段，GUI 的分支选择列出远程分支。ADR 补充 Base Branch（ADR 0034）在选定起点时记什么。
2. CLI：`condr workspace create` 能新建 Managed Worktree，同样可选起点，并可在新 Workspace 的第一个 Pane 里启动 Agent、发出第一条提示；JSON 返回新的 Workspace 与 Pane。内嵌 Skill 写上“一个 Agent 一个 worktree”的编排方式。
3. 初始化：仓库里一份可以提交的声明，写明要从父仓库复制的被忽略文件（考虑沿用 `.worktreeinclude` 的写法）、setup 命令和 teardown 命令，从父仓库已提交的版本读取。Server 在 `CreateWorktree` 之后复制文件，在新 Workspace 的第一个 Pane 里执行 setup，输出留在终端里；要启动 Agent 的，等 setup 结束再启动。Remove Worktree 之前执行 teardown。这是在执行仓库里写下的命令，所以第一次执行和声明内容变化时都要用户确认，确认记在 Server 这台机器上；Windows 要有 PowerShell 的写法。

**停在哪**：setup 失败不自动删除 worktree，输出留在 Pane 里给人看。不做后台服务的监管、端口分配或反向代理（端口见“探测 Workspace 里的端口”）。不做一个任务在多个仓库各开一个 worktree（见“多仓库 Workspace 的 Changes”）。

### 公开预览收尾，拿到第一批外部用户

**为什么**：摩擦记录和远期表里几乎每一项的启动条件都是外部用户或真实需求。没有外部用户，这些条件不会满足，路线只能靠单人 dogfood 推进。但宣布带来的新用户会按宣布时的样子判断 Condr，所以先做完上面各项再宣布，让他们第一次试用就能走完多 worktree 的完整流程，离开电脑也能在手机上看到 Agent 的状态。

**做到哪**（按顺序）：

1. 上面各项完成后，对外宣布。
2. 外部 Issue 按 [triage 标签](agents/triage-labels.md) 分流。缺陷直接修；功能请求进摩擦记录，同一件事出现两次以上再排。

**停在哪**：Windows 安装包预览期仍不签名，文档已写明 SmartScreen 放行步骤。

**退出条件**：外部用户按文档装好，并提出第一批 Issue。

### WSL 发行版作为 Device

**为什么**：Windows 是 Condr 的一等平台，可 Windows 上很多人在 WSL 里跑 Agent。今天在 Windows Server 的 Pane 里运行 `wsl`，进程表只看到 `wsl.exe`，认不出里面的 Agent，发行版里的 hooks 也找不到 `condr`。

**做到哪**（落地前先写 ADR）：沿用 ADR 0015 的 bridge，Client 执行 `wsl.exe -d <distro> -- condr server bridge`，经 stdin/stdout 跑同一套协议，得到一个 `wsl://<distro>` Device。Server 跑在发行版里，Agent 检测、hooks 和 Git 都在 Linux 一侧照常工作。Connect Remote Device 列出本机已装的发行版；发行版里还没有 `condr` 时，给出安装命令。`condr --device` 同样可以直连。

**停在哪**：只做 WSL 2。不自动在发行版里安装 `condr`，和 SSH 一样留给以后的远端安装。不做 Windows 与 WSL 之间的路径翻译。

### 多仓库 Workspace 的 Changes

**为什么**：常见的做法是把几个仓库放在一个父目录下，一起交给 Agent。今天只有 Workspace 在某个仓库里时才有 Changes，父目录本身不是仓库，右侧栏就只剩 Files。

**做到哪**（落地前先写 ADR）：Server 找出 Root Directory 下一层的子仓库，各算一份状态；Changes 和 History 按仓库分组，Diff Tab 用“仓库 + 路径”定位。ADR 要说明 Base Branch 和 Managed Worktree 在多仓库下的含义；先只做查看。

**停在哪**：只往下找一层，不处理 submodule。多仓库 Workspace 不能新建 Managed Worktree：一个任务在多个仓库各开一个 worktree，会改变“一个 Workspace 一个 Root Directory”的模型，等真实需求再议。

## 按摩擦记录再做

这些还没有足够的使用证据；出现两次以上真实摩擦再排。外部用户的 Issue 和单人 dogfood 一样算数。

- Diff Tab 内跳到编辑器。
- GUI 里启动 Agent 的菜单：列出这台 Device 上装了哪些 Agent，点一下就在 Pane 里启动。现在在终端里输入命令已经够快；从任务启动 Agent 由 ADR 0068 的 Start 承担。真要做时，用协议里现成的 `AgentCommand::Available` 和 `Start`。
- 定时任务：按 cron 启动新的 Agent，或定时给已有的 Agent 发提示。现有的 `agent start`、`agent prompt` 和 `agent wait` 已经覆盖了大半。它会让 Condr 从看着 Agent 干活走向无人值守地派活，用户的期待也会跟着变，所以等有人提出需求再定。
- `observe` 与 `control` 两级 capability（观察者拿 Bootstrap 和视觉流，不能输入、改布局、读文件；配对时决定，`condr server clients` 可看可改；开 Stream，即转发与文件传输，归 `control`，ADR 0061）：触发条件是第二个人开始共用同一个 Server。之后再谈设备密钥进 OS Keychain、密钥轮换、审计日志。不做账号体系、不做公网 listener。

## 远期

会做，但都排在近期方向之后，且各自有启动条件。条件未到之前不写 ADR、不预留接口。

| 事项 | 启动条件 | 前置 |
| --- | --- | --- |
| 遥测（opt-in） | 有外部用户，且本地日志已不足以排障 | 诊断数据默认不含终端内容；先有本地日志再谈上报 |
| 团队协作、账号与 RBAC | 出现多人共用一个 Server 的真实需求 | observe/control capability（见摩擦记录）扩展为多用户；审计日志 |
| 替用户批准 Agent 的权限请求（现在只做"看见并跳转"） | 厂商开放外部审批接口；Claude Code 的远程审批仍是 [open feature request](https://github.com/anthropics/claude-code/issues/38299)，Codex 的审批只在它自己的 app-server 客户端里 | `Blocked` 与 `detail`（ADR 0024、ADR 0044） |
| 编辑器、内置浏览器等 IDE 化能力（任务看板见 ADR 0068 的 Tasks 模式） | dogfood 中反复出现"为了这件事必须离开 Condr" | 逐项立 ADR，不成套引入 |

不做的三件事：**Web 客户端**（Condr 是原生 GUI，不把控制面搬进浏览器；异地需求由 Peer-to-peer 和移动端 Companion 承接）；**自建对话或工具循环、绑定单一模型厂商**（Condr 编排原生 Agent CLI，不替代它；因此也不在界面上另做输入框、模型选择或语音对话，这些交给各个 CLI 自己）；**用户自定义 Agent 和插件 SDK**（ADR 0035：Agent 种类变化不频繁，官方支持常见的 CLI 就够了；每种集成都需要声明写不出来的 hook 代码和真实会话验证；未支持的 CLI 通过 Issue 或 PR 加入）。

## 节奏

- 日常修复合入 `main` 即进入次日 Nightly；需要固定版本时按 [Releases](releases.md) 打 `v*` 标签。
- 对外宣布之后正式版是否放慢，尚未决定。要权衡的是：每个正式版都会让 GUI 亮起更新提示，更新后还会提示重启旧版 Server，发得越勤，修复越快到用户手里，提示也越频繁。
- 首个稳定版（release gate）：定下协议兼容窗口，把协议升到 `2` 并定下两个最低版本，`WatchTerminals` 是第一个受门控的消息（ADR 0028、0041），Releases 里“不承诺跨 build 兼容”一起改；App Store 与 Google Play 随它上架。
- 每个近期方向落地前先写 ADR，落地时同步更新 CONTEXT.md 与 AGENTS.md，并从本文删去这一项。
- 本文每次重大方向变化时整体重写，不追加"已完成"段落；历史看 git log 和 ADR。

