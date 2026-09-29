# GPUI Kit 能力复用排查

调查日期：2026-09-28，基于提交 `96ec5ac`。对照 GPUI Kit 0.6.0（`gpui-base`、`gpui-component`、`gpui-kit-assets`）与 GPUI `gpui-pre` 0.3.3 源码，排查 `crates/condr-gui` 的依赖、手写 UI 组件和非 UI 工具函数。标「已复核」的条目对照过代码，其余来自子任务报告，动手前再确认。

结论：没有哪个依赖在重复 Kit 已有的能力；可换成 Kit 的是约 10 处手写代码，外加 5 个顺带发现的小问题。起因是误以为系统通知用了 `notify`：通知已走 Kit 的 `SystemNotification`，`notify` 是 condr-server 监听 Workspace 文件改动的库。

## 可直接换成 Kit/GPUI

这 4 处行为不变、改动小。

| 位置 | 现在的做法 | 换成 | 复核 |
| --- | --- | --- | --- |
| `crates/condr-gui/src/color_scheme.rs:84-102`（及 `:155` 的测试） | 自写 `Color` newtype 解析 `#rrggbb` / `0xrrggbb`，约 25 行 | `Colors` / `Ansi` 字段直接用 `Hsla`，它自带 `Deserialize`（`gpui-pre-0.3.3/src/color.rs:734`）。只解析内置配色，135 个文件共 2926 个颜色全是 `#rrggbb`；严格性测试 `colors_accept_both_alacritty_spellings_and_reject_the_rest` 要删。前提：不支持用户导入 Alacritty 配色，否则要保留 `0x` 写法 | 已复核 |
| `src/app/sidebar/icon.rs:45-46,110`、`crates/condr-gui/assets/icons/file-text.svg` | 重复定义 `CondrIconName::FileText`。`assets.rs` 没登记这个文件，运行时其实一直落到 Kit 的那份 | `changes.rs:771` 改用 `IconName::FileText`，删变体和 svg | 已复核 |
| `src/app/settings/server.rs:~575-676` | Settings「已配对设备」手搭表头和行，约 75 行 | Kit `Table` / `TableHeader` / `TableRow` / `TableCell`（`gpui-component-0.6.0/src/table/table.rs:41`），接受样式覆盖，rem 宽度可用 | 已复核现状 |
| `src/app/gui_state.rs:126-130,149`、`src/app/settings.rs:36-51` | 手动展开 `WindowBounds` 三种变体、手算居中和 clamp，约 10 行 | `WindowBounds::get_bounds()`、`Bounds::centered_at` / `center()`、`Size::min`、`Point::clamp`（`gpui-pre-0.3.3/src/platform.rs:2105`、`geometry.rs:367,582,879`） | 未复核 |

## 能换但有成本

价值最大的是 Workspace Tab 栏：换成 Kit 的 `TabBar` 能顺带修掉 Tab 多了点不到的问题。其余几项收益有限。

