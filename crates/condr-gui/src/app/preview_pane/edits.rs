//! Editing in the Preview Pane (ADR 0045): saving from the version the text started from,
//! the conflict when the file moved on meanwhile, and unsaved text its Pane is about to
//! lose, asked about before this Client's own action or at once after another's.

use super::*;
use condr_core::{FileMutation, FileMutationResult, MAX_FILE_BYTES};
use rust_i18n::t;

pub(in crate::app) type Continuation =
    Box<dyn FnOnce(&mut Condr, &mut Window, &mut Context<Condr>)>;
type Action = Rc<dyn Fn(&mut Condr, &mut Window, &mut Context<Condr>)>;

/// A save in flight on one connection.
pub(in crate::app) struct PendingSave {
    pub(in crate::app) workspace_id: WorkspaceId,
    pub(in crate::app) path: RelativePathBuf,
    /// The Editor's text as sent, before its line endings were converted.
    pub(in crate::app) text: String,
    pub(in crate::app) owner: SaveOwner,
}

pub(in crate::app) enum SaveOwner {
    Pane(PaneId),
    /// Text whose Pane is gone; it comes back to the person if the save does not land.
    Unsaved(UnsavedText),
}

/// Text a Preview Pane lost before it was saved.
pub(in crate::app) struct UnsavedText {
    pub(in crate::app) key: ConnectionKey,
    pub(in crate::app) workspace_id: WorkspaceId,
    /// The Workspace's root when the text was read, so a reused id is not taken for it.
    pub(in crate::app) root: PathBuf,
    pub(in crate::app) path: RelativePathBuf,
    pub(in crate::app) text: String,
    pub(in crate::app) version: Option<u64>,
    pub(in crate::app) crlf: bool,
    /// The last save met a newer file: saving again overwrites it.
    pub(in crate::app) conflicted: bool,
}

/// A question that arrived without a window to ask it in, asked on the next render.
pub(in crate::app) enum FilePrompt {
    Unsaved(UnsavedText),
    Conflict {
        key: ConnectionKey,
        pane_id: PaneId,
        after: Option<Continuation>,
    },
}

/// Which Preview Panes an action of this Client is about to take away.
#[derive(Clone, Copy)]
pub(in crate::app) enum UnsavedScope {
    Pane(ConnectionKey, PaneId),
    Tab(ConnectionKey, TabId),
    Workspace(ConnectionKey, WorkspaceId),
    Device(ConnectionKey),
    All,
}

impl UnsavedScope {
    /// Whether the Preview Pane `pane_id`, in Tab `tab_id` of `workspace_id`, goes too.
    fn covers(
        self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        pane_id: PaneId,
        tab_id: Option<TabId>,
    ) -> bool {
        match self {
            Self::Pane(scope_key, scope_pane) => key == scope_key && pane_id == scope_pane,
            Self::Tab(scope_key, scope_tab) => key == scope_key && tab_id == Some(scope_tab),
            Self::Workspace(scope_key, scope_workspace) => {
                key == scope_key && workspace_id == scope_workspace
            }
            Self::Device(scope_key) => key == scope_key,
            Self::All => true,
        }
    }
}

/// The file as read had `\r\n` and no other line break, so saving keeps it that way.
pub(in crate::app) fn crlf_only(text: &str) -> bool {
    let crlf = text.matches("\r\n").count();
    crlf > 0 && text.matches('\n').count() == crlf && text.matches('\r').count() == crlf
}

/// `text` with each bare `\n`, as Enter inserts it, written as `\r\n`.
pub(in crate::app) fn to_crlf(text: &str) -> String {
    let mut converted = String::with_capacity(text.len() + text.len() / 16);
    let mut previous = None;
    for character in text.chars() {
        if character == '\n' && previous != Some('\r') {
            converted.push('\r');
        }
        converted.push(character);
        previous = Some(character);
    }
    converted
}

#[derive(Clone, Copy)]
pub(in crate::app) enum ChoiceStyle {
    Primary,
    Danger,
    Plain,
}

