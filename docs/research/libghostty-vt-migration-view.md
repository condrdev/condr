# libghostty-vt 迁移评估：网格状态与视图

调查日期：2026-10-28，基于 Condr 提交 `afcd14a`、ghostty 提交 `46e9dc9a`（main，2026-10-28）。libghostty-vt 的 C API 头文件写 `render.h:行`（`include/ghostty/vt/` 下）；要看行为细节时引用同一提交的 Zig 源码（`src/terminal/Terminal.zig` 等）；alacritty 引用 `alacritty_terminal` 0.26.0 源码。Condr 路径省略 `crates/condr-core/src/` 前缀，其他 crate 写全。行数是区间内的物理行（含注释和空行），不含测试。Rust 绑定 `libghostty-vt` 0.2.2 落后于 main，缺口列在「风险与未决」。

结论：libghostty-vt 的 render state、终端自持的选区和 formatter 能全部或大部分接管视口读取、选区跟踪和文本导出，约 427 行；damage 出帧、颜色和 cell 映射约 259 行只能部分接管；帧协议、线格式、每客户端 baseline 和 URL 检测约 1710 行不动，URL 检测接不过去是因为 libghostty-vt 的搜索只做子串匹配。换完后最值得做的新功能是 scrollback 搜索、OSC 133 提示符导航、mode 2027 字形簇和按行 id 的滚动增量帧。代价有三：选区语义要改（resize 和清屏不再清选区，选区变化会让 render state 整屏重建）；同步输出（2026）的 hold 和超时要自己做；0.2.2 绑定缺搜索、行 id、render hold 等 API。

## 接管清单

