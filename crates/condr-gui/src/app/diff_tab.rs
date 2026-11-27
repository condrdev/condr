//! The Diff Tab (ADR 0017, ADR 0046): one file's diff in GPUI Kit's `Diff`, against the
//! side the Workspace's Changes compare with, or as a commit Git History chose changed it.
//! Everything shown here is Server data; the Client only asks and draws.

use super::format::age;
use super::git_marks::{diff_stat, short_id};
use super::icons::CondrIconName;
use super::placeholders::{Oversized, empty_state, failed_state, too_large};
use super::syntax::language_for;
use super::viewer_header::{header_meta, mode_toggle, viewer_header};
use super::*;
use condr_core::protocol::DiffBase;
use condr_core::{DiffHunk, DiffLineKind, FileDiffContent, GitChangeEntry, GitChangeStatus};
use gpui_kit::base::text::TextView;
use gpui_kit::component::diff::{Diff, DiffFile, DiffLinePosition, DiffMode, DiffSide, DiffState};
use gpui_kit::component::scroll::ScrollableElement as _;
use rust_i18n::t;

/// The Diff Tab's Kit `Diff` and what it currently shows (ADR 0046).
pub(super) struct DiffViewer {
    pub(super) state: Entity<DiffState>,
    /// The path, the commit when Git History chose one, and the connection's diff generation
    /// the state was built from.
    shown: Option<((RelativePathBuf, Option<String>), u64)>,
    /// The patch last handed to the state. A refetch that writes the same one leaves the
    /// view alone, since `set_files` resets its scrolling and folds.
    pub(super) patch: String,
    pub(super) content: DiffContent,
    /// The line last clicked: where Show File lands, and where a changed diff scrolls back.
    pub(super) clicked: Option<DiffLinePosition>,
    /// The highlighter the state paints with, replaced when the code theme changes.
    pub(super) highlighter: syntax::DiffHighlighter,
}

/// The one name every patch gives its file. The Tab header shows the path, and Kit's
/// parser then never meets a path that needs quoting.
pub(super) const PATCH_PATH: &str = "file";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DiffContent {
    Loading,
    Text,
    /// Both sides are identical: the file left the change list.
    Unchanged,
    Binary,
    TooLarge {
        bytes: u64,
    },
    Failed(String),
}

/// The commit this Client's Diff Tab shows for one path, chosen in Git History. Any other path
/// in the Tab, whoever put it there, is the working tree's.
pub(super) struct DiffCommit {
    pub(super) path: RelativePathBuf,
    pub(super) commit: CommitSummary,
}

/// How this Client shows the Diff Tabs.
#[derive(Default)]
pub(super) struct DiffTabState {
    /// The Diff Tabs' viewers, by connection and Tab; pruned with the Tabs.
    pub(super) viewers: HashMap<(ConnectionKey, TabId), DiffViewer>,
    /// Every Diff Tab shows one column rather than two (ADR 0046): how this person reads
    /// diffs, remembered in the state file.
    pub(super) unified: bool,
    /// The commit each Workspace's Diff Tab shows, chosen in Git History (ADR 0046); this
    /// Client's view, never remembered.
    pub(super) commits: HashMap<(ConnectionKey, WorkspaceId), DiffCommit>,
    /// The Diff Tab shows a commit's whole message, not only its subject.
    pub(super) commit_message_open: bool,
}

impl DiffTabState {
    /// Forgets what `key`'s closed Workspaces and Tabs left behind; `None` is a connection
    /// gone, which leaves nothing of `key`.
    pub(super) fn retain_session(&mut self, key: ConnectionKey, session: Option<&Session>) {
        self.commits.retain(|(connection, id), _| {
            *connection != key || session.is_some_and(|session| session.workspace(*id).is_some())
        });
        self.viewers.retain(|(connection, id), _| {
            *connection != key
                || session
                    .is_some_and(|session| session.tab(*id).is_some_and(|tab| tab.diff().is_some()))
        });
    }
}