#[derive(Clone)]
pub(in crate::app) struct Choice {
    /// Names the button's debug selector, which the translated label cannot.
    id: &'static str,
    label: SharedString,
    style: ChoiceStyle,
    enabled: bool,
    act: Action,
}

impl Choice {
    pub(in crate::app) fn new(
        id: &'static str,
        label: impl Into<SharedString>,
        style: ChoiceStyle,
        enabled: bool,
        act: impl Fn(&mut Condr, &mut Window, &mut Context<Condr>) + 'static,
    ) -> Self {
        Self {
            id,
            label: label.into(),
            style,
            enabled,
            act: Rc::new(act),
        }
    }
}

/// A continuation the dialog's buttons share; whichever runs first takes it.
pub(in crate::app) fn shared(then: Continuation) -> Rc<RefCell<Option<Continuation>>> {
    Rc::new(RefCell::new(Some(then)))
}

impl Condr {
    /// A dialog that must be answered by one of `choices`: unsaved text is never dropped
    /// by a click beside it or Escape.
    pub(in crate::app) fn open_choice(
        &self,
        title: String,
        description: String,
        choices: Vec<Choice>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owner = cx.weak_entity();
        let choices = Rc::new(choices);
        window.defer(cx, move |window, cx| {
            window.open_dialog(cx, move |dialog, _, _| {
                let mut footer = DialogFooter::new();
                for (index, choice) in choices.iter().enumerate() {
                    let owner = owner.clone();
                    let act = choice.act.clone();
                    let id = choice.id;
                    let button = Button::new(("file-choice", index))
                        .debug_selector(move || format!("file-choice-{id}"))
                        .label(choice.label.clone())
                        .disabled(!choice.enabled)
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            let _ = owner.update(cx, |this, cx| act(this, window, cx));
                        });
                    footer = footer.child(match choice.style {
                        ChoiceStyle::Primary => button.primary(),
                        ChoiceStyle::Danger => button.danger(),
                        ChoiceStyle::Plain => button,
                    });
                }
                let description = description.clone();
                dialog
                    .title(title.clone())
                    .overlay_closable(false)
                    .keyboard(false)
                    .close_button(false)
                    .content(move |content, _, cx| {
                        content.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(description.clone()),
                        )
                    })
                    .footer(footer)
            });
        });
    }

    /// Sends one file mutation; `None` when it could not be sent, which is reported.
    pub(in crate::app) fn send_file_mutation(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        mutation: FileMutation,
        cx: &mut App,
    ) -> Option<u64> {
        let connection = self.connection_mut(key)?;
        let request_id = connection.send_request(|server_id, session_id, request_id| {
            ClientMessage::MutateFile {
                server_id,
                session_id,
                request_id,
                workspace_id,
                mutation,
            }
        });
        if request_id.is_none() {
            let label = connection.label.clone();
            self.report_error(t!("files.not_connected", device = label), cx);
        }
        request_id
    }

    /// Writes `text` to `path`, its line endings converted when the file had only `\r\n`.
    #[allow(clippy::too_many_arguments)]
    fn send_write(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: RelativePathBuf,
        text: String,
        crlf: bool,
        base: Option<u64>,
        owner: SaveOwner,
        cx: &mut App,
    ) -> Option<u64> {
        let bytes = if crlf { to_crlf(&text) } else { text.clone() };
        // A frame too large to encode would drop the whole connection.
        if bytes.len() as u64 > MAX_FILE_BYTES {
            self.report_error(
                t!(
                    "files.save_too_large",
                    path = path,
                    size = MAX_FILE_BYTES / 1024
                ),
                cx,
            );
            if let SaveOwner::Unsaved(unsaved) = owner {
                self.preview_pane
                    .prompts
                    .push_back(FilePrompt::Unsaved(unsaved));
            }
            return None;
        }
        let request_id = self.send_file_mutation(
            key,
            workspace_id,
            FileMutation::Write {
                path: path.clone(),
                text: bytes,
                base,
            },
            cx,
        );
        match request_id {
            Some(request_id) => {
                self.preview_pane.saves.insert(
                    (key, request_id),
                    PendingSave {
                        workspace_id,
                        path,
                        text,
                        owner,
                    },
                );
            }
            None => {
                if let SaveOwner::Unsaved(unsaved) = owner {
                    self.preview_pane
                        .prompts
                        .push_back(FilePrompt::Unsaved(unsaved));
                }
            }
        }
        request_id
    }

    /// Saves the Preview Pane's text, then runs `then` once the save leaves nothing unsaved.
    pub(in crate::app) fn save_file(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        then: Option<Continuation>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.write_file(key, pane_id, true, then, window, cx);
    }

    fn write_file(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        versioned: bool,
        then: Option<Continuation>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.preview_pane.editors.get(&(key, pane_id)) else {
            return;
        };
        // ponytail: a second Save while one runs is dropped; the first one's reply decides.
        if editor.saving.is_some() {
            return;
        }
        let (Some(base), Some((path, _))) = (editor.version, editor.shown.clone()) else {
            return;
        };
        if !editor.dirty && versioned {
            if let Some(then) = then {
                then(self, window, cx);
            }
            return;
        }
        let text = editor.state.read(cx).value().to_string();
        let (workspace_id, crlf) = (editor.workspace_id, editor.crlf);
        let request_id = self.send_write(
            key,
            workspace_id,
            path,
            text,
            crlf,
            versioned.then_some(base),
            SaveOwner::Pane(pane_id),
            cx,
        );
        if let Some(editor) = self.preview_pane.editors.get_mut(&(key, pane_id))
            && let Some(request_id) = request_id
        {
            editor.saving = Some(request_id);
            editor.after_save = then;
            cx.notify();
        }
    }

    /// The Server's answer to a write, create, rename or delete.
    pub(in crate::app) fn file_mutated(
        &mut self,
        key: ConnectionKey,
        request_id: u64,
        result: FileMutationResult,
        cx: &mut Context<Self>,
    ) {
        if let Some(save) = self.preview_pane.saves.remove(&(key, request_id)) {
            self.file_saved(key, request_id, save, result, cx);
        } else {
            self.entry_mutated(key, request_id, result, cx);
        }
    }

    fn file_saved(
        &mut self,
        key: ConnectionKey,
        request_id: u64,
        save: PendingSave,
        result: FileMutationResult,
        cx: &mut Context<Self>,
    ) {
        let pane_id = match save.owner {
            SaveOwner::Pane(pane_id) => pane_id,
            SaveOwner::Unsaved(mut unsaved) => {
                match result {
                    FileMutationResult::Done { .. } => {}
                    FileMutationResult::Conflict => {
                        unsaved.conflicted = true;
                        self.preview_pane
                            .prompts
                            .push_back(FilePrompt::Unsaved(unsaved));
                    }
                    FileMutationResult::Invalid(error) | FileMutationResult::Failed(error) => {
                        self.report_error(
                            t!("files.save_failed", path = save.path, error = error),
                            cx,
                        );
                        self.preview_pane
                            .prompts
                            .push_back(FilePrompt::Unsaved(unsaved));
                    }
                }
                cx.notify();
                return;
            }
        };
        let Some(editor) = self
            .preview_pane
            .editors
            .get_mut(&(key, pane_id))
            .filter(|editor| editor.saving == Some(request_id))
        else {
            return;
        };
        editor.saving = None;
        let after = editor.after_save.take();
        match result {
            FileMutationResult::Done { version } => {
                editor.version = version;
                editor.disk_changed = false;
                editor.dirty = editor.state.read(cx).value().as_ref() != save.text;
                editor.base_text = save.text;
                // Reads that overlapped the save were dropped: read the file once more.
                self.request_file(key, save.workspace_id, save.path);
                if let Some(after) = after {
                    self.preview_pane.deferred.push(after);
                }
            }
            FileMutationResult::Conflict => {
                self.preview_pane.prompts.push_back(FilePrompt::Conflict {
                    key,
                    pane_id,
                    after,
                });
            }
            FileMutationResult::Invalid(error) | FileMutationResult::Failed(error) => {
                self.report_error(t!("files.save_failed", path = save.path, error = error), cx);
            }
        }
        cx.notify();
    }

    /// A connection that dropped will not answer its saves: the text stays unsaved, and
    /// text whose Pane was already gone is asked about again.
    pub(in crate::app) fn forget_file_saves(&mut self, key: ConnectionKey) {
        let lost: Vec<_> = self
            .preview_pane
            .saves
            .keys()
            .filter(|(save_key, _)| *save_key == key)
            .copied()
            .collect();
        for slot in lost {
            let Some(save) = self.preview_pane.saves.remove(&slot) else {
                continue;
            };
            match save.owner {
                SaveOwner::Pane(pane_id) => {
                    if let Some(editor) = self.preview_pane.editors.get_mut(&(key, pane_id)) {
                        editor.saving = None;
                        editor.after_save = None;
                    }
                }
                SaveOwner::Unsaved(unsaved) => self
                    .preview_pane
                    .prompts
                    .push_back(FilePrompt::Unsaved(unsaved)),
            }
        }
        self.forget_entry_requests(key);
    }

    /// Runs what waited for a window and asks what arrived without one, one dialog at a
    /// time. Called from `render`, so the work itself is deferred past the frame.
    pub(in crate::app) fn ask_file_prompts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let idle = self.preview_pane.prompt_open || self.preview_pane.prompts.is_empty();
        if self.preview_pane.deferred.is_empty() && idle {
            return;
        }
        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let _ = owner.update(cx, |this, cx| this.ask_file_prompts_now(window, cx));
        });
    }

    fn ask_file_prompts_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for deferred in std::mem::take(&mut self.preview_pane.deferred) {
            deferred(self, window, cx);
        }
        if self.preview_pane.prompt_open {
            return;
        }
        let Some(prompt) = self.preview_pane.prompts.pop_front() else {
            return;
        };
        self.preview_pane.prompt_open = true;
        match prompt {
            FilePrompt::Unsaved(unsaved) => self.ask_unsaved(unsaved, window, cx),
            FilePrompt::Conflict {
                key,
                pane_id,
                after,
            } => self.ask_conflict(key, pane_id, after, window, cx),
        }
    }

    fn prompt_answered(&mut self, cx: &mut Context<Self>) {
        self.preview_pane.prompt_open = false;
        cx.notify();
    }

    /// The file changed on disk since the Editor read it.
    fn ask_conflict(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        after: Option<Continuation>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, _)) = self
            .preview_pane
            .editors
            .get(&(key, pane_id))
            .and_then(|editor| editor.shown.clone())
        else {
            self.prompt_answered(cx);
            return;
        };
        let after = Rc::new(RefCell::new(after));
        let (overwrite, reload) = (after.clone(), after);
        self.open_choice(
            t!("files.conflict.title", path = path).into(),
            t!("files.conflict.description").into(),
            vec![
                Choice::new(
                    "cancel",
                    t!("files.choice.cancel"),
                    ChoiceStyle::Plain,
                    true,
                    |this, _, cx| this.prompt_answered(cx),
                ),
                Choice::new(
                    "reload",
                    t!("files.choice.discard_and_reload"),
                    ChoiceStyle::Plain,
                    true,
                    move |this, window, cx| {
                        this.prompt_answered(cx);
                        this.discard_file_edits(key, pane_id, cx);
                        if let Some(after) = reload.borrow_mut().take() {
                            after(this, window, cx);
                        }
                    },
                ),
                Choice::new(
                    "overwrite",
                    t!("files.choice.overwrite"),
                    ChoiceStyle::Danger,
                    true,
                    move |this, window, cx| {
                        this.prompt_answered(cx);
                        let after = overwrite.borrow_mut().take();
                        this.write_file(key, pane_id, false, after, window, cx);
                    },
                ),
            ],
            window,
            cx,
        );
    }

    /// Drops the Editor's edits and shows the file as it now is.
    fn discard_file_edits(&mut self, key: ConnectionKey, pane_id: PaneId, cx: &mut Context<Self>) {
        let Some(editor) = self.preview_pane.editors.get_mut(&(key, pane_id)) else {
            return;
        };
        editor.dirty = false;
        editor.disk_changed = false;
        let Some((path, _)) = editor.shown.clone() else {
            return;
        };
        // No answer has generation 0, so the next one applies whatever it holds.
        editor.shown = Some((path.clone(), 0));
        editor.version = None;
        let workspace_id = editor.workspace_id;
        self.request_file(key, workspace_id, path);
        cx.notify();
    }

    /// Text whose Pane another Client or a reconnect took away: save it where it was read,
    /// copy it, or let it go.
    fn ask_unsaved(&mut self, unsaved: UnsavedText, window: &mut Window, cx: &mut Context<Self>) {
        let can_save = unsaved.version.is_some()
            && self.connection(unsaved.key).is_some_and(|connection| {
                connection.status == ConnectionStatus::Connected
                    && connection
                        .session()
                        .and_then(|session| session.workspace(unsaved.workspace_id))
                        .is_some_and(|workspace| workspace.root_directory() == unsaved.root)
            });
        let title = t!("files.unsaved.title", path = unsaved.path).into_owned();
        let description = if unsaved.conflicted {
            t!("files.unsaved.conflicted")
        } else {
            t!("files.unsaved.orphaned")
        };
        let (save_id, save_label) = if unsaved.conflicted {
            ("overwrite", t!("files.choice.overwrite"))
        } else {
            ("save", t!("files.choice.save"))
        };
        let text = unsaved.text.clone();
        let unsaved = Rc::new(RefCell::new(Some(unsaved)));
        self.open_choice(
            title,
            description.into(),
            vec![
                Choice::new(
                    "discard",
                    t!("files.choice.discard"),
                    ChoiceStyle::Plain,
                    true,
                    |this, _, cx| this.prompt_answered(cx),
                ),
                Choice::new(
                    "copy",
                    t!("files.choice.copy"),
                    ChoiceStyle::Plain,
                    true,
                    move |this, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                        this.prompt_answered(cx);
                    },
                ),
                Choice::new(
                    save_id,
                    save_label,
                    ChoiceStyle::Primary,
                    can_save,
                    move |this, _, cx| {
                        this.prompt_answered(cx);
                        let Some(unsaved) = unsaved.borrow_mut().take() else {
                            return;
                        };
                        let base = if unsaved.conflicted {
                            None
                        } else {
                            unsaved.version
                        };
                        this.send_write(
                            unsaved.key,
                            unsaved.workspace_id,
                            unsaved.path.clone(),
                            unsaved.text.clone(),
                            unsaved.crlf,
                            base,
                            SaveOwner::Unsaved(unsaved),
                            cx,
                        );
                    },
                ),
            ],
            window,
            cx,
        );
    }

    /// Runs `then` once no Preview Pane in `scope` holds unsaved text, asking about each
    /// in turn: save, don't save, or cancel the whole action.
    pub(in crate::app) fn confirm_unsaved(
        &mut self,
        scope: UnsavedScope,
        then: Continuation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dirty = self
            .preview_pane
            .editors
            .iter()
            .find(|((key, pane_id), editor)| {
                let tab_id = self
                    .connection(*key)
                    .and_then(|connection| connection.session()?.tab_for_pane(*pane_id))
                    .map(Tab::id);
                editor.dirty && scope.covers(*key, editor.workspace_id, *pane_id, tab_id)
            })
            .and_then(|((key, pane_id), editor)| {
                Some((*key, *pane_id, editor.shown.as_ref()?.0.clone()))
            });
        let Some((key, pane_id, path)) = dirty else {
            then(self, window, cx);
            return;
        };
        let connected = self
            .connection(key)
            .is_some_and(|connection| connection.status == ConnectionStatus::Connected);
        let then = shared(then);
        let (save, skip) = (then.clone(), then.clone());
        let resume = move |then: &Rc<RefCell<Option<Continuation>>>| -> Option<Continuation> {
            let then = then.borrow_mut().take()?;
            Some(Box::new(
                move |this: &mut Condr, window: &mut Window, cx: &mut Context<Condr>| {
                    this.confirm_unsaved(scope, then, window, cx)
                },
            ) as Continuation)
        };
        let save_choice = if connected {
            Choice::new(
                "save",
                t!("files.choice.save"),
                ChoiceStyle::Primary,
                true,
                move |this, window, cx| {
                    let after = resume(&save);
                    this.save_file(key, pane_id, after, window, cx);
                },
            )
        } else {
            Choice::new(
                "copy",
                t!("files.choice.copy"),
                ChoiceStyle::Primary,
                true,
                move |this, window, cx| {
                    if let Some(editor) = this.preview_pane.editors.get_mut(&(key, pane_id)) {
                        let text = editor.state.read(cx).value().to_string();
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        editor.dirty = false;
                    }
                    if let Some(after) = resume(&save) {
                        after(this, window, cx);
                    }
                },
            )
        };
        self.open_choice(
            t!("files.save_prompt.title", path = path).into(),
            if connected {
                t!("files.save_prompt.connected")
            } else {
                t!("files.save_prompt.disconnected")
            }
            .into(),
            vec![
                Choice::new(
                    "cancel",
                    t!("files.choice.cancel"),
                    ChoiceStyle::Plain,
                    true,
                    |_, _, _| {},
                ),
                Choice::new(
                    "dont-save",
                    t!("files.choice.dont_save"),
                    ChoiceStyle::Plain,
                    true,
                    move |this, window, cx| {
                        if let Some(editor) = this.preview_pane.editors.get_mut(&(key, pane_id)) {
                            editor.dirty = false;
                        }
                        if let Some(after) = resume(&skip) {
                            after(this, window, cx);
                        }
                    },
                ),
                save_choice,
            ],
            window,
            cx,
        );
    }

    /// The text of a Preview Pane that is going away without this Client's say, kept for
    /// the question `ask_file_prompts` puts.
    pub(in crate::app) fn keep_unsaved(&mut self, key: ConnectionKey, pane_id: PaneId, cx: &App) {
        let Some(editor) = self.preview_pane.editors.get_mut(&(key, pane_id)) else {
            return;
        };
        if !editor.dirty {
            return;
        }
        editor.dirty = false;
        let Some((path, _)) = editor.shown.clone() else {
            return;
        };
        let unsaved = UnsavedText {
            key,
            workspace_id: editor.workspace_id,
            root: editor.root.clone(),
            path,
            text: editor.state.read(cx).value().to_string(),
            version: editor.version,
            crlf: editor.crlf,
            conflicted: false,
        };
        self.preview_pane
            .prompts
            .push_back(FilePrompt::Unsaved(unsaved));
    }
}

#[cfg(test)]
mod tests {
    use super::{crlf_only, to_crlf};

    #[test]
    fn only_a_file_of_crlf_alone_gets_its_new_lines_converted() {
        assert!(crlf_only("one\r\ntwo\r\n"));
        assert!(crlf_only("\u{feff}one\r\ntwo"));
        for text in ["", "one line", "one\ntwo\n", "one\r\ntwo\n", "one\rtwo\r\n"] {
            assert!(!crlf_only(text), "{text:?}");
        }
        assert_eq!(to_crlf("one\r\ntwo\nthree\n"), "one\r\ntwo\r\nthree\r\n");
        assert_eq!(to_crlf("\n"), "\r\n");
        assert_eq!(to_crlf("no break"), "no break");
    }
}
