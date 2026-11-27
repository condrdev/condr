//! The Preview Tab (ADR 0018): one file of the Workspace in GPUI Kit's Editor, or
//! rendered when it is Markdown or an image (ADR 0037). The Editor saves what it edits
//! back to the Server (ADR 0045); `edits` holds that side, `change_marks` the gutter's
//! marks against the Workspace's Changes (ADR 0050).

mod change_marks;
mod edits;
mod rendering;

use super::placeholders::{Oversized, empty_state, failed_state, too_large};
use super::syntax::language_for;
use super::viewer_header::{header_meta, mode_toggle, viewer_header};
use super::*;
use gpui_kit::base::text::TextViewState;
use gpui_kit::component::input::{Position, RopeExt as _};
use rust_i18n::t;

pub(super) use edits::*;
pub(super) use rendering::*;

/// What this Client keeps of the Preview Tabs.
#[derive(Default)]
pub(super) struct PreviewTabState {
    /// The Preview Tabs' Editors, by connection and Tab; pruned with the Tabs.
    pub(super) editors: HashMap<(ConnectionKey, TabId), FileEditor>,
    /// The 0-based line the Preview Tab lands on once it shows this file: set by the Diff
    /// Tab's "Show File", applied by `sync_file_view` when the text is there, then cleared.
    /// Presentation only, so it never rides `ShowFile`.
    pub(super) pending_line: Option<(ConnectionKey, WorkspaceId, RelativePathBuf, u32)>,
    /// The images the Preview Tabs show, decoded (ADR 0037).
    pub(super) images: PreviewImages,
    /// Saves in flight, by connection and request (ADR 0045).
    pub(super) saves: HashMap<(ConnectionKey, u64), PendingSave>,
    /// Questions that arrived without a window, asked one at a time from `render`.
    pub(super) prompts: std::collections::VecDeque<FilePrompt>,
    pub(super) prompt_open: bool,
    /// What waits for a window: the action a landed save let through, or a dialog to
    /// open again with the Server's answer.
    pub(super) deferred: Vec<Continuation>,
}

impl PreviewTabState {
    /// Whether any Preview Tab holds text it has not saved.
    pub(super) fn has_unsaved(&self) -> bool {
        self.editors.values().any(|editor| editor.dirty)
    }

    /// Forgets what `key`'s closed Workspaces and Tabs left behind; `None` is a connection
    /// gone, which leaves nothing of `key`. Hands back the images it dropped, for
    /// `release_images` to free from the atlas.
    pub(super) fn retain_session(
        &mut self,
        key: ConnectionKey,
        session: Option<&Session>,
    ) -> Vec<Arc<DecodedImage>> {
        self.editors.retain(|(connection, id), _| {
            *connection != key
                || session
                    .is_some_and(|session| session.tab(*id).is_some_and(|tab| tab.file().is_some()))
        });
        if self
            .pending_line
            .as_ref()
            .is_some_and(|(connection, id, _, _)| {
                *connection == key
                    && !session.is_some_and(|session| session.workspace(*id).is_some())
            })
        {
            self.pending_line = None;
        }
        // A Workspace's images are its Preview Tab's, so they go when that Tab closes.
        self.images.retain(|(connection, id, _)| {
            *connection != key
                || session
                    .and_then(|session| session.workspace(*id))
                    .is_some_and(|workspace| {
                        workspace.tabs().iter().any(|tab| tab.file().is_some())
                    })
        })
    }
}

/// The byte range of `old` that differs from `new`, and the text of `new` that replaces it.
fn changed_span<'a>(old: &str, new: &'a str) -> (std::ops::Range<usize>, &'a str) {
    let mut prefix = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let mut suffix = old
        .bytes()
        .rev()
        .zip(new.bytes().rev())
        .take(old.len().min(new.len()) - prefix)
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }
    (prefix..old.len() - suffix, &new[prefix..new.len() - suffix])
}

/// Takes the line the Diff Tab asked the Preview Tab to land on, if it is for this file.
fn take_pending_line(
    pending: &mut Option<(ConnectionKey, WorkspaceId, RelativePathBuf, u32)>,
    key: ConnectionKey,
    workspace_id: WorkspaceId,
    path: &RelativePath,
) -> Option<u32> {
    let (pending_key, pending_workspace, pending_path, _) = pending.as_ref()?;
    if *pending_key != key || *pending_workspace != workspace_id || pending_path != path {
        return None;
    }
    pending.take().map(|(_, _, _, line)| line)
}

