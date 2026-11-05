//! The Preview Tab's change marks (ADR 0050): the Source view's text, unsaved edits
//! included, against the side its Workspace's Changes compare with, drawn in the gutter
//! and on the scrollbar track through Kit's decoration collections.

use super::change_tree::status_color;
use super::workspace_resources::OldFilePath;
use super::*;
use condr_core::GitChangeStatus;
use gpui_kit::component::input::{
    GutterDecoration, GutterDecorationCollection, OverviewRulerDecoration,
    OverviewRulerDecorationCollection, OverviewRulerLane,
};
use std::ops::Range;
use std::time::{Duration, Instant};

/// A diff runs at most this often while the text or the old side moves, as VS Code's.
const THROTTLE: Duration = Duration::from_millis(200);
/// A diff still running after this keeps the marks it had.
const DEADLINE: Duration = Duration::from_millis(500);
/// VS Code draws the scrollbar mark at 60% of the gutter colour.
const RULER_OPACITY: f32 = 0.6;

/// One changed run of the new text, in the UTF-8 byte offsets Kit's decorations take.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LineChange {
    /// Lines with nothing on the old side.
    Added(Range<usize>),
    /// Lines that replace old ones.
    Modified(Range<usize>),
    /// Old lines with nothing on the new side, at the start of the line after them, or at
    /// the end of the text when the end was deleted.
    Deleted(usize),
}

/// A Preview Tab's marks: Kit's two collections while its file has a side to compare
/// with, and the throttled diff that fills them.
#[derive(Default)]
pub(super) struct ChangeMarks {
    collections: Option<(
        GutterDecorationCollection,
        OverviewRulerDecorationCollection,
    )>,
    /// What the marks compare with, the generation that answer landed at, and its text;
    /// `None` text marks nothing.
    old: Option<(OldFilePath, u64, Option<Arc<str>>)>,
    /// Moves with every change of the text or the old side; a diff applies only to the
    /// revision it read.
    revision: u64,
    last_run: Option<Instant>,
    task: Option<Task<()>>,
    /// Diffs started, for the throttle's test.
    #[cfg(test)]
    pub(super) runs: u32,
}

impl ChangeMarks {
    /// Reserves the gutter strip; it stays while the collections exist, so the text does
    /// not shift as marks come and go.
    fn ensure_collections(&mut self, state: &Entity<EditorState>, cx: &mut App) {
        if self.collections.is_none() {
            self.collections = Some(state.update(cx, |state, cx| {
                (
                    state.create_gutter_decorations_collection(Vec::new(), cx),
                    state.create_overview_ruler_decorations_collection(Vec::new(), cx),
                )
            }));
        }
    }

    fn set(&self, changes: &[LineChange], cx: &mut App) {
        let Some((gutter, ruler)) = &self.collections else {
            return;
        };
        let (bars, ruler_marks) = decorations(changes, cx);
        gutter.set(bars, cx);
        ruler.set(ruler_marks, cx);
    }

    /// Nothing to compare with: the marks go, the strip stays.
    fn clear(&mut self, cx: &mut App) {
        self.old = None;
        self.task = None;
        self.set(&[], cx);
    }

    /// The gutter's and the scrollbar's ranges, while the strip is reserved.
    #[cfg(test)]
    pub(super) fn ranges(&self, cx: &App) -> Option<[Vec<Range<usize>>; 2]> {
        let (gutter, ruler) = self.collections.as_ref()?;
        Some([gutter.get_ranges(cx), ruler.get_ranges(cx)])
    }

    /// No side to compare with at all: the strip goes too.
    fn dispose(&mut self, cx: &mut App) {
        self.old = None;
        self.task = None;
        if let Some((gutter, ruler)) = self.collections.take() {
            gutter.dispose(cx);
            ruler.dispose(cx);
        }
    }
}

