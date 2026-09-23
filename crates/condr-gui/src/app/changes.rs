//! The Changes sidebar and the Diff Tab (ADR 0017): the presented Workspace's working-tree
//! changes as the Server reports them, and one file's diff in GPUI Kit's read-only Editor.
//! Everything shown here is Server data; the Client only asks and draws.

use super::sidebar::CondrIconName;
use super::*;
use condr_core::protocol::DiffBase;
use condr_core::{
    DiffLineKind, FileDiffContent, GitChangeEntry, GitChangeStatus, GitDiffStat, MAX_GIT_CHANGES,
};
use gpui_kit::component::input::{TextDecoration, TextDecorationCollection};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tab::{Tab, TabBar};

/// What the right sidebar shows (ADR 0018); remembered per Workspace (ADR 0023).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SidebarView {
    #[default]
    Changes,
    Files,
}

pub(super) const INITIAL_CHANGES_WIDTH: Pixels = px(300.);
pub(super) const MIN_CHANGES_WIDTH: Pixels = px(200.);
pub(super) const MAX_CHANGES_WIDTH: Pixels = px(480.);

/// Dragging the Changes sidebar's left edge resizes it.
pub(super) struct DraggedChanges;

/// The three sections of the list, in display order, as Zed's git panel groups them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum ChangesSection {
    Conflicts,
    Tracked,
    Untracked,
}

impl ChangesSection {
    const ORDER: [Self; 3] = [Self::Conflicts, Self::Tracked, Self::Untracked];

    fn of(status: GitChangeStatus) -> Self {
        match status {
            GitChangeStatus::Conflicted => Self::Conflicts,
            GitChangeStatus::Untracked => Self::Untracked,
            GitChangeStatus::Added
            | GitChangeStatus::Modified
            | GitChangeStatus::Deleted
            | GitChangeStatus::Renamed => Self::Tracked,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Conflicts => "Conflicts",
            Self::Tracked => "Tracked",
            Self::Untracked => "Untracked",
        }
    }

    fn selector(self) -> &'static str {
        match self {
            Self::Conflicts => "changes-conflicts",
            Self::Tracked => "changes-tracked",
            Self::Untracked => "changes-untracked",
        }
    }
}

/// The glyph and colour a status shows, as Zed draws them: a square with a plus, a dot or
/// a minus in the theme's success, warning and danger colours; a conflict warns.
pub(super) fn status_glyph(status: GitChangeStatus, cx: &App) -> Icon {
    let name = match status {
        GitChangeStatus::Added | GitChangeStatus::Untracked => Icon::new(CondrIconName::SquarePlus),
        GitChangeStatus::Modified | GitChangeStatus::Renamed => Icon::new(CondrIconName::SquareDot),
        GitChangeStatus::Deleted => Icon::new(CondrIconName::SquareMinus),
        GitChangeStatus::Conflicted => Icon::new(IconName::TriangleAlert),
    };
    name.size_4().text_color(status_color(status, cx))
}

/// The theme's version-control colour for a status: success, warning or danger.
pub(super) fn status_color(status: GitChangeStatus, cx: &App) -> Hsla {
    match status {
        GitChangeStatus::Added | GitChangeStatus::Untracked => cx.theme().success,
        GitChangeStatus::Modified | GitChangeStatus::Renamed => cx.theme().warning,
        GitChangeStatus::Deleted | GitChangeStatus::Conflicted => cx.theme().danger,
    }
}

/// `+N −N`, each half only when it is not zero.
pub(super) fn diff_stat(stat: GitDiffStat, cx: &App) -> AnyElement {
    h_flex()
        .flex_none()
        .gap_1()
        .text_xs()
        .when(stat.added > 0, |this| {
            this.child(
                div()
                    .text_color(cx.theme().success)
                    .child(format!("+{}", stat.added)),
            )
        })
        .when(stat.deleted > 0, |this| {
            this.child(
                div()
                    .text_color(cx.theme().danger)
                    .child(format!("−{}", stat.deleted)),
            )
        })
        .into_any_element()
}

/// The rows of one section as a directory tree, the way Zed's git panel shows them: a
/// directory that holds nothing but one directory folds into it (`src/app`), files sort after
/// directories, and both sort by name.
#[derive(Debug, PartialEq)]
enum ChangeNode<'a> {
    Directory {
        /// The folded segments joined with `/`.
        label: String,
        /// The directory's repository-relative path, the key of its collapsed state.
        path: RelativePathBuf,
        children: Vec<ChangeNode<'a>>,
    },
    File {
        name: String,
        entry: &'a GitChangeEntry,
    },
}