/// Puts the cursor at the start of the 0-based `line`, clamped to the text; Kit scrolls
/// the cursor into view and focuses the Editor.
fn land_on_line(
    state: &mut EditorState,
    line: u32,
    window: &mut Window,
    cx: &mut Context<EditorState>,
) {
    let last = state.text().lines_len().saturating_sub(1) as u32;
    state.set_cursor_position(Position::new(line.min(last), 0), window, cx);
}

/// The Preview Tab's Editor and what it currently shows.
pub(super) struct FileEditor {
    pub(super) state: Entity<EditorState>,
    pub(super) workspace_id: WorkspaceId,
    /// The Workspace's root when the Tab opened, so unsaved text is never saved into a
    /// different Workspace that took over its id.
    pub(super) root: PathBuf,
    /// What a save starts from (ADR 0045); `None` keeps the Editor read-only.
    pub(super) version: Option<u64>,
    /// The message key of why the Editor is read-only, for the header.
    readonly_reason: Option<&'static str>,
    /// The text as last read or saved: anything else in the Editor is unsaved.
    pub(super) base_text: String,
    pub(super) dirty: bool,
    /// The file as read had only `\r\n` line breaks, so saving keeps it that way.
    pub(super) crlf: bool,
    /// The save in flight, by request.
    pub(super) saving: Option<u64>,
    /// A different file was read while the text was unsaved.
    pub(super) disk_changed: bool,
    /// What waits on the save in flight leaving nothing unsaved.
    pub(super) after_save: Option<Continuation>,
    _changes: Subscription,
    /// The path and the generation of the cached answer the Editor text was built from.
    pub(super) shown: Option<(RelativePathBuf, u64)>,
    pub(super) content: FileViewContent,
    /// The file rendered, while it is Markdown (ADR 0037).
    pub(super) markdown: Option<Entity<TextViewState>>,
    /// Where the rendered file was read, and its block count then, while an edit to it
    /// may still reset Kit's list to the top; see `render_markdown`.
    pub(super) markdown_scroll: Rc<Cell<Option<(usize, ListOffset)>>>,
    /// A file the Tab can render shows its text instead. Each file opens rendered, except
    /// one "Show File" opens at a line.
    pub(super) source: bool,
    /// The lines the text changed against its Workspace's comparison (ADR 0050).
    pub(super) marks: change_marks::ChangeMarks,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum FileViewContent {
    Loading,
    Text,
    /// Decoded into `PreviewTabState::images`.
    Image,
    Binary,
    TooLarge {
        bytes: u64,
    },
    Failed(String),
}

impl Condr {
    /// Shows `path` in the Workspace's Preview Tab: a layout command like any other,
    /// presented once the Server confirms it.
    pub(super) fn show_file_on(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: RelativePathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Showing another file takes the Preview Tab's unsaved text away (ADR 0045).
        let preview = self
            .connection(key)
            .and_then(|connection| connection.session()?.file_tab(workspace_id))
            .filter(|tab| tab.file().is_some_and(|file| file.path() != path))
            .map(Tab::id);
        let show: Continuation = Box::new(move |this, window, cx| {
            this.send_presenting_layout_to(
                key,
                LayoutCommand::ShowViewer {
                    workspace_id,
                    viewer: Viewer::File { path },
                },
                window,
                cx,
            );
        });
        match preview {
            Some(tab_id) => self.confirm_unsaved(UnsavedScope::Tab(key, tab_id), show, window, cx),
            None => show(self, window, cx),
        }
    }