| 实现 | 位置 | 非测试行数 | libghostty-vt API | 结论 | 迁移后剩下 / 语义差异 |
| --- | --- | --- | --- | --- | --- |
| Term 持有、配置与喂字节 | `terminal/runtime.rs:160-163,525-536,732-744`；`terminal/pty_io.rs:3,38,75` | 32 | `ghostty_terminal_new`（terminal.h:3231）、`ghostty_terminal_vt_write`（terminal.h:3342）、`OPT_SCROLLBACK_MAX_LINES`（terminal.h:2502） | 大部分接管 | 留下 `Arc<Mutex<…>>` 的持有方式（libghostty-vt 同样要求调用方串行化）和 Condr 的 OSC 扫描器（另一块）。`terminal_config` 的三项各有去处：OSC 52 改成剪贴板回调（另一块）；`semantic_escape_chars`（runtime.rs:738）改成选词的 `boundary_codepoints`（selection.h:159-163）；kitty 键盘归键盘那一块。alacritty 默认的 10000 行 scrollback 要显式设置 |
| 视口滚动与回到底部 | `terminal/runtime.rs:581-596,639-646` | 24 | `ghostty_terminal_scroll_viewport`（terminal.h:3585；TOP/BOTTOM/DELTA 见 terminal.h:297-305）、`DATA_VIEWPORT_ACTIVE`（terminal.h:3093）、`DATA_SCROLLBAR`（terminal.h:652-660,2870） | 大部分接管 | 留下 `TerminalScroll` 到 libghostty-vt 的映射。libghostty-vt 没有 PageUp/PageDown，要按行数自己换算；delta 方向相反（libghostty-vt 向上为负，terminal.h:342）。"有没有真滚动"改成比较前后 scrollbar offset，`display_offset` 由 `total - len - offset` 推出 |
| resize 的 VT 一侧 | `terminal/resize.rs:101-116`；`terminal/view/snapshot.rs:400-423` | 40 | `ghostty_terminal_resize(cols, rows, cell_w, cell_h)`（terminal.h:3264-3293） | 大部分接管 | 给 alacritty 的 `Dimensions`/`WindowSize` 适配整段删掉。libghostty-vt 的 resize 还会结束 2026 同步输出，并在 mode 2048 下发带内尺寸报告（terminal.h:3273-3277）；它不清选区（见下文 ADR 0008 一节）。ConPTY 要设 `OPT_RESIZE_PULL_SCROLLBACK=false`（terminal.h:2661-2684） |
| resize 合并、PTY 重试与 `PtySize` | `terminal/resize.rs:1-99`；`terminal/view/snapshot.rs:425-434` | 109 | 无（PTY 侧） | 不接管 | 原样保留 |
| 视口快照遍历 | `terminal/view/snapshot.rs:108-164` | 57 | render state 的行、格迭代器（render.h:1285-1314,1433-1560）；`ROW_DATA_RAW` 加 `ghostty_row_get(WRAP)`（render.h:935-936，screen.h:267） | 大部分接管 | 留下逐格拼 `Vec<TerminalCell>` 和合并检测到的链接。`WRAPLINE` 位要从行级 wrap 回填到行尾格。`end_update` 之后读格不再需要终端锁（render.h:42-52），读格可以挪到锁外 |
| cell 映射（样式到 flags、空白格） | `terminal/view/snapshot.rs:259-286,323-331` | 37 | `ROW_CELLS_DATA_STYLE` 得到 `GhosttyStyle`（render.h:1449-1450，style.h:94-108）；`ROW_CELLS_DATA_RAW` 加 `CELL_DATA_WIDE`（screen.h:102-115）；`CELL_DATA_HAS_HYPERLINK`（screen.h:196） | 部分接管 | libghostty-vt 给的是结构化样式，线上 `flags` 却是 alacritty 的位布局，GUI 和 Companion 都写死了这套位（见下文），所以映射函数要重写成"style 到 alacritty 位"。WIDE、SPACER_TAIL、SPACER_HEAD 分别对应 alacritty 第 5、6、10 位 |
| 字形簇文本 | `terminal/view/snapshot.rs:288-302` | 15 | `ROW_CELLS_DATA_GRAPHEMES_UTF8`（render.h:1494-1505） | 全部接管 | 只剩 256 字节上限（`terminal.rs:75`）的截断，要改成在 UTF-8 码点边界截 |
| 颜色解析（含 OSC 4/10/11 覆盖） | `terminal/view/snapshot.rs:333-352` | 20 | `GhosttyStyleColor` 的 NONE/PALETTE/RGB（style.h:49-80）；无字格的背景走 content tag（screen.h:80-93,212-226）；`DATA_COLOR_PALETTE` 与 `DATA_COLOR_PALETTE_DEFAULT`（terminal.h:238,242） | 部分接管 | libghostty-vt 没有 named 色：NONE 要映射回 `Named(256/257)`，PALETTE 一律成 `Indexed`。GUI 对 `Named(0..15)` 和 `Indexed(0..15)` 处理相同（`crates/condr-gui/src/terminal_element/colors.rs:103-111,116-122`），不丢信息。OSC 覆盖要自己比较当前调色板和默认调色板，被改过的条目发 `Rgb`。render state 的 `FG_COLOR`/`BG_COLOR`（render.h:1461-1475）已经解析成 RGB，用了会让客户端自己的配色失效，不能直接用 |
| 光标读取 | `terminal/view/snapshot.rs:304-321,354-364`；`terminal/pty_io.rs:76-91` | 45 | `DATA_CURSOR` 结构（render.h:649-651,1023-1050）；每读一块 PTY 都要做的观测改用 `DATA_CURSOR_X/Y`（terminal.h:2817-2824），不必每块都更新 render state | 大部分接管 | 留下形状枚举映射：libghostty-vt 没有 Hidden，`visible=false` 映射成 Hidden。ConPTY 光标稳定（`cursor_settle.rs`）是 Condr 自有逻辑，不计入。libghostty-vt 多给一个 `wide_tail`（render.h:1036） |
| 鼠标跟踪模式投影 | `terminal/view/snapshot.rs:386-398` | 13 | `DATA_MOUSE_TRACKING` 只是布尔（terminal.h:2881-2889）；要细分就用 `DATA_MODE` 查 9/1000/1002/1003（terminal.h:3153，modes.h:66,75-77） | 部分接管 | 映射逻辑不变，只换取值来源 |
| damage 出帧与生产端 baseline | `terminal/view_source.rs:1-189` | 189 | render state 的 `begin_update`/`end_update`（render.h:1169-1186）、`DATA_DIRTY`（render.h:503-513,590-591）、`row_iterator_next_dirty`（render.h:1339-1341）、`render_state_clean`（render.h:1203） | 部分接管 | 换掉的是 `terminal.damage()`/`reset_damage()` 和按 `LineDamageBounds` 取格（view_source.rs:66-69,115-124,148-156,187）。留下的有：修订号闸门、光标稳定补修订（85-92）、检测链接变化时补 damage（125-140）、半屏阈值（164-166）和 baseline 记录。详见下文 |
| Server 每客户端 baseline、待发 Pane、批次 | `crates/condr-server/src/server/terminal_stream.rs:1-343` | 343 | 无 | 不接管 | render state 的更新会消费终端的脏状态（render.h:1117-1121），只能有一个生产者。ADR 0004 的每客户端 baseline 原样保留 |
| 帧类型、diff、校验、应用，选区与尺寸领域类型 | `terminal/view.rs:1-418` | 418 | 无 | 不接管 | 协议领域类型，没有 `runtime` feature 的客户端也在用。`TerminalSelection::selected_cell_range`（view.rs:332-365）还要给 Server 用，把半格 side 换成 libghostty-vt 的闭区间 |
| 线格式编解码（含 OSC 8 intern 与预算） | `terminal/view/wire.rs:1-706` | 706 | 无 | 不接管 | 只有新增样式列时要动（见新功能清单） |
| 视口内选区投影 | `terminal/view/snapshot.rs:166-224` | 59 | 行内选区 `ROW_DATA_SELECTION`（render.h:944-945,1000-1009）；是否有选区 `DATA_SELECTION`（terminal.h:3070-3083） | 大部分接管 | 留下：从首尾有选区的行拼出 `TerminalSelection`，以及"选区整个在视口外时报 `row == size.rows`"的哨兵（snapshot.rs:166-194）。起点落在宽字符尾格的情况 libghostty-vt 已经处理（render.zig:1380-1386） |
| 选区命令：Select、SelectAt、Copy、输入时清除 | `terminal/runtime.rs:303-309,598-637,746-752`；`terminal/view/snapshot.rs:366-384` | 73 | `OPT_SELECTION`（terminal.h:2392-2405）、`ghostty_terminal_select_word`/`select_line`（selection.h:771-774,826-829）、`ghostty_terminal_selection_format_alloc`（selection.h:932-937，复制语义见 selection.h:229-231）、`ghostty_terminal_grid_ref`（terminal.h:3721-3723） | 大部分接管 | 留下三件事：把视口坐标加客户端的 `display_offset` 换成 SCREEN 坐标；把半格 side 换成闭区间；"输入就清选区"的策略（runtime.rs:273-309）。语义差异见下文 ADR 0008 一节 |
| URL 检测（`RegexSearch`/`RegexIter` 加尾部修剪） | `terminal/view/snapshot.rs:12-106` | 95 | 没有对应：libghostty-vt 搜索只做子串匹配（search.h:33-35,248-252），头文件里没有正则或 URL 检测 | 不接管 | `detect_links`（26-79）要重写：从 render state 的行拼出视口文本和格索引映射，用 `ROW_DATA_WRAP`/`WRAP_CONTINUATION`（screen.h:267-274）跨软换行连接，再交给正则库。`trimmed_url_len`（81-106）原样保留。正则库现在经 alacritty 间接引入，迁移后要直接依赖（`regex-automata` 仍被 `tracing-subscriber` 等引入，不增加构建量） |
| OSC 8 快照 intern 与预算 | `terminal/view/snapshot.rs:226-257` | 32 | URI 只能经 `ghostty_grid_ref_hyperlink_uri` 取（grid_ref.h:167-191）；格和行上只有布尔标记（screen.h:196,295） | 不接管 | 预算逻辑不变。按指针去重（234-238）依赖 alacritty 共享的 `Arc` 字符串，libghostty-vt 每次把 URI 复制进调用方缓冲，要改成按值去重。grid ref 不适合放进渲染循环（grid_ref.h:28-30），只对带标记的格调用。头文件里没有暴露超链接 id |
| 纯文本导出 `visible_text`、`recent_text` | `terminal/runtime.rs:484-523`；`terminal/probes.rs:230-271` | 82 | formatter 的 PLAIN 加 trim（formatter.h:100-123,139-218），或 `selection_format_alloc` 配一个用 grid ref 构造的选区（selection.h:932-937） | 大部分接管 | 留下的是算范围（最后 N 个非空行）。formatter 总会去掉尾部空行（formatter.h:110-115）。隐藏（SGR 8）格在 PLAIN 输出里是不是空格：未确认（Condr 现在输出空格，probes.rs:260-261）。`visible_text` 只有测试在用 |
| 修订号发布 | `terminal/view/snapshot.rs:4-10` | 7 | 无 | 不接管 | Condr 自有 |

