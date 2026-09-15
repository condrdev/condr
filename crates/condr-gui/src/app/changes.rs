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
use std::path::Path;

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
fn status_glyph(status: GitChangeStatus, cx: &App) -> Icon {
    let (name, color): (Icon, Hsla) = match status {
        GitChangeStatus::Added | GitChangeStatus::Untracked => {
            (Icon::new(CondrIconName::SquarePlus), cx.theme().success)
        }
        GitChangeStatus::Modified | GitChangeStatus::Renamed => {
            (Icon::new(CondrIconName::SquareDot), cx.theme().warning)
        }
        GitChangeStatus::Deleted => (Icon::new(CondrIconName::SquareMinus), cx.theme().danger),
        GitChangeStatus::Conflicted => (Icon::new(IconName::TriangleAlert), cx.theme().danger),
    };
    name.size_4().text_color(color)
}

fn status_label(status: GitChangeStatus) -> &'static str {
    match status {
        GitChangeStatus::Added => "Added",
        GitChangeStatus::Modified => "Modified",
        GitChangeStatus::Deleted => "Deleted",
        GitChangeStatus::Renamed => "Renamed",
        GitChangeStatus::Untracked => "Untracked",
        GitChangeStatus::Conflicted => "Conflicted",
    }
}

/// `+N −N`, each half only when it is not zero.
fn diff_stat(stat: GitDiffStat, cx: &App) -> AnyElement {
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

/// A path split for display: the file name first, its directory after it, dimmed.
fn split_path(path: &Path) -> (String, Option<String>) {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.display().to_string().replace('\\', "/"));
    (name, directory)
}