fn change_tree<'a>(entries: &[&'a GitChangeEntry]) -> Vec<ChangeNode<'a>> {
    #[derive(Default)]
    struct Builder<'a> {
        directories: std::collections::BTreeMap<String, Builder<'a>>,
        files: Vec<(String, &'a GitChangeEntry)>,
    }

    fn insert<'a>(builder: &mut Builder<'a>, components: &[String], entry: &'a GitChangeEntry) {
        match components {
            [] => {}
            [name] => builder.files.push((name.clone(), entry)),
            [directory, rest @ ..] => insert(
                builder.directories.entry(directory.clone()).or_default(),
                rest,
                entry,
            ),
        }
    }

    fn finish<'a>(builder: Builder<'a>, parent: &RelativePath) -> Vec<ChangeNode<'a>> {
        let mut nodes = Vec::new();
        for (name, mut child) in builder.directories {
            let mut label = name.clone();
            let mut path = parent.join(&name);
            // Fold a chain of lone directories into one row.
            while child.files.is_empty() && child.directories.len() == 1 {
                let (next_name, next) = child.directories.pop_first().expect("one entry");
                label.push('/');
                label.push_str(&next_name);
                path.push(&next_name);
                child = next;
            }
            nodes.push(ChangeNode::Directory {
                label,
                children: finish(child, &path),
                path,
            });
        }
        let mut files = builder.files;
        files.sort_by(|(a, _), (b, _)| a.cmp(b));
        nodes.extend(
            files
                .into_iter()
                .map(|(name, entry)| ChangeNode::File { name, entry }),
        );
        nodes
    }

    let mut root = Builder::default();
    for entry in entries {
        let components: Vec<String> = entry
            .path
            .components()
            .map(|component| component.as_str().to_owned())
            .collect();
        insert(&mut root, &components, entry);
    }
    finish(root, RelativePath::new(""))
}

/// What every row of one section's tree shares.
#[derive(Clone, Copy)]
struct TreeContext<'a> {
    key: ConnectionKey,
    workspace_id: WorkspaceId,
    /// The file the Diff Tab shows, drawn selected.
    shown: Option<&'a RelativePath>,
}

/// A tree row's left padding: the section gutter plus one indent per level. A file row
/// also skips the chevron slot a directory row starts with, so names line up; both scale
/// with the font since they frame text.
pub(super) fn tree_indent(depth: usize, past_chevron: bool) -> Rems {
    let chevron = if past_chevron { 1.125 } else { 0. };
    rems(0.5 + 0.875 * depth as f32 + chevron)
}

/// The Diff Tab's Editor and what it currently shows.
pub(super) struct DiffEditor {
    pub(super) state: Entity<EditorState>,
    decorations: TextDecorationCollection,
    /// The path and the connection's diff generation the Editor text was built from.
    shown: Option<(RelativePathBuf, u64)>,
    pub(super) content: DiffContent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DiffContent {
    Loading,
    Text {
        stat: GitDiffStat,
    },
    /// Both sides are identical: the file left the change list.
    Unchanged,
    Binary,
    TooLarge {
        bytes: u64,
    },
    Failed(String),
}

impl Condr {
    /// Each Workspace keeps its own right sidebar: opening it here leaves the others
    /// closed, and coming back finds it the way it was left.
    pub(super) fn toggle_changes_sidebar(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.presented_workspace() else {
            return;
        };
        if !self.changes_open.remove(&workspace) {
            self.changes_open.insert(workspace);
        }
        self.schedule_state_save(cx);
        cx.notify();
    }

    pub(super) fn changes_sidebar_open(&self) -> bool {
        self.presented_workspace()
            .is_some_and(|workspace| self.changes_open.contains(&workspace))
    }

    /// The Workspace presented for the sidebar to review. Without one the column stays
    /// hidden and its toggle disabled.
    pub(super) fn presented_workspace(&self) -> Option<(ConnectionKey, WorkspaceId)> {
        let connection = self.active_connection()?;
        let session = Session::restore(connection.snapshot.clone()).ok()?;
        let workspace_id = self.presented_workspace_id(connection.key, &session)?;
        Some((connection.key, workspace_id))
    }

    fn toggle_changes_section(&mut self, section: ChangesSection, cx: &mut Context<Self>) {
        if !self.collapsed_changes_sections.remove(&section) {
            self.collapsed_changes_sections.insert(section);
        }
        cx.notify();
    }

    /// Shows `path`'s diff in the Workspace's Diff Tab: a layout command like any other,
    /// presented once the Server confirms it.
    pub(super) fn show_diff_on(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: RelativePathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_presenting_layout_to(
            key,
            LayoutCommand::ShowDiff { workspace_id, path },
            window,
            cx,
        );
    }