impl Condr {
    /// Keeps a Preview Tab's marks on the side its Workspace's Changes compare with
    /// (ADR 0050): asks for that side whenever none is cached, which a Git change or a
    /// comparison switch causes, and diffs again when a new answer lands. Only text in a
    /// Workspace with Git is marked.
    pub(super) fn sync_change_marks(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        let against = self.workspace_diff_base(key, workspace_id);
        let (Some(connection), Some(editor)) = (
            self.connection(key),
            self.files_view.file_editors.get(&(key, tab_id)),
        ) else {
            return;
        };
        let slot = editor
            .shown
            .as_ref()
            .filter(|_| editor.content == FileViewContent::Text)
            .filter(|_| connection.model.workspace_git.contains_key(&workspace_id))
            .filter(|_| !connection.resources.old_files_unsupported)
            .map(|(path, _)| (workspace_id, path.clone(), against));
        let answer = slot
            .as_ref()
            .and_then(|slot| connection.resources.old_files.get(slot).cloned());
        let ask = slot
            .as_ref()
            .filter(|slot| connection.resources.old_files.wants(slot))
            .cloned();
        if let Some(slot) = ask {
            self.request_old_file(key, slot);
        }
        let Some(editor) = self.files_view.file_editors.get_mut(&(key, tab_id)) else {
            return;
        };
        let marks = &mut editor.marks;
        let Some(slot) = slot else {
            marks.dispose(cx);
            return;
        };
        marks.ensure_collections(&editor.state, cx);
        // Another file's marks go at once; this file's stay until its new side lands.
        if marks
            .old
            .as_ref()
            .is_some_and(|(shown, _, _)| shown.1 != slot.1)
        {
            marks.clear(cx);
        }
        let Some((generation, text)) = answer else {
            return;
        };
        if marks
            .old
            .as_ref()
            .is_some_and(|(shown, landed, _)| *shown == slot && *landed == generation)
        {
            return;
        }
        let markable = text.is_some();
        marks.old = Some((slot, generation, text));
        if markable {
            self.schedule_change_marks(key, tab_id, cx);
        } else {
            marks.set(&[], cx);
        }
    }

    fn request_old_file(&mut self, key: ConnectionKey, slot: OldFilePath) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let (workspace_id, path, against) = slot.clone();
        if let Some(request_id) =
            connection.send_request(
                |server_id, session_id, request_id| ClientMessage::GitOldFile {
                    server_id,
                    session_id,
                    request_id,
                    workspace_id,
                    path,
                    against,
                },
            )
        {
            connection.resources.asked_old_file(slot, request_id);
        }
    }

    /// The text or the old side moved: diffs again, at most once per [`THROTTLE`], on the
    /// background executor. A burst of edits waits for the pass in flight, which then runs
    /// once more on the latest text.
    pub(super) fn schedule_change_marks(
        &mut self,
        key: ConnectionKey,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.files_view.file_editors.get_mut(&(key, tab_id)) else {
            return;
        };
        let marks = &mut editor.marks;
        marks.revision += 1;
        let markable = marks
            .old
            .as_ref()
            .is_some_and(|(_, _, text)| text.is_some());
        if marks.task.is_some() || !markable {
            return;
        }
        let wait = marks.last_run.map_or(Duration::ZERO, |last| {
            (last + THROTTLE).saturating_duration_since(cx.background_executor().now())
        });
        marks.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let Ok(Some((revision, old, new))) = this.update(cx, |this, cx| {
                let editor = this.files_view.file_editors.get_mut(&(key, tab_id))?;
                let marks = &mut editor.marks;
                marks.last_run = Some(cx.background_executor().now());
                #[cfg(test)]
                {
                    marks.runs += 1;
                }
                let old = marks.old.as_ref().and_then(|(_, _, text)| text.clone());
                let Some(old) = old else {
                    marks.task = None;
                    return None;
                };
                Some((marks.revision, old, editor.state.read(cx).value()))
            }) else {
                return;
            };
            let changes = cx
                .background_spawn(
                    async move { line_changes(&old, &new, Instant::now() + DEADLINE) },
                )
                .await;
            this.update(cx, |this, cx| {
                this.apply_change_marks(key, tab_id, revision, changes, cx)
            })
            .ok();
        }));
    }

    fn apply_change_marks(
        &mut self,
        key: ConnectionKey,
        tab_id: TabId,
        revision: u64,
        changes: Option<Vec<LineChange>>,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.files_view.file_editors.get_mut(&(key, tab_id)) else {
            return;
        };
        let marks = &mut editor.marks;
        marks.task = None;
        if marks.revision != revision {
            // Kit has moved the marks with the edits since; only the latest text's diff
            // replaces them.
            self.schedule_change_marks(key, tab_id, cx);
            return;
        }
        // A diff past its deadline keeps the marks Kit has been moving with the edits.
        if let Some(changes) = changes {
            marks.set(&changes, cx);
        }
    }

    /// The GUI theme changed: the marks take its status colours.
    pub(super) fn recolor_change_marks(&mut self, cx: &mut Context<Self>) {
        let tabs: Vec<_> = self.files_view.file_editors.keys().copied().collect();
        for (key, tab_id) in tabs {
            self.schedule_change_marks(key, tab_id, cx);
        }
    }
}