合计：全部或大部分接管 427 行，部分接管 259 行，不接管 1710 行，共 2396 行。前两类迁移后仍要留少量适配代码（坐标换算、枚举映射），427 是可被 libghostty-vt 调用替换的实现规模，不是净删除行数。

### RenderState 能否替代 `TermDamage` 加 baseline

能替代生产端的 `TermDamage`，不能替代 baseline。

- 粒度更粗。alacritty 给的是每行的列区间（view_source.rs:116-124 用了 left/right），libghostty-vt 只有行级脏标记（render.h:932-933）。delta 会从"改动的列段"变成"整行"。AGENTS.md 要求小变化不放大，所以 Condr 要把脏行和上一帧的保留视图逐格比较后再出 run，view.rs:185-235 里已经有同样的比较逻辑。
- 整屏重建的触发条件：换屏、调色板或清屏等终端级脏位、选区变化、尺寸变化、视口起点变化（render.zig:980-1022；Screen.zig:89-97）。普通输出把活动区往上推时视口起点也会变，所以连续滚屏输出每帧都是 FULL，render.h:179-182 也写了滚动后所有行标脏。alacritty 同样把滚动标成整屏 damage（alacritty term/mod.rs:789），这一点和现在一样。
- 选区变化会整屏重建。libghostty-vt 设置或清除选区都会置 `Screen.dirty.selection`（Screen.zig:3097,3104），下一次 update 整屏重建（render.zig:993-998）。Condr 现在把选区当元数据，alacritty 不为选区产生 damage，选择只发一个元数据 delta，符合 ADR 0008 "Selection changes ride the existing visual stream and do not invalidate shaping caches"。迁移后遇到 FULL 要先和保留视图比较（`frame_from`），否则每次选择都发整帧。
- 只能有一个消费者。update 会"消费终端和屏幕的脏状态"（render.h:1117-1121），脏标记要由调用方清（render.h:65-69），所以不能给每个客户端各开一个 render state。ADR 0004 的结构（生产端一个 baseline，Server 每客户端各一个 baseline）保持不变。
- 修订号、光标稳定，以及"文字没变但检测到的链接变了"时补 damage（view_source.rs:20-22,125-140），都是 Condr 的语义，libghostty-vt 不知道，继续保留。
- 收益有两项。两阶段 update（render.h:42-52）让读格发生在终端锁外，PTY 读线程被挡的时间变短。行 id（render.h:165-203）能把滚动表达成"行平移"，但要改协议（见新功能清单）。GUI 的塑形缓存按内容命中，同一行滚到别的位置也能命中（`crates/condr-gui/src/terminal_element/cache.rs:15`），所以行 id 省的是带宽和序列化，不是塑形。