impl Condr {
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
        // The working tree's, unless Git History puts its commit back right after (ADR 0046).
        let left_commit = self.diff_tab.commits.remove(&(key, workspace_id)).is_some();
        self.send_presenting_layout_to(
            key,
            LayoutCommand::ShowDiff { workspace_id, path },
            window,
            cx,
        );
        // Retargeting the Tab to its own path changes no layout, so nothing else resyncs it.
        if left_commit {
            self.rebuild_dock(window, cx);
        }
    }

    /// The change a Diff Tab of `path` shows: its commit's own entry once Git History has the
    /// commit, otherwise the Workspace's against its comparison.
    fn diff_entry(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: &RelativePath,
    ) -> Option<&GitChangeEntry> {
        let connection = self.connection(key)?;
        let changes = match self.diff_commit(key, workspace_id, path) {
            Some(commit) => {
                let slot = (workspace_id, commit.id.clone());
                &connection
                    .resources
                    .commits
                    .get(&slot)?
                    .as_ref()
                    .ok()?
                    .changes
            }
            None => {
                let git = connection.model.workspace_git.get(&workspace_id)?;
                let against = self.diff_base(key, workspace_id, git);
                return shown_entries(git, against).find(|entry| entry.path == path);
            }
        };
        changes.entries.iter().find(|entry| entry.path == path)
    }

    /// Every diff of the Workspace is stale: a Diff Tab showing one asks again on the
    /// rebuild.
    pub(super) fn forget_workspace_diffs(&mut self, key: ConnectionKey, workspace_id: WorkspaceId) {
        if let Some(connection) = self.connection_mut(key) {
            connection.resources.invalidate_diffs(workspace_id);
        }
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
        let git = self
            .connection(key)
            .and_then(|connection| connection.model.workspace_git.get(&workspace_id));
        let against = self.workspace_diff_base(key, workspace_id);
        let commit = self.diff_commit(key, workspace_id, path);
        let entry = self.diff_entry(key, workspace_id, path);
        let comparison = match (commit, git) {
            (Some(commit), _) => t!("changes.comparison.commit", id = short_id(&commit.id)),
            (None, Some(git)) => comparison_label(git, against),
            (None, None) => t!("changes.comparison.head"),
        };
        let viewer = self.diff_tab.viewers.get(&(key, tab_id));
        let content = viewer.map_or(DiffContent::Loading, |viewer| viewer.content.clone());
        let muted = theme.muted_foreground;
        let header = viewer_header("diff-header", path, entry.map(|entry| entry.status), cx)
            .child(header_meta("diff-comparison", comparison, muted))
            .when_some(entry.and_then(|entry| entry.stat), |this, stat| {
                this.child(diff_stat(stat, cx))
            })
            .when(content == DiffContent::Text, |this| {
                let owner = cx.weak_entity();
                this.child(mode_toggle(
                    "diff-mode",
                    Button::new("diff-mode-split")
                        .debug_selector(|| "diff-mode-split".into())
                        .icon(Icon::new(gpui_kit::assets::IconName::Columns2))
                        .tooltip(t!("changes.diff.split"))
                        .accessibility_label(t!("changes.diff.split")),
                    Button::new("diff-mode-unified")
                        .debug_selector(|| "diff-mode-unified".into())
                        .icon(Icon::new(gpui_kit::assets::IconName::Rows2))
                        .tooltip(t!("changes.diff.unified"))
                        .accessibility_label(t!("changes.diff.unified")),
                    self.diff_tab.unified,
                    move |unified, cx| {
                        let _ = owner.update(cx, |this, cx| this.set_diff_unified(unified, cx));
                    },
                ))
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
                    .tooltip(t!("changes.diff.show_file"))
                    .accessibility_label(t!("changes.diff.show_file"))
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
        let body = match (&content, viewer) {
            (DiffContent::Text, Some(viewer)) => {
                let owner = cx.weak_entity();
                div()
                    .debug_selector(|| "diff-body".into())
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        // The Tab header already names the file, its status and its counts.
                        Diff::new(&viewer.state)
                            .header_visible(false)
                            .on_line_click(move |position, _, _, cx| {
                                let position = position.clone();
                                let _ = owner.update(cx, |this, _| {
                                    if let Some(viewer) =
                                        this.diff_tab.viewers.get_mut(&(key, tab_id))
                                    {
                                        viewer.clicked = Some(position);
                                    }
                                });
                            })
                            .size_full(),
                    )
                    .into_any_element()
            }
            (DiffContent::Loading, _) => empty_state(t!("changes.diff.loading"), cx),
            (DiffContent::Unchanged, _) => {
                empty_state(t!("changes.diff.unchanged", path = path_text), cx)
            }
            (DiffContent::Binary, _) => empty_state(t!("changes.diff.binary"), cx),
            (DiffContent::TooLarge { bytes }, _) => {
                empty_state(too_large(Oversized::Diff, *bytes), cx)
            }
            (DiffContent::Failed(reason), _) => failed_state(reason.clone(), cx),
            (DiffContent::Text, None) => empty_state(t!("changes.diff.loading"), cx),
        };
        v_flex()
            .debug_selector(|| "diff-tab".into())
            .size_full()
            .child(header)
            .when_some(commit.cloned(), |this, commit| {
                this.child(self.render_commit_header(key, workspace_id, &commit, cx))
            })
            .child(body)
            .into_any_element()
    }

    /// The Diff Tab's second header row for a commit: its subject, short id, author and
    /// age, Copy Commit ID, and the rest of the message behind a disclosure.
    pub(super) fn render_commit_header(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        commit: &CommitSummary,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let body = self
            .connection(key)
            .and_then(|connection| {
                connection
                    .resources
                    .commits
                    .get(&(workspace_id, commit.id.clone()))
            })
            .and_then(|details| details.as_ref().ok())
            .map(|details| message_body(&details.message))
            .filter(|body| !body.is_empty());
        let open = self.diff_tab.commit_message_open;
        let owner = cx.weak_entity();
        let copy_id = commit.id.clone();
        let row = h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_sm()
                    .font_medium()
                    .child(commit.subject.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "{} · {} · {}",
                        short_id(&commit.id),
                        commit.author,
                        age(commit.time)
                    )),
            )
            .child(
                Button::new("diff-copy-commit")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::Copy))
                    .debug_selector(|| "diff-copy-commit".into())
                    .tooltip(t!("git_history.copy_commit_id"))
                    .accessibility_label(t!("git_history.copy_commit_id"))
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy_id.clone()));
                    }),
            )
            .when(body.is_some(), |this| {
                this.child(
                    Button::new("diff-commit-message")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(if open {
                            IconName::ChevronUp
                        } else {
                            IconName::ChevronDown
                        }))
                        .debug_selector(|| "diff-commit-message".into())
                        .tooltip(if open {
                            t!("git_history.hide_message")
                        } else {
                            t!("git_history.show_message")
                        })
                        .on_click(move |_, _, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                this.diff_tab.commit_message_open ^= true;
                                cx.notify();
                            });
                        }),
                )
            });
        v_flex()
            .debug_selector(|| "diff-commit-header".into())
            .flex_none()
            .w_full()
            .px_3()
            .py_1()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .child(row)
            .when_some(body.filter(|_| open), |this, body| {
                this.child(
                    div()
                        .id("diff-commit-body")
                        .debug_selector(|| "diff-commit-body".into())
                        .max_h(rems(12.))
                        .overflow_y_scrollbar()
                        .text_sm()
                        .child(
                            TextView::markdown(
                                SharedString::from(format!("commit-body-{}", commit.id)),
                                escaped_markdown(&body),
                            )
                            .selectable(true),
                        ),
                )
            })
            .into_any_element()
    }

    /// The Diff Tab's "Show File": opens the file in the Preview Tab at the line last
    /// clicked in the diff, or at the first change (ADR 0017, ADR 0046). The line is this
    /// Client's presentation: it waits in `pending_file_line` until the text lands.
    fn show_file_from_diff(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        path: RelativePathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A commit's line numbers say nothing about today's file, so it opens at its top.
        if self.diff_commit(key, workspace_id, &path).is_some() {
            self.preview_tab.pending_line = None;
            self.show_file_on(key, workspace_id, path, window, cx);
            return;
        }
        let clicked = self
            .diff_tab
            .viewers
            .get(&(key, tab_id))
            .and_then(|viewer| viewer.clicked.clone());
        let line = self
            .connection(key)
            .and_then(|connection| {
                connection
                    .resources
                    .diffs
                    .get(&(workspace_id, path.clone()))
            })
            .and_then(|answer| answer.as_ref().ok())
            .and_then(|diff| match &diff.content {
                FileDiffContent::Text { hunks } => Some(hunks),
                _ => None,
            })
            .and_then(|hunks| file_line_for_click(hunks, clicked.as_ref()));
        self.preview_tab.pending_line = line.map(|line| (key, workspace_id, path.clone(), line));
        self.show_file_on(key, workspace_id, path, window, cx);
    }

    /// Brings the Diff Tab's `Diff` in line with the Server's answer for its file: asks for
    /// the diff once, then hands the hunks to Kit as a patch (ADR 0046). Called from
    /// `rebuild_dock`, the one place that sees every presentation change.
    pub(super) fn sync_diff_view(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        path: RelativePathBuf,
        cx: &mut Context<Self>,
    ) {
        let commit = self
            .diff_commit(key, workspace_id, &path)
            .map(|commit| commit.id.clone());
        let Some(connection) = self.connection(key) else {
            return;
        };
        let resources = &connection.resources;
        let status = self
            .diff_entry(key, workspace_id, &path)
            .map(|entry| entry.status);
        // A commit's answer never changes, so it needs no generation.
        let (answer, generation) = match &commit {
            Some(commit) => (
                resources
                    .commit_diffs
                    .get(&(workspace_id, commit.clone(), path.clone()))
                    .cloned(),
                0,
            ),
            None => (
                resources.diffs.get(&(workspace_id, path.clone())).cloned(),
                resources.diffs_generation,
            ),
        };
        let request = answer.is_none()
            && commit.is_none()
            && !resources.diffs.is_pending(&(workspace_id, path.clone()));
        match &commit {
            Some(commit) if answer.is_none() => {
                self.request_commit_diff(key, workspace_id, commit.clone(), path.clone());
            }
            _ if request => self.request_diff(key, workspace_id, path.clone()),
            _ => {}
        }

        let mode = diff_mode(self.diff_tab.unified);
        let highlighter = syntax::diff_highlighter(cx);
        let viewer = self
            .diff_tab
            .viewers
            .entry((key, tab_id))
            .or_insert_with(|| DiffViewer {
                state: cx.new(|cx| {
                    let highlight = highlighter.clone();
                    DiffState::new([], cx)
                        .with_mode(mode)
                        .with_syntax_highlighter(move |language, text| highlight(language, text))
                }),
                shown: None,
                patch: String::new(),
                content: DiffContent::Loading,
                clicked: None,
                highlighter,
            });
        let source = (path, commit);
        let Some(answer) = answer else {
            if viewer
                .shown
                .as_ref()
                .is_none_or(|(shown, _)| *shown != source)
            {
                viewer.content = DiffContent::Loading;
                viewer.shown = None;
            }
            return;
        };
        if viewer.shown.as_ref() == Some(&(source.clone(), generation)) {
            return;
        }
        let retargeted = viewer
            .shown
            .as_ref()
            .is_none_or(|(shown, _)| *shown != source);
        // The patch names no path, so another file's could match it but not its language.
        let language = language_for(&source.0);
        viewer.shown = Some((source, generation));
        if retargeted {
            viewer.clicked = None;
            viewer.patch.clear();
        }
        let patch = match answer {
            Err(reason) => Err(DiffContent::Failed(reason)),
            Ok(diff) => match diff.content {
                FileDiffContent::Binary => Err(DiffContent::Binary),
                FileDiffContent::TooLarge { bytes } => Err(DiffContent::TooLarge { bytes }),
                FileDiffContent::Text { hunks } if hunks.is_empty() => Err(DiffContent::Unchanged),
                FileDiffContent::Text { hunks } => Ok(patch_text(status, &hunks)),
            },
        };
        let patch = match patch {
            Ok(patch) => patch,
            Err(content) => {
                viewer.content = content;
                viewer.patch.clear();
                return;
            }
        };
        if patch == viewer.patch {
            viewer.content = DiffContent::Text;
            return;
        }
        match DiffFile::parse(&patch) {
            Ok(files) => {
                let files = files
                    .into_iter()
                    .map(|file| file.with_language(language.clone()));
                let clicked = viewer.clicked.clone();
                viewer.state.update(cx, |state, cx| {
                    state.set_files(files, cx);
                    // ponytail: Kit keeps no viewport across `set_files`, so a changed
                    // diff returns to the line last clicked, else to the top; keep the
                    // top line once Kit can say which it is.
                    if let Some(line) = clicked {
                        state.scroll_to_line(line, cx);
                    }
                });
                viewer.content = DiffContent::Text;
                viewer.patch = patch;
            }
            Err(error) => {
                viewer.content =
                    DiffContent::Failed(t!("changes.diff.failed", error = error).into_owned());
                viewer.patch.clear();
            }
        }
    }

    /// Hands every Diff the highlighter for the code theme painting now (ADR 0046).
    pub(super) fn sync_diff_highlighters(&mut self, cx: &mut Context<Self>) {
        let highlighter = syntax::diff_highlighter(cx);
        for viewer in self.diff_tab.viewers.values_mut() {
            if Arc::ptr_eq(&viewer.highlighter, &highlighter) {
                continue;
            }
            viewer.highlighter = highlighter.clone();
            let highlight = highlighter.clone();
            viewer.state.update(cx, |state, cx| {
                state.set_syntax_highlighter(move |language, text| highlight(language, text), cx);
            });
        }
    }

    fn set_diff_unified(&mut self, unified: bool, cx: &mut Context<Self>) {
        self.diff_tab.unified = unified;
        let mode = diff_mode(unified);
        for viewer in self.diff_tab.viewers.values() {
            viewer
                .state
                .update(cx, |state, cx| state.set_mode(mode, cx));
        }
        self.schedule_state_save(cx);
        cx.notify();
    }

    fn request_diff(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: RelativePathBuf,
    ) {
        let against = self.workspace_diff_base(key, workspace_id);
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if let Some(request_id) =
            connection.send_request(|server_id, session_id, request_id| ClientMessage::GitDiff {
                server_id,
                session_id,
                request_id,
                workspace_id,
                path: path.clone(),
                against,
                full_context: true,
                commit: None,
            })
        {
            connection
                .resources
                .diffs
                .asked((workspace_id, path), request_id);
        }
    }

    /// The commit the Diff Tab shows for `path`, when this Client chose one in Git History.
    pub(super) fn diff_commit(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: &RelativePath,
    ) -> Option<&CommitSummary> {
        self.diff_tab
            .commits
            .get(&(key, workspace_id))
            .filter(|chosen| chosen.path == path)
            .map(|chosen| &chosen.commit)
    }

    /// One file of one commit for the Diff Tab, with every line as context.
    pub(super) fn request_commit_diff(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        commit: String,
        path: RelativePathBuf,
    ) {
        let slot = (workspace_id, commit.clone(), path.clone());
        if self.connection(key).is_none_or(|connection| {
            connection.resources.commit_diffs.contains_key(&slot)
                || connection
                    .resources
                    .pending_commit_diffs
                    .values()
                    .any(|pending| *pending == slot)
        }) {
            return;
        }
        // The message's body for the header comes with the commit's own details, from the
        // repository the path lies in.
        let repository = self
            .connection(key)
            .and_then(|connection| connection.model.workspace_git.get(&workspace_id))
            .and_then(|git| {
                // The innermost: the Root's own, first, holds every path.
                git.repositories
                    .iter()
                    .rev()
                    .find(|repository| path.starts_with(&repository.directory))
            })
            .map(|repository| repository.directory.clone())
            .unwrap_or_default();
        self.request_commit(key, workspace_id, repository, commit.clone());
        if let Some(request_id) = self.send_request(key, |server_id, session_id, request_id| {
            ClientMessage::GitDiff {
                server_id,
                session_id,
                request_id,
                workspace_id,
                path,
                against: DiffBase::Head,
                full_context: true,
                commit: Some(commit),
            }
        }) && let Some(connection) = self.connection_mut(key)
        {
            connection
                .resources
                .pending_commit_diffs
                .insert(request_id, slot);
        }
    }
}