/// The changed runs of `new` against `old`, line by line, or `None` past `deadline`.
/// Line breaks are not compared, so a CRLF side matches an LF one line for line.
pub(super) fn line_changes(old: &str, new: &str, deadline: Instant) -> Option<Vec<LineChange>> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let ops = similar::capture_diff_slices_deadline(
        similar::Algorithm::Myers,
        &old_lines,
        &new_lines,
        Some(deadline),
    );
    if Instant::now() > deadline {
        return None;
    }
    // Where each new line starts, and the end of the text after the last.
    let starts: Vec<usize> = std::iter::once(0)
        .chain(new.match_indices('\n').map(|(at, _)| at + 1))
        .take(new_lines.len())
        .chain(std::iter::once(new.len()))
        .collect();
    let mut changes = Vec::new();
    // The run being collected: where it starts and ends among the new lines, and how many
    // old lines it replaces.
    let mut run: Option<(usize, usize, usize)> = None;
    let mut flush = |run: &mut Option<(usize, usize, usize)>| {
        if let Some((start, end, old_len)) = run.take() {
            changes.push(match (old_len, end - start) {
                (_, 0) => LineChange::Deleted(starts[start]),
                (0, _) => LineChange::Added(starts[start]..starts[end]),
                _ => LineChange::Modified(starts[start]..starts[end]),
            });
        }
    };
    for op in &ops {
        if let similar::DiffOp::Equal { .. } = op {
            flush(&mut run);
            continue;
        }
        let (old_range, new_range) = (op.old_range(), op.new_range());
        let (start, _, old_len) = run.get_or_insert((new_range.start, new_range.start, 0));
        run = Some((*start, new_range.end, *old_len + old_range.len()));
    }
    flush(&mut run);
    Some(changes)
}

/// Each change's gutter bar or deletion pill, and its scrollbar mark in the left lane, in
/// the Changes status colours.
fn decorations(
    changes: &[LineChange],
    cx: &App,
) -> (Vec<GutterDecoration>, Vec<OverviewRulerDecoration>) {
    changes
        .iter()
        .map(|change| {
            let (range, status) = match change {
                LineChange::Added(range) => (range.clone(), GitChangeStatus::Added),
                LineChange::Modified(range) => (range.clone(), GitChangeStatus::Modified),
                LineChange::Deleted(at) => (*at..*at, GitChangeStatus::Deleted),
            };
            let color = status_color(status, cx);
            (
                GutterDecoration::new(range.clone()).with_color(color),
                OverviewRulerDecoration::new(range)
                    .with_lane(OverviewRulerLane::Left)
                    .with_color(color.opacity(RULER_OPACITY)),
            )
        })
        .unzip()
}

#[cfg(test)]
mod tests {
    use super::{LineChange, line_changes};
    use std::time::{Duration, Instant};

    fn changes(old: &str, new: &str) -> Vec<LineChange> {
        line_changes(old, new, Instant::now() + Duration::from_secs(10)).unwrap()
    }

    #[test]
    fn a_line_diff_becomes_added_modified_and_deleted_runs() {
        let old = "one\ntwo\nthree\nfour\nfive\n";
        let new = "one\nTWO\nthree\nnew\nfour\n";
        assert_eq!(
            changes(old, new),
            vec![
                LineChange::Modified(4..8),
                LineChange::Added(14..18),
                // `five` went from the end: after the last line.
                LineChange::Deleted(new.len()),
            ]
        );
        assert_eq!(changes(old, old), vec![]);
    }

    #[test]
    fn a_deletion_sits_on_the_top_edge_of_the_line_after_it() {
        // At the top of the file: before the first line.
        assert_eq!(
            changes("gone\nkept\n", "kept\n"),
            vec![LineChange::Deleted(0)]
        );
        // In the middle: at the start of the line after it.
        assert_eq!(
            changes("a\ngone\nb\n", "a\nb\n"),
            vec![LineChange::Deleted(2)]
        );
        // At the end of a file without a final line break: its end.
        assert_eq!(changes("a\ngone", "a"), vec![LineChange::Deleted(1)]);
        // Everything: the empty text's only boundary.
        assert_eq!(changes("a\nb\n", ""), vec![LineChange::Deleted(0)]);
    }

    #[test]
    fn a_file_the_old_side_lacks_is_wholly_added() {
        let new = "fn main() {}\n\nlast";
        assert_eq!(changes("", new), vec![LineChange::Added(0..new.len())]);
        assert_eq!(changes("", ""), vec![]);
    }

    #[test]
    fn crlf_lines_match_lf_lines_and_ranges_count_the_cr() {
        assert_eq!(changes("a\r\nb\r\n", "a\nb\n"), vec![]);
        assert_eq!(changes("a\nb\n", "a\r\nb\r\n"), vec![]);
        // The second line of a CRLF text starts after both bytes of the first break.
        assert_eq!(
            changes("a\r\nb\r\n", "a\r\nB\r\n"),
            vec![LineChange::Modified(3..6)]
        );
    }

    #[test]
    fn a_diff_past_its_deadline_marks_nothing_new() {
        assert_eq!(
            line_changes("a\n", "b\n", Instant::now() - Duration::from_secs(1)),
            None
        );
    }
}