    /// The title bar's toggle for the right sidebar, next to "Open in". Only a Workspace
    /// has anything to show, so the welcome page renders none.
    pub(super) fn render_changes_toggle(&self, cx: &mut Context<Self>) -> AnyElement {
        let owner = cx.weak_entity();
        let label = if self.changes_sidebar_open() {
            "Hide Changes & Files"
        } else {
            "Show Changes & Files"
        };
        let icon = if self.changes_sidebar_open() {
            IconName::PanelRightClose
        } else {
            IconName::PanelRightOpen
        };
        h_flex()
            .flex_none()
            .h_full()
            .items_center()
            .pl_2()
            // The same gutter the sidebar keeps before the window edge.
            .pr_3()
            // Inside the title bar an unclaimed press starts a window move on Windows.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                Button::new("toggle-changes")
                    .debug_selector(|| "toggle-changes".into())
                    .ghost()
                    .small()
                    .icon(Icon::new(icon))
                    .tooltip_with_action(label, &ToggleChanges, Some(SHORTCUT_CONTEXT))
                    .accessibility_label(label)
                    .on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| this.toggle_changes_sidebar(cx));
                    }),
            )
            .into_any_element()
    }

    /// The user's pick for this Workspace; a Workspace outside any repository opens on
    /// Files, since its Changes view has nothing to show (ADR 0018).
    fn sidebar_view_for(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        in_repository: bool,
    ) -> SidebarView {
        self.sidebar_views
            .get(&(key, workspace_id))
            .copied()
            .unwrap_or(if in_repository {
                SidebarView::Changes
            } else {
                SidebarView::Files
            })
    }

    fn set_sidebar_view(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        view: SidebarView,
        cx: &mut Context<Self>,
    ) {
        if self.sidebar_views.insert((key, workspace_id), view) != Some(view) {
            self.schedule_state_save(cx);
            cx.notify();
        }
    }

    /// The right sidebar: the presented Workspace's changes against `HEAD`, or its files
    /// (ADR 0018), chosen by the tabs in the header.
    pub(super) fn render_right_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let column = v_flex()
            .debug_selector(|| "condr-right-sidebar".into())
            .size_full()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_l_1()
            .border_color(theme.sidebar_border);

        let presented = self.active_connection().and_then(|connection| {
            let session = Session::restore(connection.snapshot.clone()).ok()?;
            let workspace_id = self.presented_workspace_id(connection.key, &session)?;
            let workspace = session.workspace(workspace_id)?;
            let presented_tab = self.presented_tab_id(connection.key, &session, workspace_id);
            let shown_diff = session
                .diff_tab(workspace_id)
                .filter(|tab| presented_tab == Some(tab.id()))
                .and_then(|tab| tab.diff().map(|diff| diff.path().to_relative_path_buf()));
            let shown_file = session
                .file_tab(workspace_id)
                .filter(|tab| presented_tab == Some(tab.id()))
                .and_then(|tab| tab.file().map(|file| file.path().to_relative_path_buf()));
            Some((
                connection.key,
                workspace_id,
                workspace.name().to_owned(),
                connection.workspace_git.get(&workspace_id).cloned(),
                shown_diff,
                shown_file,
            ))
        });
        let Some((key, workspace_id, workspace_name, git, shown_diff, shown_file)) = presented
        else {
            return column
                .child(self.sidebar_header(None, SidebarView::Changes, None, None, cx))
                .child(empty_state("No Workspace", cx))
                .into_any_element();
        };
        let view = self.sidebar_view_for(key, workspace_id, git.is_some());
        let count = git.as_ref().map(|git| git.changes.entries.len());
        let total = git.as_ref().and_then(|git| {
            git.changes
                .entries
                .iter()
                .filter_map(|entry| entry.stat)
                .reduce(|sum, stat| GitDiffStat {
                    added: sum.added + stat.added,
                    deleted: sum.deleted + stat.deleted,
                })
        });
        let header = self.sidebar_header(Some((key, workspace_id)), view, count, total, cx);
        let body = match view {
            SidebarView::Files => {
                self.render_files_list(key, workspace_id, shown_file.as_deref(), cx)
            }
            SidebarView::Changes => match git {
                Some(git) => self.render_changes_list(key, workspace_id, &git, shown_diff, cx),
                None => empty_state(format!("{workspace_name} is not a Git repository"), cx),
            },
        };
        column.child(header).child(body).into_any_element()
    }

    /// The Files / Changes tabs, sized to their labels, with the change count in the
    /// Changes tab and the summed `+N −N` at the far end whichever view is shown, the way
    /// Zed totals a branch.
    fn sidebar_header(
        &self,
        target: Option<(ConnectionKey, WorkspaceId)>,
        view: SidebarView,
        count: Option<usize>,
        total: Option<GitDiffStat>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // GPUI Kit's own tab strip, as the Kit gallery shows it; the count rides in the
        // Changes tab and the total takes the strip's free end.
        let owner = cx.weak_entity();
        let selected = match view {
            SidebarView::Files => 0,
            SidebarView::Changes => 1,
        };
        let stat = total.map(|total| div().pr_3().child(diff_stat(total, cx)));
        TabBar::new("condr-sidebar-view")
            .selected_index(selected)
            .on_click(move |index, _, cx| {
                let view = if *index == 1 {
                    SidebarView::Changes
                } else {
                    SidebarView::Files
                };
                let _ = owner.update(cx, |this, cx| {
                    if let Some((key, workspace_id)) = target {
                        this.set_sidebar_view(key, workspace_id, view, cx);
                    }
                });
            })
            .children([
                Tab::new()
                    .label("Files")
                    .debug_selector(|| "sidebar-view-files".into()),
                Tab::new()
                    .label(match count {
                        Some(count) => format!("Changes ({count})"),
                        None => "Changes".to_owned(),
                    })
                    .debug_selector(|| "sidebar-view-changes".into()),
            ])
            .when_some(stat, |this, stat| this.suffix(stat))
            .into_any_element()
    }

    /// The Changes view's body: the sections and their trees, or why there are none.
    fn render_changes_list(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        git: &WorkspaceGitSnapshot,
        shown: Option<RelativePathBuf>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Copied out so the rows below can take `cx` again.
        let theme = cx.theme();
        let (radius, accent, muted, warning) = (
            theme.radius,
            theme.sidebar_accent,
            theme.muted_foreground,
            theme.warning,
        );
        let entries = &git.changes.entries;
        if entries.is_empty() {
            return empty_state("No changes", cx);
        }

        let mut list = v_flex()
            .id("condr-changes-list")
            .flex_1()
            .min_h_0()
            .w_full()
            .px_1()
            .pb_1()
            .overflow_y_scrollbar();
        if git.changes.truncated {
            list = list.child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(warning)
                    .child(format!(
                        "Only the first {MAX_GIT_CHANGES} changes are listed; add a .gitignore for the rest."
                    )),
            );
        }
        for section in ChangesSection::ORDER {
            let rows: Vec<&GitChangeEntry> = entries
                .iter()
                .filter(|entry| ChangesSection::of(entry.status) == section)
                .collect();
            if rows.is_empty() {
                continue;
            }
            let collapsed = self.collapsed_changes_sections.contains(&section);
            let toggle_owner = cx.weak_entity();
            list = list.child(
                h_flex()
                    .id(section.selector())
                    .debug_selector(move || section.selector().into())
                    .h_7()
                    .w_full()
                    .px_1()
                    .gap_1()
                    .items_center()
                    .cursor_pointer()
                    .rounded(radius)
                    .hover(move |this| this.bg(accent.opacity(0.8)))
                    .on_click(move |_, _, cx| {
                        let _ = toggle_owner.update(cx, |this, cx| {
                            this.toggle_changes_section(section, cx);
                        });
                    })
                    .child(
                        Icon::new(IconName::ChevronRight)
                            .size_3p5()
                            .text_color(muted)
                            .when(!collapsed, |icon| icon.rotate(percentage(90. / 360.))),
                    )
                    .child(div().text_xs().text_color(muted).child(format!(
                        "{} ({})",
                        section.title(),
                        rows.len()
                    ))),
            );
            if collapsed {
                continue;
            }
            let tree = change_tree(&rows);
            let mut elements = Vec::new();
            let context = TreeContext {
                key,
                workspace_id,
                shown: shown.as_deref(),
            };
            self.render_change_tree(&context, &tree, 0, &mut elements, cx);
            list = list.children(elements);
        }
        list.into_any_element()
    }

    fn render_change_tree(
        &self,
        context: &TreeContext<'_>,
        nodes: &[ChangeNode],
        depth: usize,
        list: &mut Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) {
        for node in nodes {
            match node {
                ChangeNode::Directory {
                    label,
                    path,
                    children,
                } => {
                    let collapsed = self.collapsed_change_dirs.contains(&(
                        context.key,
                        context.workspace_id,
                        path.clone(),
                    ));
                    list.push(
                        self.render_change_directory(context, label, path, depth, collapsed, cx),
                    );
                    if !collapsed {
                        self.render_change_tree(context, children, depth + 1, list, cx);
                    }
                }
                ChangeNode::File { name, entry } => {
                    let selected = context.shown == Some(entry.path.as_relative_path());
                    list.push(self.render_change_file(context, name, entry, depth, selected, cx));
                }
            }
        }
    }

    fn render_change_directory(
        &self,
        context: &TreeContext<'_>,
        label: &str,
        path: &RelativePath,
        depth: usize,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (key, workspace_id) = (context.key, context.workspace_id);
        let theme = cx.theme();
        let path_text = path.to_string();
        let selector: SharedString = format!("change-dir-{path_text}").into();
        let id = selector.clone();
        let owner = cx.weak_entity();
        let toggle_path = path.to_relative_path_buf();
        h_flex()
            .id(id)
            .debug_selector(move || selector.to_string())
            .h_7()
            .w_full()
            .min_w_0()
            .pl(tree_indent(depth, false))
            .pr_2()
            .gap_1()
            .items_center()
            .cursor_pointer()
            .rounded(theme.radius)
            .text_sm()
            .hover(|this| this.bg(theme.sidebar_accent.opacity(0.8)))
            .on_click(move |_, _, cx| {
                let _ = owner.update(cx, |this, cx| {
                    let dir = (key, workspace_id, toggle_path.clone());
                    if !this.collapsed_change_dirs.remove(&dir) {
                        this.collapsed_change_dirs.insert(dir);
                    }
                    cx.notify();
                });
            })
            .child(
                Icon::new(IconName::ChevronRight)
                    .size_3p5()
                    .text_color(theme.muted_foreground)
                    .when(!collapsed, |icon| icon.rotate(percentage(90. / 360.))),
            )
            .child(
                Icon::new(IconName::Folder)
                    .size_4()
                    .text_color(theme.muted_foreground),
            )
            .child(div().min_w_0().flex_1().truncate().child(label.to_owned()))
            .into_any_element()
    }

    fn render_change_file(
        &self,
        context: &TreeContext<'_>,
        name: &str,
        entry: &GitChangeEntry,
        depth: usize,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let path_text = entry.path.to_string();
        let selector: SharedString = format!("change-{path_text}").into();
        let id = selector.clone();
        let deleted = entry.status == GitChangeStatus::Deleted;
        let owner = cx.weak_entity();
        let path = entry.path.clone();
        let (key, workspace_id) = (context.key, context.workspace_id);
        h_flex()
            .id(id)
            .debug_selector(move || selector.to_string())
            .h_7()
            .w_full()
            .min_w_0()
            .pl(tree_indent(depth, true))
            .pr_2()
            .gap_2()
            .items_center()
            .cursor_pointer()
            .rounded(theme.radius)
            .text_sm()
            .when(!selected, |this| {
                this.hover(|this| {
                    this.bg(theme.sidebar_accent.opacity(0.8))
                        .text_color(theme.sidebar_accent_foreground)
                })
            })
            .when(selected, |this| {
                this.bg(theme.tokens.sidebar_accent)
                    .text_color(theme.sidebar_accent_foreground)
            })
            .on_click(move |_, window, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.show_diff_on(key, workspace_id, path.clone(), window, cx);
                });
            })
            .context_menu(self.file_context_menu(
                key,
                workspace_id,
                &entry.path,
                Some(!deleted),
                cx,
            ))
            .child(status_glyph(entry.status, cx))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .when(deleted, |this| {
                        this.line_through().text_color(theme.muted_foreground)
                    })
                    .child(name.to_owned()),
            )
            .when_some(entry.stat, |this, stat| this.child(diff_stat(stat, cx)))
            .into_any_element()
    }

    /// The Diff Tab's body: one file's diff, or why there is none.
    pub(super) fn render_diff_tab(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        path: &RelativePath,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let path_text = path.to_string();
        let entry = self
            .connection(key)
            .and_then(|connection| connection.workspace_git.get(&workspace_id))
            .and_then(|git| git.changes.entries.iter().find(|entry| entry.path == path));
        let editor = self.diff_editors.get(&(key, tab_id));
        let content = editor.map_or(DiffContent::Loading, |editor| editor.content.clone());
        let header = h_flex()
            .debug_selector(|| "diff-header".into())
            .flex_none()
            .h_9()
            .w_full()
            .px_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .when_some(entry, |this, entry| {
                this.child(status_glyph(entry.status, cx))
            })
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_sm()
                    .font_medium()
                    .child(path_text.clone()),
            )
            .when_some(entry.and_then(|entry| entry.stat), |this, stat| {
                this.child(diff_stat(stat, cx))
            })
            .child({
                // A deleted file has nothing on disk to show.
                let deleted = entry.is_some_and(|entry| entry.status == GitChangeStatus::Deleted);
                let owner = cx.weak_entity();
                let path = path.to_relative_path_buf();
                Button::new("diff-show-file")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(CondrIconName::FileText))
                    .debug_selector(|| "diff-show-file".into())
                    .tooltip("Show File")
                    .accessibility_label("Show File")
                    .disabled(deleted)
                    .on_click(move |_, window, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            this.show_file_from_diff(
                                key,
                                workspace_id,
                                tab_id,
                                path.clone(),
                                window,
                                cx,
                            );
                        });
                    })
            });
        let body = match (&content, editor) {
            (DiffContent::Text { .. }, Some(editor)) => div()
                .debug_selector(|| "diff-body".into())
                .flex_1()
                .min_h_0()
                .w_full()
                .child(
                    Editor::new(&editor.state)
                        .readonly(true)
                        .appearance(false)
                        .bordered(false)
                        .font_family(self.terminal_font.family.clone())
                        .text_size(px(self.terminal_font.size))
                        .h(relative(1.)),
                )
                .into_any_element(),
            (DiffContent::Loading, _) => empty_state("Loading diff…", cx),
            (DiffContent::Unchanged, _) => empty_state(format!("No changes in {path_text}"), cx),
            (DiffContent::Binary, _) => empty_state("Binary file", cx),
            (DiffContent::TooLarge { bytes }, _) => empty_state(
                format!(
                    "Diff too large to show ({:.1} MiB)",
                    *bytes as f64 / (1024. * 1024.)
                ),
                cx,
            ),
            (DiffContent::Failed(reason), _) => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .text_sm()
                .text_color(theme.danger)
                .child(reason.clone())
                .into_any_element(),
            (DiffContent::Text { .. }, None) => empty_state("Loading diff…", cx),
        };
        v_flex()
            .debug_selector(|| "diff-tab".into())
            .size_full()
            .child(header)
            .child(body)
            .into_any_element()
    }

    /// The Diff Tab's "Show File": opens the file in the Preview Tab at the line the diff
    /// cursor is on, so the hunk's surroundings are one click away (ADR 0017). The line is
    /// this Client's presentation: it waits in `pending_file_line` until the text lands.
    fn show_file_from_diff(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        path: RelativePathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row = self
            .diff_editors
            .get(&(key, tab_id))
            .map(|editor| editor.state.read(cx).cursor_position().line as usize);
        let line = self
            .connection(key)
            .and_then(|connection| connection.diffs.get(&(workspace_id, path.clone())))
            .and_then(|answer| answer.as_ref().ok())
            .and_then(|diff| match &diff.content {
                FileDiffContent::Text { hunks } => Some(hunks),
                _ => None,
            })
            .zip(row)
            .and_then(|(hunks, row)| file_line_for_diff_row(hunks, row));
        self.pending_file_line = line.map(|line| (key, workspace_id, path.clone(), line));
        self.show_file_on(key, workspace_id, path, window, cx);
    }

    /// Brings the Diff Tab's Editor in line with the Server's answer for its file: asks
    /// for the diff once, then lays the hunks out as unified text with tinted lines.
    /// Called from `rebuild_dock`, the one place that sees every presentation change.
    pub(super) fn sync_diff_view(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        path: RelativePathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection(key) else {
            return;
        };
        let generation = connection.diffs_generation;
        let answer = connection.diffs.get(&(workspace_id, path.clone()));
        let request = answer.is_none()
            && !self
                .pending_diffs
                .contains(&(key, workspace_id, path.clone()));
        let answer = answer.cloned();
        if request {
            self.request_diff(key, workspace_id, path.clone());
        }

        let editor = self.diff_editors.entry((key, tab_id)).or_insert_with(|| {
            let state = cx.new(|cx| {
                EditorState::new(window, cx)
                    .language("diff")
                    .line_number(true)
                    .soft_wrap(false)
                    .indent_guides(false)
                    .folding(false)
                    .searchable(true)
            });
            let decorations = state.update(cx, |state, cx| {
                state.create_decorations_collection(Vec::new(), cx)
            });
            DiffEditor {
                state,
                decorations,
                shown: None,
                content: DiffContent::Loading,
            }
        });
        let Some(answer) = answer else {
            if editor
                .shown
                .as_ref()
                .is_none_or(|(shown, _)| *shown != path)
            {
                editor.content = DiffContent::Loading;
                editor.shown = None;
            }
            return;
        };
        if editor.shown.as_ref() == Some(&(path.clone(), generation)) {
            return;
        }
        editor.shown = Some((path, generation));
        let (text, decorations, content) = match answer {
            Err(reason) => (String::new(), Vec::new(), DiffContent::Failed(reason)),
            Ok(diff) => match diff.content {
                FileDiffContent::Binary => (String::new(), Vec::new(), DiffContent::Binary),
                FileDiffContent::TooLarge { bytes } => {
                    (String::new(), Vec::new(), DiffContent::TooLarge { bytes })
                }
                FileDiffContent::Text { hunks } if hunks.is_empty() => {
                    (String::new(), Vec::new(), DiffContent::Unchanged)
                }
                FileDiffContent::Text { hunks } => {
                    let (text, decorations, stat) = unified_text(&hunks, cx);
                    (text, decorations, DiffContent::Text { stat })
                }
            },
        };
        editor.content = content;
        editor
            .state
            .update(cx, |state, cx| state.set_value(text, window, cx));
        editor.decorations.set(decorations, cx);
    }

    fn request_diff(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: RelativePathBuf,
    ) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let (Some(server_id), Some(session_id)) = (connection.server_id, connection.session_id)
        else {
            return;
        };
        if connection.status != ConnectionStatus::Connected {
            return;
        }
        let request_id = connection.next_layout_request_id;
        connection.next_layout_request_id = request_id.wrapping_add(1).max(1);
        connection.send(ClientMessage::GitDiff {
            server_id,
            session_id,
            request_id,
            workspace_id,
            path: path.clone(),
            against: DiffBase::Head,
        });
        self.pending_diffs.insert((key, workspace_id, path));
    }

    /// Drops Editors whose Tabs are gone, so a closed Diff Tab frees its text.
    pub(super) fn prune_diff_editors(&mut self) {
        let sessions = self.restored_sessions();
        self.diff_editors.retain(|(key, tab_id), _| {
            sessions
                .get(key)
                .is_some_and(|session| session.tab(*tab_id).is_some_and(|tab| tab.diff().is_some()))
        });
    }
}