fn diff_mode(unified: bool) -> DiffMode {
    if unified {
        DiffMode::Unified
    } else {
        DiffMode::Split
    }
}

/// The Server's hunks written as the Git patch Kit's `DiffFile::parse` reads (ADR 0046):
/// the wire carries structured lines, and Kit has no constructor that takes them.
fn patch_text(status: Option<GitChangeStatus>, hunks: &[DiffHunk]) -> String {
    let added = matches!(
        status,
        Some(GitChangeStatus::Added | GitChangeStatus::Untracked)
    );
    let old = if added {
        "/dev/null".to_owned()
    } else {
        format!("a/{PATCH_PATH}")
    };
    let new = if status == Some(GitChangeStatus::Deleted) {
        "/dev/null".to_owned()
    } else {
        format!("b/{PATCH_PATH}")
    };
    let mut text = format!("--- {old}\n+++ {new}\n");
    for hunk in hunks {
        text.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
        ));
        for line in &hunk.lines {
            text.push(match line.kind {
                DiffLineKind::Context => ' ',
                DiffLineKind::Added => '+',
                DiffLineKind::Removed => '-',
            });
            text.push_str(&line.text);
            text.push('\n');
        }
    }
    text
}

/// The 0-based line of the new file a click in the diff is about: the line itself on the
/// new side, and for a line only the old side has, the new line it was removed before. With
/// no click, the first change. `None` when there is no line at all.
fn file_line_for_click(hunks: &[DiffHunk], clicked: Option<&DiffLinePosition>) -> Option<u32> {
    let lines = || hunks.iter().flat_map(|hunk| &hunk.lines);
    let old_number = match clicked {
        Some(position) if position.side() == DiffSide::Modified => {
            return u32::try_from(position.line()).ok().map(|line| line - 1);
        }
        Some(position) => u32::try_from(position.line()).ok(),
        None => {
            let first = lines().find(|line| line.kind != DiffLineKind::Context)?;
            if let Some(new) = first.new_number {
                return Some(new - 1);
            }
            first.old_number
        }
    }?;
    let mut after = lines().skip_while(|line| line.old_number != Some(old_number));
    let clicked = after.next()?;
    if let Some(new) = clicked.new_number {
        return Some(new - 1);
    }
    // Removed: the next line the new side has, or the end of the file.
    after
        .find_map(|line| line.new_number)
        .map(|new| new - 1)
        .or_else(|| lines().filter_map(|line| line.new_number).next_back())
        .or(Some(0))
}