/// The Diff Tab's Editor and what it currently shows.
pub(super) struct DiffEditor {
    pub(super) state: Entity<EditorState>,
    decorations: TextDecorationCollection,
    /// The path and the connection's diff generation the Editor text was built from.
    shown: Option<(PathBuf, u64)>,
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
    pub(super) fn toggle_changes_sidebar(&mut self, cx: &mut Context<Self>) {
        self.changes_open = !self.changes_open;
        cx.notify();
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
        path: PathBuf,
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

    /// The title bar's toggle for the Changes sidebar, next to "Open in".
    pub(super) fn render_changes_toggle(&self, cx: &mut Context<Self>) -> AnyElement {
        let owner = cx.weak_entity();
        let label = if self.changes_open {
            "Hide Changes"
        } else {
            "Show Changes"
        };
        let icon = if self.changes_open {
            IconName::PanelRightClose
        } else {
            IconName::PanelRightOpen
        };
        h_flex()
            .flex_none()
            .h_full()
            .items_center()
            .pl_2()
            // Inside the title bar an unclaimed press starts a window move on Windows.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                Button::new("toggle-changes")
                    .debug_selector(|| "toggle-changes".into())
                    .ghost()
                    .small()
                    .icon(Icon::new(icon))
                    .tooltip(label)
                    .accessibility_label(label)
                    .on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| this.toggle_changes_sidebar(cx));
                    }),
            )
            .into_any_element()
    }

    /// The right sidebar: the presented Workspace's changes against `HEAD`.
    pub(super) fn render_changes_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        // Copied out so the rows below can take `cx` again.
        let theme = cx.theme();
        let (radius, accent, muted, warning) = (
            theme.radius,
            theme.sidebar_accent,
            theme.muted_foreground,
            theme.warning,
        );
        let column = v_flex()
            .debug_selector(|| "condr-changes".into())
            .size_full()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_l_1()
            .border_color(theme.sidebar_border);

        let presented = self.active_connection().and_then(|connection| {
            let session = Session::restore(connection.snapshot.clone()).ok()?;
            let workspace_id = self.presented_workspace_id(connection.key, &session)?;
            let workspace = session.workspace(workspace_id)?;
            let shown = session
                .diff_tab(workspace_id)
                .filter(|tab| {
                    self.presented_tab_id(connection.key, &session, workspace_id) == Some(tab.id())
                })
                .and_then(|tab| tab.diff().map(|diff| diff.path().to_path_buf()));
            Some((
                connection.key,
                workspace_id,
                workspace.name().to_owned(),
                connection.workspace_git.get(&workspace_id).cloned(),
                shown,
            ))
        });
        let Some((key, workspace_id, workspace_name, git, shown)) = presented else {
            return column
                .child(changes_header("Changes", None, cx))
                .child(empty_state("No Workspace", cx))
                .into_any_element();
        };
        let Some(git) = git else {
            return column
                .child(changes_header("Changes", None, cx))
                .child(empty_state(
                    format!("{workspace_name} is not a Git repository"),
                    cx,
                ))
                .into_any_element();
        };
        let entries = &git.changes.entries;
        let column = column.child(changes_header("Changes", Some(entries.len()), cx));
        if entries.is_empty() {
            return column
                .child(empty_state("No changes", cx))
                .into_any_element();
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
            for entry in rows {
                list = list.child(self.render_change_row(
                    key,
                    workspace_id,
                    entry,
                    shown.as_deref() == Some(entry.path.as_path()),
                    cx,
                ));
            }
        }
        column.child(list).into_any_element()
    }

    fn render_change_row(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        entry: &GitChangeEntry,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let (name, directory) = split_path(&entry.path);
        let path_text = entry.path.display().to_string().replace('\\', "/");
        let id: SharedString = format!("change-{path_text}").into();
        let selector = id.clone();
        let deleted = entry.status == GitChangeStatus::Deleted;
        let owner = cx.weak_entity();
        let path = entry.path.clone();
        let tooltip = match &entry.old_path {
            Some(old) => format!(
                "{}: {} → {path_text}",
                status_label(entry.status),
                old.display().to_string().replace('\\', "/")
            ),
            None => format!("{}: {path_text}", status_label(entry.status)),
        };
        h_flex()
            .id(id)
            .debug_selector(move || selector.to_string())
            .h_7()
            .w_full()
            .min_w_0()
            .px_2()
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
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .on_click(move |_, window, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.show_diff_on(key, workspace_id, path.clone(), window, cx);
                });
            })
            .child(status_glyph(entry.status, cx))
            .child(
                h_flex()
                    .min_w_0()
                    .flex_1()
                    .gap_1p5()
                    .items_baseline()
                    .child(
                        div()
                            .flex_none()
                            .when(deleted, |this| {
                                this.line_through().text_color(theme.muted_foreground)
                            })
                            .child(name),
                    )
                    .when_some(directory, |this, directory| {
                        this.child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(directory),
                        )
                    }),
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
        path: &Path,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let path_text = path.display().to_string().replace('\\', "/");
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

    /// Brings the Diff Tab's Editor in line with the Server's answer for its file: asks
    /// for the diff once, then lays the hunks out as unified text with tinted lines.
    /// Called from `rebuild_dock`, the one place that sees every presentation change.
    pub(super) fn sync_diff_view(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        path: PathBuf,
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

    fn request_diff(&mut self, key: ConnectionKey, workspace_id: WorkspaceId, path: PathBuf) {
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

fn changes_header(title: &str, count: Option<usize>, cx: &App) -> AnyElement {
    h_flex()
        .flex_none()
        .h_9()
        .w_full()
        .px_3()
        .items_center()
        .gap_1()
        .text_sm()
        .font_medium()
        .child(title.to_owned())
        .when_some(count, |this, count| {
            this.child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("({count})")),
            )
        })
        .into_any_element()
}

fn empty_state(text: impl Into<SharedString>, cx: &App) -> AnyElement {
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
    use super::{ChangesSection, split_path};
    use condr_core::GitChangeStatus;
    use std::path::Path;

    #[test]
    fn statuses_land_in_zed_sections_and_paths_split_name_first() {
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
        assert_eq!(
            split_path(Path::new("crates/condr-gui/src/app.rs")),
            ("app.rs".to_owned(), Some("crates/condr-gui/src".to_owned()))
        );
        assert_eq!(
            split_path(Path::new("README.md")),
            ("README.md".to_owned(), None)
        );
    }
}