pub(super) fn empty_state(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .p_4()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
        .into_any_element()
}

/// The hunks as unified-diff text, the byte ranges of added and removed lines tinted, and
/// the line counts. The `diff` grammar colours the prefixes; the tint marks whole lines.
/// The 0-based line of the new file that `row` of `unified_text`'s layout is about: a
/// hunk header stands for the hunk's first line, a removed line for the new-file line it
/// was removed before, and a row past the last hunk for the end of that hunk. `None` when
/// there is no hunk at all.
fn file_line_for_diff_row(hunks: &[condr_core::DiffHunk], row: usize) -> Option<u32> {
    let mut display = 0;
    for hunk in hunks {
        // `new_start` is 1-based and 0 for a file with no lines left.
        let mut new_line = hunk.new_start.max(1);
        if row == display {
            return Some(new_line - 1);
        }
        display += 1;
        for line in &hunk.lines {
            if row == display {
                return Some(new_line - 1);
            }
            display += 1;
            if line.kind != DiffLineKind::Removed {
                new_line += 1;
            }
        }
    }
    hunks
        .last()
        .map(|hunk| (hunk.new_start + hunk.new_lines).max(1) - 1)
}

fn unified_text(
    hunks: &[condr_core::DiffHunk],
    cx: &App,
) -> (String, Vec<TextDecoration>, GitDiffStat) {
    let added_tint = cx.theme().success.opacity(0.15);
    let removed_tint = cx.theme().danger.opacity(0.15);
    let mut text = String::new();
    let mut decorations = Vec::new();
    let mut stat = GitDiffStat::default();
    for (index, hunk) in hunks.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        text.push_str(&format!(
            "@@ -{},{} +{},{} @@",
            hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
        ));
        for line in &hunk.lines {
            text.push('\n');
            let start = text.len();
            let (prefix, tint) = match line.kind {
                DiffLineKind::Context => (' ', None),
                DiffLineKind::Added => {
                    stat.added += 1;
                    ('+', Some(added_tint))
                }
                DiffLineKind::Removed => {
                    stat.deleted += 1;
                    ('-', Some(removed_tint))
                }
            };
            text.push(prefix);
            text.push_str(&line.text);
            if let Some(tint) = tint {
                decorations.push(TextDecoration::new(
                    start..text.len(),
                    HighlightStyle {
                        background_color: Some(tint),
                        ..HighlightStyle::default()
                    },
                ));
            }
        }
    }
    (text, decorations, stat)
}