    /// The Preview Tab's body: one file's content, rendered when it can be, or why there
    /// is none.
    pub(super) fn render_file_tab(
        &self,
        view: &WindowView,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        path: &RelativePath,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editor = self.preview_tab.editors.get(&(key, tab_id));
        let content = editor.map_or(FileViewContent::Loading, |editor| editor.content.clone());
        let rendering = rendering_for(path).filter(|_| content == FileViewContent::Text);
        let source = editor.is_some_and(|editor| editor.source);
        let image = (content == FileViewContent::Image
            || (rendering == Some(Rendering::Svg) && !source))
            .then(|| {
                self.preview_tab
                    .images
                    .get(&(key, workspace_id, path.to_relative_path_buf()))
            });
        let header = self.render_file_header(
            key,
            workspace_id,
            tab_id,
            path,
            rendering,
            source,
            image.as_ref(),
            cx,
        );
        let markdown = editor
            .and_then(|editor| Some((editor.markdown.clone()?, editor.markdown_scroll.clone())))
            .filter(|_| rendering == Some(Rendering::Markdown) && !source);
        let body = match (image, markdown, &content, editor) {
            (Some(None), ..) => empty_state(t!("files.loading_image"), cx),
            (Some(Some(Ok(image))), ..) => render_image(image),
            (Some(Some(Err(NoImage::Note(note)))), ..) => empty_state(note, cx),
            (Some(Some(Err(NoImage::Failed(reason)))), ..) => failed_state(reason, cx),
            (None, Some((markdown, scroll)), ..) => {
                self.render_markdown(view, key, workspace_id, path, &markdown, scroll, cx)
            }
            (None, None, FileViewContent::Text, Some(editor)) => {
                let theme = cx.theme();
                div()
                    .debug_selector(|| "file-body".into())
                    .key_context("CondrPreview")
                    .on_action(cx.listener(move |this, _: &SaveFile, window, cx| {
                        this.save_file(key, tab_id, None, window, cx);
                    }))
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    // The code theme's background: the Editor draws none of its own.
                    .when_some(
                        theme.highlight_theme.style.editor_background,
                        |this, background| this.bg(background),
                    )
                    .child(
                        Editor::new(&editor.state)
                            .readonly(editor.version.is_none())
                            .appearance(false)
                            .bordered(false)
                            .font_family(self.terminal_font.family.clone())
                            .text_size(px(self.code_font_size))
                            .h(relative(1.)),
                    )
                    .into_any_element()
            }
            (
                None,
                None,
                FileViewContent::Loading | FileViewContent::Text | FileViewContent::Image,
                _,
            ) => empty_state(t!("files.loading_file"), cx),
            (None, None, FileViewContent::Binary, _) => empty_state(t!("files.binary"), cx),
            (None, None, FileViewContent::TooLarge { bytes }, _) => {
                empty_state(too_large(Oversized::File, *bytes), cx)
            }
            (None, None, FileViewContent::Failed(reason), _) => failed_state(reason.clone(), cx),
        };
        v_flex()
            .debug_selector(|| "file-tab".into())
            .size_full()
            .child(header)
            .child(body)
            .into_any_element()
    }