| 位置 | 现在的做法 | Kit 对应 | 代价 / 判断 |
| --- | --- | --- | --- |
| Workspace Tab 栏 `src/app/workspace.rs:89-318`，约 230 行 | 每个 Tab 是 ghost `Button`，带拖拽排序、悬停关闭、右键菜单。容器 `:280` 是没有横向滚动的 `h_flex().min_w_0()`，Tab 是 `flex_shrink_0` | `TabBar`（`tab/tab_bar.rs:40`，横向滚动、滚到当前 Tab、溢出菜单）+ `Tab`（`tab/tab.rs:397`，支持拖放和右键菜单）；Kit Dock 在 `dock/tab_panel.rs:468-600` 就是这样组合的 | 视觉上要把 ghost button 样式对齐 Kit 的 Tab 变体；标题栏里 `on_mouse_down` 阻止拖动窗口的处理要搬过去。溢出问题是读代码推断的，未实际运行验证 |
| Changes 树 `src/app/changes.rs:510-720`，约 210 行 | 分区标题 + 目录/文件行，手写 hover、选中、缩进，普通 `v_flex` 不虚拟化（最多 1000 条） | `Tree` / `TreeState`（`gpui-component-0.6.0/src/tree.rs:18`，`uniform_list` 虚拟化、方向键导航）+ `ListItem` | 数据一次性已知，很契合。Kit 把展开状态存在 `TreeItem` 里，要在 `set_items` 时从 `collapsed_change_dirs` 回灌，并通过 `TreeEvent` 同步回来 |
| Files 树 `src/app/files.rs:194-423`，约 230 行 | 行构建和 Changes 树几乎重复 | 同上 `Tree` | 不太适合：Kit 只把有子项的条目当目录，懒加载目录要塞占位子项，Loading/报错/截断行要伪造成条目；Kit 在 mouse-down 就选中；ADR 0018 已否决键盘导航。至少可以把两棵树里 4 个近乎相同的行构建函数合并 |
| `src/app/open_in.rs:413-416` `reveal()` | 自己起 `explorer.exe` / `open` / `xdg-open` | `App::open_with_system`（`gpui-pre-0.3.3/src/app.rs:1609`）；`reveal_path` 不等价，它是在父目录里选中该项 | 不划算：GPUI 的是发出即不管，失败只写日志，我们的报错 toast 会没有。文件管理器还要继续作为「Open in」菜单的 `OpenTarget` |
| 目录浏览器行 `src/app/directory_browser.rs:359-472` | 手写行样式 | `ListItem` | 行为本身合理（由对话框自己的 Input 驱动、接受手敲路径），只有行样式可换 |
| 对话框表单字段 `src/app/dialogs.rs:232-252`、`server_management/dialogs.rs:250-286`，约 40 行 | label + `Input` + 红色错误行 | `v_form` / `Field`（`form/form.rs:14`），错误放 `description_fn` | 收益低。Cancel/主按钮页脚得保留：Kit `Dialog` 不按 `button_props` 画按钮，只有 `AlertDialog` 会 |
| 收起的侧边栏 `src/app/sidebar.rs:743-807`，约 65 行 | `SidebarCollapsible::None` + 自写窄栏 | `SidebarCollapsible::Icon`（同样 48px，带动画） | 收益低：标题栏分段按同一宽度计算（`workspace.rs:540`），不会跟着动画走 |
| Hooks 状态文字 `src/app/settings/server.rs:840-867` | 彩色文字 | `Tag`，同文件 `:212` 的 Status 行已在用 | 仅为一致性 |
| 错误 toast 的 Copy 按钮 `src/app/dialogs.rs:30-38` | `Button` + `write_to_clipboard` | Kit `Clipboard` 元素，`settings/server.rs:528` 已在用 | 可选：会变成带勾选反馈的图标按钮，现在的文字「Copy」更贴 ADR 0020 |

## 有意自写，理由成立

下面这些查过，Kit/GPUI 没有对应能力或行为不同，不必再排查。

| 项 | 位置 | 为什么不用 Kit/GPUI |
| --- | --- | --- |
| `async-channel` | `src/app/connection.rs:71` | Kit 只 re-export `smol::channel` 的 `unbounded`（`gpui-base-0.6.0/src/async_util.rs:4`）；PTY 路径要 `bounded` 做背压。同一个 crate 同版本，无额外构建成本 |
| `gpui-fps` | `src/app/workspace.rs:821` | Kit 仓库出品但不 re-export；GPUI 自带的帧时间叠加层要 `profiler` feature，只显示帧时间 |
| `futures-lite` | `src/app/updates.rs:153-158` | 读 `AsyncBody` 要 `AsyncRead` trait，GPUI/Kit 未公开 re-export `futures`。`http_client::github::latest_github_release` 覆盖不了 nightly 通道 |
| `serde_json` | `src/app/updates.rs:151-155` | GPUI 只有 `#[doc(hidden)] gpui::private::serde_json` |
| `image` | `src/assets.rs:13-16`、`terminal_input/clipboard.rs:147-159` | `WindowOptions.icon` 的类型就是 `image::RgbaImage` 且未 re-export；GPUI 没有 PNG 编码器 |
| `resvg`（dev） | `src/app/file_icons.rs:175-178`、`examples/generate_icons.rs` | 测试可改用 `SvgRenderer::render_single_frame`，但图标生成器要多写 BGRA→RGBA 和编码，整体不划算 |
| `smol_str` | `src/terminal_element.rs:31`、`terminal_element/cache.rs:17` | 类型来自 condr-core 的 `TerminalCell`；`SharedString` 没有 `From<SmolStr>`，热路径逐格转换没意义。想去掉直接依赖可让 condr-core re-export |
| `toml` / `toml_edit` / `prost` | `src/app/config.rs`、`color_scheme.rs`、`gui_state.rs` | Kit 无对应 |
| `keepawake` | `src/app/power.rs:30-36` | GPUI `Platform` 没有防休眠接口 |
| `windows-registry` | `src/app/startup.rs:320-335` | GPUI 注册通知 AUMID 时只写 `DisplayName`，我们补 `IconUri`；`SystemNotification` 没有图标字段 |
| Settings `TextField` | `src/app/settings/text_field.rs` | Kit `SettingField::input` 每次按键就保存，我们要失焦/回车才提交（commit ffc4d0e） |
| 字号步进器 | Settings 外观页 | Kit 数字输入会改写打一半的值 |
| 侧边栏条目 `CondrSidebarTreeItem` / `CondrSidebarSection` | `src/app/sidebar/item.rs`，约 600 行 | Kit `SidebarMenuItem` 缺：任意元素作图标、详情行、外部受控的展开状态、拖放、悬停操作菜单、双击切换。适合回馈给 Kit |
| 方形头像、Agent 状态角标 | `src/app/sidebar/icon.rs:342-376` | Kit `Avatar` 是圆形、两个字母、按首字母取色；我们按根路径取色，改名不变 |
| 侧边栏调宽手柄、Dock 落点指示、拖拽排序辅助 | — | Kit resizable 会按比例缩放且 `resize_handle` 是 `#[doc(hidden)]`；落点指示是 crate 私有；Dock 外没有可排序列表 |
| `apca.rs`、`terminal_element/colors.rs` | 对比度计算 | Kit `Colorize` 只有 lighten/darken/mix，没有对比度或可读前景色 |
| `file_icons.rs` | 文件类型图标 | Kit 不带文件类型图标 |
| `ime.rs:18-49` UTF-16 边界处理 | 约 35 行 | Kit `RopeExt` 遇到代理对中间向下取整，我们末尾有意向上取整，有测试锁定 |
| `terminal_element/input.rs` | 输入包装 | 只为让 `prefers_ime_for_printable_keys` 返回 `false`，GPUI 的 `ElementInputHandler` 绑定在 `accepts_text_input` 上 |
| `directory_browser.rs` 子序列匹配 | — | Kit 没有公开的模糊匹配 |
| Diff 视图 | — | Kit 没有 diff 组件，ADR 0017 选了只读 Editor |