#[cfg(test)]
mod tests {
    use super::{ChangeNode, ChangesSection, change_tree, file_line_for_diff_row};
    use condr_core::{GitChangeEntry, GitChangeStatus};
    use relative_path::{RelativePath, RelativePathBuf};

    fn entry(path: &str) -> GitChangeEntry {
        GitChangeEntry {
            path: RelativePathBuf::from(path),
            old_path: None,
            status: GitChangeStatus::Modified,
            stat: None,
        }
    }

    #[test]
    fn statuses_land_in_zed_sections() {
        assert_eq!(
            ChangesSection::of(GitChangeStatus::Conflicted),
            ChangesSection::Conflicts
        );
        assert_eq!(
            ChangesSection::of(GitChangeStatus::Untracked),
            ChangesSection::Untracked
        );
        for status in [
            GitChangeStatus::Added,
            GitChangeStatus::Modified,
            GitChangeStatus::Deleted,
            GitChangeStatus::Renamed,
        ] {
            assert_eq!(ChangesSection::of(status), ChangesSection::Tracked);
        }
    }

    #[test]
    fn the_tree_folds_lone_directories_and_puts_files_after_them() {
        let entries = [
            entry("crates/gui/src/app.rs"),
            entry("crates/gui/src/dock.rs"),
            entry("crates/core/lib.rs"),
            entry("README.md"),
            entry("Cargo.toml"),
        ];
        let refs: Vec<&GitChangeEntry> = entries.iter().collect();
        let tree = change_tree(&refs);
        let ChangeNode::Directory {
            label,
            path,
            children,
        } = &tree[0]
        else {
            panic!("directories come first: {tree:?}");
        };
        assert_eq!(label, "crates");
        assert_eq!(path, RelativePath::new("crates"));
        let labels: Vec<&str> = children
            .iter()
            .map(|node| match node {
                ChangeNode::Directory { label, .. } => label.as_str(),
                ChangeNode::File { name, .. } => name.as_str(),
            })
            .collect();
        assert_eq!(
            labels,
            ["core", "gui/src"],
            "a lone chain folds into one row"
        );
        let ChangeNode::Directory { path, children, .. } = &children[1] else {
            panic!("gui/src is a directory");
        };
        assert_eq!(path, RelativePath::new("crates/gui/src"));
        assert!(matches!(&children[0], ChangeNode::File { name, .. } if name == "app.rs"));
        assert!(matches!(&children[1], ChangeNode::File { name, .. } if name == "dock.rs"));
        assert!(matches!(&tree[1], ChangeNode::File { name, .. } if name == "Cargo.toml"));
        assert!(matches!(&tree[2], ChangeNode::File { name, .. } if name == "README.md"));
    }