### 选区能否满足 ADR 0008 的跟踪语义

ADR 0008 要求：Server 持有选区，选区随输出移动，resize 和切屏时清掉，键盘输入时丢弃，每帧带上裁剪到视口的选区。逐条对照：

| ADR 0008 语义 | alacritty（现状） | libghostty-vt | 迁移后 |
| --- | --- | --- | --- |
| Server 持有 | `Term.selection`（runtime.rs:613-618） | `OPT_SELECTION` 复制选区并转成终端持有的跟踪状态（terminal.h:2392-2405） | 等价 |
| 随输出移动 | `Selection::rotate`（alacritty term/mod.rs:686-689,752,778） | 跟踪 grid ref 随滚动、裁剪、resize 重排移动（grid_ref.h:46-58） | 等价。选区所在行被裁出 scrollback 后怎么样：未确认 |
| 切屏清掉 | `swap_alt` 置空（term/mod.rs:733） | `switchScreen` 清掉新屏的选区（Terminal.zig:4962-4963），进出 alt screen 两个方向都清 | 等价 |
| resize 清掉 | 列数变才清，只改行数时平移（term/mod.rs:680-689） | resize 路径里没有清选区（Terminal.zig:4151-4260），跟踪引用随重排移动 | 有差异：要么在 `resize_terminal_and_pty` 里显式清，要么改 ADR。ADR 0036 让最后获得焦点的窗口决定尺寸（ADR 0036 第 9 行），切窗口就会 resize，保留选区对多窗口更友好 |
| 清屏时 | 擦除区和选区相交就丢弃（term/mod.rs:1657,1773,1786） | 擦除路径不碰选区，Terminal.zig 全文只有 4963 一处 `clearSelection` | 有差异：`clear` 之后选区可能停在新文字上 |
| 键盘输入时丢弃 | Condr 自己做（runtime.rs:273-309） | 同 | 不变 |
| 每帧带视口裁剪后的选区 | `viewport_selection`（snapshot.rs:166-224） | 行内选区（render.h:944-945,1000-1009） | 大部分接管 |
| 双击选词 | Semantic 加自定分隔符（runtime.rs:738），点在括号上时选到配对括号（alacritty selection.rs:298-311） | `select_word` 加 `boundary_codepoints`（selection.h:142-164,771-774），跨软换行（Screen.zig:3480-3511） | 有差异：没有括号配对（头文件和 `selectWord` 都没有） |
| 三击选行 | Lines，沿软换行（alacritty term/search.rs:593-603） | `select_line`，沿软换行（Screen.zig:3211,3267），默认去掉首尾空白，可以按提示符边界截断（selection.h:193-218） | 等价，多了去空白 |
| 复制 | `selection_to_string`（runtime.rs:598-608） | `selection_format_alloc`，unwrap 加 trim 就是 Ghostty 的复制语义（selection.h:229-231） | 等价 |
| 半格 side | `Side`（snapshot.rs:379-384） | 端点是闭区间的格，没有 side（selection.h:76-78） | Server 先换算，可复用 view.rs:332-365 |