/// A message without its subject line and the blank line after it.
fn message_body(message: &str) -> String {
    let mut lines = message.lines();
    lines.next();
    lines
        .skip_while(|line| line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned()
}

/// `text` as Markdown that renders it as written: every punctuation character escaped and
/// every line break kept, since a commit message is plain text (ADR 0046).
fn escaped_markdown(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() * 2);
    for (index, line) in text.lines().enumerate() {
        if index > 0 {
            escaped.push_str("\\\n");
        }
        for character in line.chars() {
            if character.is_ascii_punctuation() {
                escaped.push('\\');
            }
            escaped.push(character);
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::{PATCH_PATH, escaped_markdown, file_line_for_click, message_body, patch_text};
    use condr_core::GitChangeStatus;
    fn hunks() -> [condr_core::DiffHunk; 2] {
        use condr_core::{DiffHunk, DiffLine, DiffLineKind};
        let line = |kind, old_number, new_number, text: &str| DiffLine {
            kind,
            old_number,
            new_number,
            text: text.to_owned(),
        };
        // @@ -1,2 +1,3 @@ / " alpha" / "-beta" / "+BETA" / "+gamma"
        // @@ -10,1 +11,1 @@ / "-old" / "+new"
        [
            DiffHunk {
                old_start: 1,
                old_lines: 2,
                new_start: 1,
                new_lines: 3,
                lines: vec![
                    line(DiffLineKind::Context, Some(1), Some(1), "alpha"),
                    line(DiffLineKind::Removed, Some(2), None, "beta"),
                    line(DiffLineKind::Added, None, Some(2), "BETA"),
                    line(DiffLineKind::Added, None, Some(3), "gamma"),
                ],
            },
            DiffHunk {
                old_start: 10,
                old_lines: 1,
                new_start: 11,
                new_lines: 1,
                lines: vec![
                    line(DiffLineKind::Removed, Some(10), None, "old"),
                    line(DiffLineKind::Added, None, Some(11), "new"),
                ],
            },
        ]
    }

    #[test]
    fn a_clicked_diff_line_names_the_new_file_line_it_is_about() {
        use gpui_kit::component::diff::{DiffLinePosition, DiffSide};
        let hunks = hunks();
        let click = |side, line| {
            file_line_for_click(&hunks, Some(&DiffLinePosition::new(PATCH_PATH, side, line)))
        };
        assert_eq!(click(DiffSide::Modified, 2), Some(1), "BETA");
        assert_eq!(
            click(DiffSide::Original, 1),
            Some(0),
            "alpha, on the old side"
        );
        assert_eq!(
            click(DiffSide::Original, 2),
            Some(1),
            "beta was removed before BETA"
        );
        assert_eq!(
            click(DiffSide::Original, 10),
            Some(10),
            "old was removed before new"
        );
        assert_eq!(
            file_line_for_click(&hunks, None),
            Some(1),
            "no click: the first change"
        );
        assert_eq!(file_line_for_click(&[], None), None);
        let emptied = [condr_core::DiffHunk {
            old_start: 1,
            old_lines: 1,
            new_start: 0,
            new_lines: 0,
            lines: vec![condr_core::DiffLine {
                kind: condr_core::DiffLineKind::Removed,
                old_number: Some(1),
                new_number: None,
                text: String::new(),
            }],
        }];
        assert_eq!(file_line_for_click(&emptied, None), Some(0));
    }

    #[test]
    fn hunks_written_as_a_patch_parse_back_with_their_status() {
        use gpui_kit::component::diff::{DiffFile, DiffFileStatus};
        let parse = |status| {
            let mut files = DiffFile::parse(&patch_text(status, &hunks())).unwrap();
            assert_eq!(files.len(), 1);
            files.remove(0)
        };
        let modified = parse(Some(GitChangeStatus::Modified));
        assert_eq!(modified.status(), DiffFileStatus::Modified);
        assert_eq!((modified.additions(), modified.deletions()), (3, 2));
        assert_eq!(modified.path().as_ref(), PATCH_PATH);

        let added = [condr_core::DiffHunk {
            old_start: 0,
            old_lines: 0,
            new_start: 1,
            new_lines: 1,
            lines: vec![condr_core::DiffLine {
                kind: condr_core::DiffLineKind::Added,
                old_number: None,
                new_number: Some(1),
                text: "new".into(),
            }],
        }];
        let file = DiffFile::parse(&patch_text(Some(GitChangeStatus::Untracked), &added)).unwrap();
        assert_eq!(file[0].status(), DiffFileStatus::Added);
        assert_eq!(parse(None).status(), DiffFileStatus::Modified);
    }

    #[test]
    fn a_message_body_drops_the_subject_and_escapes_as_written() {
        assert_eq!(
            message_body("Fix the parser\n\n- keeps *tokens*\n- drops none\n"),
            "- keeps *tokens*\n- drops none"
        );
        assert_eq!(message_body("Only a subject\n"), "");
        assert_eq!(
            escaped_markdown("- keeps *tokens*\n# not a heading"),
            "\\- keeps \\*tokens\\*\\\n\\# not a heading"
        );
    }
}