    #[test]
    fn a_diff_row_names_the_new_file_line_it_is_about() {
        use condr_core::{DiffHunk, DiffLine, DiffLineKind};
        let line = |kind, new_number| DiffLine {
            kind,
            old_number: None,
            new_number,
            text: String::new(),
        };
        // @@ -1,2 +1,3 @@ / " alpha" / "-beta" / "+BETA" / "+gamma"
        // @@ -10,1 +11,1 @@ / "-old" / "+new"
        let hunks = [
            DiffHunk {
                old_start: 1,
                old_lines: 2,
                new_start: 1,
                new_lines: 3,
                lines: vec![
                    line(DiffLineKind::Context, Some(1)),
                    line(DiffLineKind::Removed, None),
                    line(DiffLineKind::Added, Some(2)),
                    line(DiffLineKind::Added, Some(3)),
                ],
            },
            DiffHunk {
                old_start: 10,
                old_lines: 1,
                new_start: 11,
                new_lines: 1,
                lines: vec![
                    line(DiffLineKind::Removed, None),
                    line(DiffLineKind::Added, Some(11)),
                ],
            },
        ];
        let rows: Vec<Option<u32>> = (0..9)
            .map(|row| file_line_for_diff_row(&hunks, row))
            .collect();
        assert_eq!(
            rows,
            [
                Some(0),  // hunk header: its first line
                Some(0),  // " alpha"
                Some(1),  // "-beta": where it was removed, i.e. "BETA"
                Some(1),  // "+BETA"
                Some(2),  // "+gamma"
                Some(10), // second hunk header
                Some(10), // "-old"
                Some(10), // "+new"
                Some(11), // past the end: after the last hunk
            ]
        );
        assert_eq!(file_line_for_diff_row(&[], 0), None);
        let emptied = [DiffHunk {
            old_start: 1,
            old_lines: 1,
            new_start: 0,
            new_lines: 0,
            lines: vec![line(DiffLineKind::Removed, None)],
        }];
        assert_eq!(file_line_for_diff_row(&emptied, 1), Some(0));
    }
}