## 顺带发现的问题

前两条是实际 bug。

| 问题 | 位置 | 说明 | 复核 |
| --- | --- | --- | --- |
| 两处报错 toast 没有 Copy 按钮 | `src/app/files.rs:583`、`src/app/open_in.rs:632` | 「Couldn't open … in …」直接调用 `Notification::error`，没走 `Condr::report_error`，违反 ADR 0020 | 已复核 |
| `CONDR_ICON_PATHS` 漏了 `icons/waypoints.svg` | `src/assets.rs:27` | `load()` 在 `:105` 能加载它，但 `list()` 列不出来；测试只校验「列出的都能加载」这一个方向。改成一张 `&[(&str, &[u8])]` 表同时驱动 `load` 和 `list`，约 130 行可降到 40 行 | 已复核 |
| AGENTS.md 依赖规则没列 `gpui-fps` | `AGENTS.md`（Dependencies 一条） | 只列了 `gpui-pre-reqwest-client` 一个例外，但 `gpui-fps` 也是直接声明的 | 已复核 |
| 两个 `sysinfo` 版本 | 根 `Cargo.toml` | `gpui-fps` 带进 `sysinfo 0.37.2`，condr-core 用 `0.31.4`，构建时两个版本都要编译。升级 condr-core 的 `sysinfo` 到 0.37 可合并 | 未复核 |
| 文档注释错位 | `src/app/open_in.rs:449-455` | `project_root` 的文档注释跑到了 `OPEN_IN_FEEDBACK` 常量上 | 未复核 |

## 跟进清单

第一批：低风险，行为不变或是明确的 bug

- [ ] 两处报错 toast 改走 `report_error`（`files.rs:583`、`open_in.rs:632`）
- [ ] `CONDR_ICON_PATHS` 补上 `waypoints.svg`，或改成一张表同时驱动 `load` / `list`
- [ ] 删 `CondrIconName::FileText` 和 `assets/icons/file-text.svg`，改用 `IconName::FileText`
- [ ] `color_scheme.rs` 的 `Color` 换成 `Hsla`（先确认不支持用户导入配色）
- [x] 「已配对设备」换成 Kit `Table`（不换：几行设备不值一个 `TableDelegate`，改成 `SettingGroup` 每设备一行）
- [ ] 窗口几何换成 GPUI 的 `Bounds` / `Size` / `Point` 方法（先复核）
- [ ] AGENTS.md 把 `gpui-fps` 列为例外
- [ ] 修 `open_in.rs:449-455` 错位的文档注释（先复核）

第二批：改动较大，需要 Windows 手动验收

- [x] 先在 Windows 上复现 Tab 过多被裁掉，再把 Workspace Tab 栏换成 Kit `TabBar` / `Tab`（不换：Kit 的 Tab 变体都自带边框或底色，对不上标题栏里的 ghost 样式。Tab 行改为可横向滚动，激活的 Tab 滚入视野）
- [ ] Changes 树换成 Kit `Tree`，处理折叠状态同步
- [ ] 合并 Changes / Files 两棵树重复的行构建函数

可选：收益低

- [ ] 升级 condr-core 的 `sysinfo` 到 0.37，去掉重复版本
- [ ] Hooks 状态改用 `Tag`
- [ ] 对话框字段改用 `v_form` / `Field`、目录浏览器行改用 `ListItem`
- [ ] 把侧边栏条目的扩展能力回馈给 GPUI Kit（任意图标元素、受控展开、拖放）