结论：满足 ADR 0008 的核心要求（Server 持有、随输出移动、切屏清除、复制），resize 和清屏这两条行为变了，ADR 0008 要修订。

### 搜索能否替代 `RegexSearch` / `RegexIter`

不能。Condr 只在 URL 检测里用正则搜索（prelude.rs:43，snapshot.rs:16-19,41-42）。libghostty-vt 的 `GhosttySearch` 是子串匹配：除 ASCII 字母不分大小写外，逐字节比较（search.h:33-35,248-252），没有正则选项。libghostty-vt 搜索的长处在别处：覆盖两块屏和 scrollback，随终端变化增量更新，每个匹配都是一个选区（search.h:21-30,57-66）。它适合做给用户用的搜索（见新功能清单），不适合做链接检测。

### `TerminalView` 的 cell 格式装不装得下

- 字形簇：装得下。cell 文本是 UTF-8 串，上限 256 字节（terminal.rs:75，wire.rs:362-364）；宽字符和两种 spacer 已经有位（alacritty 第 5、6、10 位）。
- 下划线样式：装得下。单线、双线、波浪、点线、虚线已经占了 alacritty 第 3、11-14 位，和 libghostty-vt 的 `GHOSTTY_SGR_UNDERLINE_*`（sgr.h:102-107）一一对应。GUI 只画单线和波浪线（`crates/condr-gui/src/terminal_element.rs:316-320`），这是 GPUI `UnderlineStyle` 的限制，不是协议的。
- 下划线颜色、上划线、闪烁：装不下。领域类型的 `flags: u16`（view.rs:90）16 位已经用满，第 15 位给了 `DETECTED_LINK_FLAG`，注释写的是"alacritty 留下的唯一空位"（view.rs:415-418）。线上的 `flags` 列是 `uint32`（`proto/condr/v1/terminal.proto:71`），但解码时 `narrow` 到 u16，超出就报 malformed（wire.rs:380-383，protocol/pb.rs:79-81），老客户端会拒收整帧，所以只能加新列。下划线颜色 alacritty 本来就有（alacritty term/cell.rs:197），是 Condr 自己没传。
- 颜色：`TerminalColor::Named(u16)` 用的是 alacritty `NamedColor` 的编号（256 前景、257 背景、258 光标、259-268 dim），GUI 和 Companion 都按这套编号解（colors.rs:103-111，`crates/condr-mobile/src/views.rs:258-266`）。libghostty-vt 没有 named 色，Server 端映射即可，协议不用改。