    /// The path with its icon and Changes status, then what the Tab shows of it: an
    /// image's size, or the choice between a renderable file's rendering and its text.
    #[allow(clippy::too_many_arguments)]
    fn render_file_header(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        path: &RelativePath,
        rendering: Option<Rendering>,
        source: bool,
        image: Option<&Option<Result<Arc<DecodedImage>, NoImage>>>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let entry = self
            .connection(key)
            .and_then(|connection| connection.model.workspace_git.get(&workspace_id))
            .and_then(|git| git.change_entries().find(|entry| entry.path == path));
        let summary = image.and_then(|image| match image {
            Some(Ok(image)) => Some(image_summary(image)),
            _ => None,
        });
        let editor = self.preview_tab.editors.get(&(key, tab_id));
        // What the person should know about editing it, most pressing first.
        let note = editor.and_then(|editor| {
            let gone = self
                .connection(key)
                .and_then(|connection| {
                    connection
                        .resources
                        .files
                        .get(&(workspace_id, path.to_relative_path_buf()))
                })
                .is_some_and(|(_, answer)| answer.is_err());
            match () {
                _ if editor.disk_changed && gone => Some(("files.note.gone", true)),
                _ if editor.disk_changed => Some(("files.note.changed", true)),
                _ if editor.content == FileViewContent::Text => {
                    editor.readonly_reason.map(|reason| (reason, false))
                }
                _ => None,
            }
        });
        let owner = cx.weak_entity();
        let toggle_path = path.to_relative_path_buf();
        let toggle = rendering.map(|_| {
            mode_toggle(
                "file-view-mode",
                Button::new("file-view-rendered")
                    .debug_selector(|| "file-view-rendered".into())
                    .label(t!("files.mode.preview")),
                Button::new("file-view-source")
                    .debug_selector(|| "file-view-source".into())
                    .label(t!("files.mode.source")),
                source,
                move |source, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        if let Some(editor) = this.preview_tab.editors.get_mut(&(key, tab_id)) {
                            editor.source = source;
                            // Rendered draws the Editor's text, saved or not (ADR 0045).
                            if !source && editor.dirty {
                                let text = editor.state.read(cx).value();
                                if let Some(markdown) = &editor.markdown {
                                    markdown.update(cx, |state, cx| state.set_text(&text, cx));
                                } else if rendering == Some(Rendering::Svg) {
                                    let image = (key, workspace_id, toggle_path.clone());
                                    this.feed_unsaved_svg(image, text.to_string(), cx);
                                }
                            }
                            cx.notify();
                        }
                    });
                },
            )
        });
        let muted = theme.muted_foreground;
        viewer_header("file-header", path, entry.map(|entry| entry.status), cx)
            .when_some(note, |this, (note, warning)| {
                let color = if warning { theme.warning } else { muted };
                this.child(header_meta("file-note", t!(note), color))
            })
            .when_some(summary, |this, summary| {
                this.child(header_meta("file-image-summary", summary, muted))
            })
            .children(toggle)
            .into_any_element()
    }

    /// Brings the Preview Tab in line with the Server's answer for its file, and its
    /// images with theirs. Called from `rebuild_dock`, the one place that sees every
    /// presentation change.
    pub(super) fn sync_file_view(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        path: RelativePathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_file_editor(key, workspace_id, tab_id, path, window, cx);
        self.sync_change_marks(key, workspace_id, tab_id, cx);
        self.feed_images(key, workspace_id, cx);
    }

    fn sync_file_editor(
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
        let answer = connection
            .resources
            .files
            .get(&(workspace_id, path.clone()))
            .cloned();
        let root = connection
            .session()
            .and_then(|session| session.workspace(workspace_id))
            .map(|workspace| workspace.root_directory().to_path_buf())
            .unwrap_or_default();
        if answer.is_none()
            && !connection
                .resources
                .files
                .is_pending(&(workspace_id, path.clone()))
        {
            self.request_file(key, workspace_id, path.clone());
        }

        if let Some(editor) = self.preview_tab.editors.get(&(key, tab_id))
            && let Some((shown, _)) = &editor.shown
            && *shown != path
        {
            // A rename of this Client's moves the Tab and its text together once the
            // Server answers; until then the Tab keeps the text it had.
            if self.rename_in_flight(key, workspace_id, shown) {
                return;
            }
            // Another Client showed something else here (ADR 0045).
            self.keep_unsaved(key, tab_id, cx);
        }
        let editor = self
            .preview_tab
            .editors
            .entry((key, tab_id))
            .or_insert_with(|| {
                let state = cx.new(|cx| {
                    let mut state = EditorState::new(window, cx)
                        // Any language puts the Editor in code mode; the real one is set per
                        // file below, since one Tab shows many files over its life.
                        .language("text")
                        .line_number(true)
                        .soft_wrap(false)
                        .indent_guides(false)
                        .folding(false)
                        .searchable(true);
                    state.set_highlighter_factory(super::syntax::highlighter_factory(cx), cx);
                    state
                });
                let changes = cx.subscribe(&state, move |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.file_text_changed(key, tab_id, cx);
                    }
                });
                FileEditor {
                    state,
                    workspace_id,
                    root,
                    version: None,
                    readonly_reason: None,
                    base_text: String::new(),
                    dirty: false,
                    crlf: false,
                    saving: None,
                    disk_changed: false,
                    after_save: None,
                    _changes: changes,
                    shown: None,
                    content: FileViewContent::Loading,
                    markdown: None,
                    markdown_scroll: Rc::default(),
                    source: false,
                    marks: Default::default(),
                }
            });
        let Some((generation, answer)) = answer else {
            if editor
                .shown
                .as_ref()
                .is_none_or(|(shown, _)| *shown != path)
            {
                editor.content = FileViewContent::Loading;
                editor.shown = None;
                editor.markdown = None;
                editor.source = false;
                editor.version = None;
                // The images of the file shown before go with it.
                let shown = (key, workspace_id, path.clone());
                release_images(self.preview_tab.images.retain_shown(&shown), cx);
            }
            return;
        };
        if editor.shown.as_ref() == Some(&(path.clone(), generation)) {
            if editor.content == FileViewContent::Text
                && let Some(line) =
                    take_pending_line(&mut self.preview_tab.pending_line, key, workspace_id, &path)
            {
                editor.source = true;
                editor
                    .state
                    .update(cx, |state, cx| land_on_line(state, line, window, cx));
            }
            return;
        }
        let shown_path = path.clone();
        let same_file = editor
            .shown
            .as_ref()
            .is_some_and(|(shown, _)| *shown == path);
        let (version, lossy) = match &answer {
            Ok(FileContent::Text { version, lossy, .. }) => (*version, *lossy),
            _ => (None, false),
        };
        if same_file {
            // A read that overlapped a save may hold the file half written; the save's
            // reply reads it again.
            if editor.saving.is_some() {
                return;
            }
            // `WorkspaceFilesChanged` names no file, so only a different version says
            // this one changed. Unsaved text is never replaced, nor its version moved on.
            let unchanged = version.is_some() && version == editor.version;
            if unchanged || editor.dirty {
                editor.shown = Some((path, generation));
                editor.disk_changed |= !unchanged;
                return;
            }
        }
        editor.shown = Some((path, generation));
        let (text, content) = match answer {
            Err(reason) => (String::new(), FileViewContent::Failed(reason)),
            Ok(FileContent::Image { .. }) => (String::new(), FileViewContent::Image),
            Ok(FileContent::Binary) => (String::new(), FileViewContent::Binary),
            Ok(FileContent::TooLarge { bytes }) => {
                (String::new(), FileViewContent::TooLarge { bytes })
            }
            Ok(FileContent::Text { text, .. }) => (text, FileViewContent::Text),
        };
        let text_to_text = same_file
            && editor.content == FileViewContent::Text
            && content == FileViewContent::Text;
        editor.version = version.filter(|_| content == FileViewContent::Text);
        editor.readonly_reason = match (&content, version) {
            (FileViewContent::Text, Some(_)) => None,
            (FileViewContent::Text, None) if lossy => Some("files.note.not_utf8"),
            (FileViewContent::Text, None) => Some("files.note.old_server"),
            _ => None,
        };
        editor.crlf = edits::crlf_only(&text);
        // Set before the Editor changes, so its change event finds nothing unsaved.
        editor.base_text = text.clone();
        editor.dirty = false;
        editor.disk_changed = false;
        let land = (content == FileViewContent::Text)
            .then(|| {
                take_pending_line(
                    &mut self.preview_tab.pending_line,
                    key,
                    workspace_id,
                    &shown_path,
                )
            })
            .flatten();
        if !same_file {
            editor.source = false;
            let shown = (key, workspace_id, shown_path.clone());
            release_images(self.preview_tab.images.retain_shown(&shown), cx);
        }
        if land.is_some() {
            editor.source = true;
        }
        let rendering = rendering_for(&shown_path).filter(|_| content == FileViewContent::Text);
        match (&editor.markdown, rendering) {
            // An agent editing the file the Tab shows: the view keeps where it was read.
            (Some(markdown), Some(Rendering::Markdown)) if text_to_text => {
                let list = markdown.read(cx).list_state();
                editor
                    .markdown_scroll
                    .set(Some((list.item_count(), list.logical_scroll_top())));
                markdown.update(cx, |state, cx| state.set_text(&text, cx));
            }
            (_, Some(Rendering::Markdown)) => {
                editor.markdown_scroll.set(None);
                editor.markdown = Some(cx.new(|cx| TextViewState::markdown(&text, cx)));
            }
            _ => editor.markdown = None,
        }
        if content == FileViewContent::Image || rendering == Some(Rendering::Svg) {
            self.preview_tab
                .images
                .want((key, workspace_id, shown_path.clone()));
        }
        editor.content = content;
        let language = language_for(&shown_path);
        editor.state.update(cx, |state, cx| {
            if text_to_text {
                // The file the Tab shows changed under it, as when an agent edits it: an
                // edit to the open document, not a new one. Kit keeps the highlighter and
                // its old tree, so the colours stay while the new parse runs, and the
                // viewport stays where it was instead of jumping to the top.
                // shortcut: each update still lands in Kit's undo history (up to 1000 steps),
                // which has no public clear; replacing only the changed span keeps a step the
                // size of the edit, not the document twice. Clear it once Kit offers a way.
                let scroll = state.scroll_offset();
                let (range, replacement) = changed_span(&state.value(), &text);
                if !range.is_empty() || !replacement.is_empty() {
                    state.set_selected_range(range, cx);
                    state.replace(replacement.to_owned(), window, cx);
                }
                state.set_selected_range(0..0, cx);
                state.set_scroll_offset(scroll, cx);
            } else {
                state.set_highlighter(language, cx);
                state.set_value(text, window, cx);
            }
            if let Some(line) = land {
                land_on_line(state, line, window, cx);
            }
        });
        // `set_value` emits no change event.
        self.schedule_change_marks(key, tab_id, cx);
    }

    /// The Editor's text moved: it is unsaved while it differs from what was read.
    fn file_text_changed(&mut self, key: ConnectionKey, tab_id: TabId, cx: &mut Context<Self>) {
        self.schedule_change_marks(key, tab_id, cx);
        let Some(editor) = self.preview_tab.editors.get_mut(&(key, tab_id)) else {
            return;
        };
        let dirty =
            editor.version.is_some() && editor.state.read(cx).value().as_ref() != editor.base_text;
        if dirty != editor.dirty {
            editor.dirty = dirty;
            cx.notify();
        }
    }

    pub(super) fn request_file(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: RelativePathBuf,
    ) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if let Some(request_id) =
            connection.send_request(
                |server_id, session_id, request_id| ClientMessage::ReadFile {
                    server_id,
                    session_id,
                    request_id,
                    workspace_id,
                    path: path.clone(),
                },
            )
        {
            connection
                .resources
                .files
                .asked((workspace_id, path), request_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PreviewTabState, changed_span};
    use condr_core::Session;

    #[test]
    fn a_changed_file_replaces_only_the_span_that_differs() {
        let span = |old, new| {
            let (range, text) = changed_span(old, new);
            let mut patched = String::from(old);
            patched.replace_range(range.clone(), text);
            assert_eq!(patched, new);
            (range, text)
        };
        assert_eq!(span("fn a() {}\n", "fn b() {}\n"), (3..4, "b"));
        assert_eq!(span("one\nthree\n", "one\ntwo\nthree\n"), (5..5, "wo\nt"));
        assert_eq!(span("aaa", "aa"), (2..3, ""));
        assert_eq!(span("same", "same"), (4..4, ""));
        // é and è share their first byte: the span widens to the whole character.
        assert_eq!(span("caé!", "caè!"), (2..4, "è"));
        assert_eq!(span("", "新"), (0..0, "新"));
    }

    #[test]
    fn closing_a_workspace_drops_the_line_its_preview_tab_was_to_land_on() {
        let mut session = Session::new();
        session.create_workspace(std::env::temp_dir()).unwrap();
        let id = session.workspaces()[0].id();
        let mut state = PreviewTabState {
            pending_line: Some((1, id, "README.md".into(), 10)),
            ..PreviewTabState::default()
        };
        state.retain_session(2, None);
        assert!(
            state.pending_line.is_some(),
            "another Device's loss keeps it"
        );
        session.close_workspace(id).unwrap();
        state.retain_session(1, Some(&session));
        assert!(state.pending_line.is_none());
    }

    #[test]
    fn closing_the_preview_tab_lets_its_images_go() {
        let mut session = Session::new();
        session.create_workspace(std::env::temp_dir()).unwrap();
        let id = session.workspaces()[0].id();
        let preview = session
            .show_viewer(
                id,
                condr_core::Viewer::File {
                    path: "logo.png".into(),
                },
            )
            .unwrap();
        let mut view = PreviewTabState::default();
        assert!(view.images.want((1, id, "logo.png".into())));
        view.retain_session(1, Some(&session));
        assert!(
            !view.images.want((1, id, "logo.png".into())),
            "the open Preview Tab keeps its image"
        );
        session.close_tab(preview).unwrap();
        view.retain_session(1, Some(&session));
        assert!(
            view.images.want((1, id, "logo.png".into())),
            "the closed Preview Tab's image was dropped"
        );
    }
}