### GUI 有没有为 alacritty 绕路

渲染上没找到绕路。和 alacritty 绑死的是协议：cell `flags` 是 alacritty 的位布局，GUI（terminal_element.rs:42-52）和 Companion（`crates/condr-mobile/src/views.rs:197-205`，注释写着"The cell flags the Server sends: alacritty's bits"）各写死一份；颜色用 alacritty 的 named 编号。iOS 把格重新编码成转义序列喂给 SwiftTerm（ADR 0048 第 7 行），宽度由 SwiftTerm 自己再算一遍。另有两处看起来像绕路，但原因不在 alacritty：光标按字形实际宽度加宽（terminal_element.rs:385-411），alacritty 其实有 WIDE_CHAR 位，GUI 没用；Windows 上文字呈现的 emoji 改用 Segoe UI Symbol（`crates/condr-gui/src/terminal_element/cache.rs:131-147`），是字体问题。

## 新功能清单

| 功能 | libghostty-vt API | Condr 现状 | 要动的层 | 价值（对多 agent CLI 场景） |
| --- | --- | --- | --- | --- |
| scrollback 搜索（子串） | `GhosttySearch`：needle、tick/feed、视口内匹配、选中上一个或下一个并滚动（search.h:21-35,176-239,247-300） | 没有：GUI 终端代码（`terminal_element*`、`app/terminal_input`）里搜 `search` 无结果；正则只用于 URL（snapshot.rs:16-19） | Server：每个 Pane、每个客户端一个 search（libghostty-vt 允许多个共存，search.h:79-81），feed 要在终端锁内做。协议：新命令（设 needle、上一个、下一个），帧带视口内匹配和"第 k 个，共 n 个"。GUI：查找栏和高亮。Companion：可选 | 高：在 agent 的长输出里找报错和文件名。但 alt screen 没有 scrollback（Terminal.zig:4940），全屏 agent 只能搜当前屏。Claude Code 2.1 的全屏 UI 用 `?1049h`（项目记忆，本次未复核）；Codex 是否用 alt screen：未确认 |
| OSC 133：跳到上一个或下一个提示符、选中一条命令的输出、命令退出码 | `SCROLL_VIEWPORT_DELTA_PROMPT`（terminal.h:323-332）、`ghostty_terminal_select_output`（selection.h:849-867）、手势 `BEHAVIOR_OUTPUT`（selection.h:359-360）、cell 和行的语义（screen.h:117-135,230-248）、`DATA_CURSOR_AT_PROMPT`（terminal.h:3170-3179）、`OPT_SEMANTIC_PROMPT` 回调带退出码和命令行（terminal.h:1700-1790,2705） | 只截获 `133;A` 给 agent 状态用（osc.rs:29-33,310-311），VT 收到后丢弃（ADR 0044 第 11 行）。不注入 shell 集成：`crates/condr-core/src` 里搜 `ZDOTDIR`、`PROMPT_COMMAND`、`shell integration` 均无结果 | Server：注入 shell 集成脚本（libghostty-vt 不管这部分），接回调。协议：`TerminalScroll` 加按提示符跳，`TerminalSelectionUnit` 加 Output，退出码可作为事件。GUI：快捷键和点选。CLI：例如 `condr pane read` 只取上一条命令的输出 | 中：shell Pane 和编排受益，编排用的 agent 可以只读上一条命令的输出。agent 自己的 TUI 多在 alt screen，`CURSOR_AT_PROMPT` 在 alt screen 恒为假（terminal.h:3176-3178）。agent CLI 会不会发 OSC 133：未确认 |
| 字形簇与 mode 2027 | `GHOSTTY_MODE_GRAPHEME_CLUSTER`（modes.h:93）、`ROW_CELLS_DATA_GRAPHEMES_UTF8`（render.h:1494-1505）、`OPT_MODE_DEFAULT` 可以默认打开（terminal.h:2552-2566）、`ghostty_unicode_grapheme_width`（unicode.h:120-146） | 零宽码点已经并进前一格（snapshot.rs:288-302），但宽度按单个码点算，不支持 2027：`crates/condr-core/src` 里搜 `2027`、`grapheme` 无非测试结果 | Server：打开 2027，读字形簇。协议：不动（cell 文本上限 256 字节）。GUI：预计不动（按格塑形）。Companion：Android 自己画格不受影响；iOS 由 SwiftTerm 重算宽度，可能对不齐（ADR 0048 第 7 行） | 中：agent 输出里的 emoji 和带 VS16 的符号能对齐。2027 能不能设成 reset 默认值：未确认（terminal.h:2558-2560 说部分模式不行） |
| formatter 导出纯文本、VT、HTML | `GhosttyFormatter`（formatter.h:100-123,139-218；格式见 types.h:256-266）；`selection_format_*` 可选输出格式（selection.h:235-262,897-937） | 复制只出纯文本（runtime.rs:598-608；GUI 用 `ClipboardItem::new_string`，`crates/condr-gui/src/app/terminal_input/clipboard.rs:74`）。`condr pane read` 也只出纯文本（`crates/condr-server/src/server/client.rs:701` 到 probes.rs:230） | Server。协议：`Copy` 和读 Pane 文本的消息加格式。GUI："复制为 HTML"（GPUI 剪贴板能不能放 HTML：未确认）。CLI：`pane read --format` | 中低：编排 agent 能读到带颜色的输出（比如 diff 的红绿），也方便分享会话片段 |
| 双击后拖动按词扩展 | `ghostty_terminal_select_word_between`（selection.h:776-808） | 双击、三击只发一次 `SelectAt`，之后拖动不扩展（`crates/condr-gui/src/app/terminal_input/selection.rs:17-30`） | Server。协议：`SelectAt` 加终点，或加新命令。GUI：松手时发送。Companion：不涉及 | 低 |
| 下划线颜色、上划线、闪烁文字 | `GhosttyStyle` 的 `underline_color`、`overline`、`blink`（style.h:94-108） | `TerminalCell` 没有下划线颜色（view.rs:85-93），flags 已满（view.rs:415-418）；`crates/condr-core/src/terminal` 和 `proto` 里搜 `overline`、`underline_color` 无结果 | Server。协议：`TerminalCells` 加列。GUI：画上划线、驱动闪烁相位。Companion：可选 | 低：agent CLI 用不用这些样式：未确认 |
| 按行 id 的滚动增量帧 | `ROW_DATA_ID`（render.h:165-203,970-973） | 滚动就发整帧：`display_offset` 变化强制 Full（view_source.rs:99-101），差异过半也转 Full（view.rs:222-224）；`TerminalViewDelta` 没有行平移操作（terminal.proto:89-100） | Server：按行 id 比对。协议：delta 加"平移 N 行"。GUI 和 Companion：应用平移 | 中高：流式输出时，远程连接（TCP、iroh、手机）的带宽明显下降。AGENTS.md 已经预留了"全帧成为瓶颈时再做 damage delta"。0.2.2 绑定没有这个 API |
| 同步输出（2026）的 render hold 加超时 | `OPT_RENDER_HOLD`（terminal.h:2696），用法和"自己超时结束 hold"见 render.h:76-95 | 依赖 vte 的 `Processor` 在 2026 期间缓冲字节（vte ansi.rs:298-311）。Condr 不调 `stop_sync`（`crates/condr-core/src` 里搜 `stop_sync`、`sync_timeout` 无结果），程序漏发结束序列时，只有 2 MiB 上限兜底（vte ansi.rs:374-377） | 只动 Server：hold 期间 `take_frame` 不出帧，另加约 1 秒的超时 | 中：防撕裂，防 Pane 卡住。libghostty-vt 不在解析器里缓冲，迁移时这一项必须做 |
| 带内尺寸报告（mode 2048）与 ConPTY 不回拉 scrollback | `GHOSTTY_MODE_IN_BAND_RESIZE`（modes.h:96），resize 时自动发报告（terminal.h:3273-3275）；`OPT_RESIZE_PULL_SCROLLBACK`（terminal.h:2661-2684） | 都没有：`crates/condr-core/src/terminal` 里搜 `2048`、`RESIZE_PULL`、`reflow` 无非测试结果 | 只动 Server | 中：Windows 上 resize 后 VT 和 ConPTY 的屏幕保持一致（头文件点名 ConPTY 是这个选项的动机）；2048 对用它的 TUI 有用 |
| Kitty 图形 | render state 的图片迭代器（render.h:205-473），`kitty_graphics.h` | 没有；终端契约研究把它列为后置项（docs/research/terminal-contract.md:41） | 四层都要动，另需图片传输通道 | 低：主流 agent CLI 会不会输出 Kitty 图形：未确认 |
| overscan 平滑滚动 | `OPT_OVERSCAN`（render.h:97-163,701-708） | Server 按整行滚动（runtime.rs:581-596） | Server、协议、GUI | 低 |

## 风险与未决

- Rust 绑定落后。0.2.2 依赖的 `libghostty-vt-sys` 钉在 ghostty `a887df42`（2026-07-11，sys 的 build.rs:5-7）。比 main 少的有：搜索（crate 里没有 search 模块，sys 绑定里也没有 `ghostty_search_new`）、行 id、overscan、按提示符滚动、`CURSOR_AT_PROMPT`，以及 semantic prompt、render hold、reset 三个回调。安全 API 没有读取当前选区的 getter，sys 里有 `TerminalData::SELECTION = 31`（bindings.rs:2463）。已经有的：render state 的行内选区和 `is_selected`（render.rs:641,910）、`graphemes_utf8`（render.rs:840）、含下划线颜色和上划线的样式（style.rs）、`hyperlink_uri`（screen.rs:116）、`track_grid_ref`（terminal.rs:402）、`set_selection` 和各种 select、选区格式化（selection.rs:210-404）、带选区的 formatter（fmt.rs:64）。缺的部分要么自己维护 sys 绑定，要么向上游提 PR。
- 构建链。sys crate 构建时用 zig 从源码编 ghostty（build.rs:112-161），并从 GitHub 拉指定提交（`GHOSTTY_SOURCE_DIR` 可以指向本地源码）。Condr 在 Linux arm64、Windows、macOS 上的构建和交叉构建能不能都过：未确认。condr-mobile 不带 `runtime` feature（ADR 0039），手机端不链接 libghostty-vt。
- 选区语义变化（见 ADR 0008 一节）：resize 和清屏不再清选区；双击括号不再选到配对括号；选区所在行被裁出 scrollback 后的行为未确认。ADR 0008 需要修订。
- 选区变化会触发整屏重建（Screen.zig:89-97，render.zig:993-998）。如果 Condr 不先和保留视图比较，每次选择都会发整帧。
- 行级脏标记比 alacritty 的列区间粗，要逐格比较脏行，否则 spinner 这类单格变化会变成整行 run。AGENTS.md 要求的性能验收（突发合并到最新修订、末帧不丢、未变格不重新塑形、真实拖选）要在 Windows 上重跑。
- SCREEN 坐标查找可能遍历整个 scrollback（terminal.h:3703-3706）。选区命令把客户端视口坐标换成 SCREEN 坐标时，如果客户端的 `display_offset` 和当前一致，应优先用 VIEWPORT 坐标。
- 超链接 URI 只能逐格经 grid ref 取，而 grid ref 不是为渲染循环设计的（grid_ref.h:28-30，terminal.h:3708-3710）。满屏 OSC 8 时的开销：未测。
- 宽度表不同。alacritty 逐码点按 `unicode-width` 算，Ghostty 用自己的表（unicode.h:38-62）。现有断言宽度的测试和 GUI 列对齐可能变化：未实测。
- 协议兼容。cell `flags` 和颜色编号都是 alacritty 的内部编号；Companion 有独立的版本（ADR 0049），协议按区间兼容（ADR 0028）。所以迁移后 Server 要继续合成同样的位和编号，新样式只能加列。
- 2026 render hold：libghostty-vt 不缓冲字节，hold 期间照常出帧就会显示半帧，所以 Condr 必须实现 hold 加超时（render.h:76-95）。这也暴露了现状的隐患：vte 的缓冲没有超时，漏发 `?2026l` 的程序会让 Pane 冻结，直到攒满 2 MiB。
- libghostty-vt 搜索在 alt screen 下没有 scrollback 可搜（Terminal.zig:4940），对全屏 agent 的价值打折。
