//! Editing in the Preview Tab (ADR 0045): saving from the version the text started from,
//! the conflict when the file moved on meanwhile, and unsaved text its Tab is about to
//! lose, asked about before this Client's own action or at once after another's.

use super::*;
use condr_core::{FileMutation, FileMutationResult, MAX_FILE_BYTES};

pub(super) type Continuation = Box<dyn FnOnce(&mut Condr, &mut Window, &mut Context<Condr>)>;
type Action = Rc<dyn Fn(&mut Condr, &mut Window, &mut Context<Condr>)>;

/// A save in flight on one connection.
pub(super) struct PendingSave {
    pub(super) workspace_id: WorkspaceId,
    pub(super) path: RelativePathBuf,
    /// The Editor's text as sent, before its line endings were converted.
    pub(super) text: String,
    pub(super) owner: SaveOwner,
}

pub(super) enum SaveOwner {
    Tab(TabId),
    /// Text whose Tab is gone; it comes back to the person if the save does not land.
    Unsaved(UnsavedText),
}

/// Text a Preview Tab lost before it was saved.
pub(super) struct UnsavedText {
    pub(super) key: ConnectionKey,
    pub(super) workspace_id: WorkspaceId,
    /// The Workspace's root when the text was read, so a reused id is not taken for it.
    pub(super) root: PathBuf,
    pub(super) path: RelativePathBuf,
    pub(super) text: String,
    pub(super) version: Option<u64>,
    pub(super) crlf: bool,
    /// The last save met a newer file: saving again overwrites it.
    pub(super) conflicted: bool,
}

/// A question that arrived without a window to ask it in, asked on the next render.
pub(super) enum FilePrompt {
    Unsaved(UnsavedText),
    Conflict {
        key: ConnectionKey,
        tab_id: TabId,
        after: Option<Continuation>,
    },
}

/// Which Preview Tabs an action of this Client is about to take away.
#[derive(Clone, Copy)]
pub(super) enum UnsavedScope {
    Tab(ConnectionKey, TabId),
    Workspace(ConnectionKey, WorkspaceId),
    Device(ConnectionKey),
    All,
}

impl UnsavedScope {
    fn covers(self, key: ConnectionKey, workspace_id: WorkspaceId, tab_id: TabId) -> bool {
        match self {
            Self::Tab(scope_key, scope_tab) => key == scope_key && tab_id == scope_tab,
            Self::Workspace(scope_key, scope_workspace) => {
                key == scope_key && workspace_id == scope_workspace
            }
            Self::Device(scope_key) => key == scope_key,
            Self::All => true,
        }
    }
}

/// The file as read had `\r\n` and no other line break, so saving keeps it that way.
pub(super) fn crlf_only(text: &str) -> bool {
    let crlf = text.matches("\r\n").count();
    crlf > 0 && text.matches('\n').count() == crlf && text.matches('\r').count() == crlf
}

/// `text` with each bare `\n`, as Enter inserts it, written as `\r\n`.
pub(super) fn to_crlf(text: &str) -> String {
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
enum ChoiceStyle {
    Primary,
    Danger,
    Plain,
}

#[derive(Clone)]
struct Choice {
    label: &'static str,
    style: ChoiceStyle,
    enabled: bool,
    act: Action,
}

impl Choice {
    fn new(
        label: &'static str,
        style: ChoiceStyle,
        enabled: bool,
        act: impl Fn(&mut Condr, &mut Window, &mut Context<Condr>) + 'static,
    ) -> Self {
        Self {
            label,
            style,
            enabled,
            act: Rc::new(act),
        }
    }
}

/// A continuation the dialog's buttons share; whichever runs first takes it.
fn shared(then: Continuation) -> Rc<RefCell<Option<Continuation>>> {
    Rc::new(RefCell::new(Some(then)))
}

impl Condr {
    /// A dialog that must be answered by one of `choices`: unsaved text is never dropped
    /// by a click beside it or Escape.
    fn open_choice(
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
                    let label = choice.label;
                    let button = Button::new(("file-choice", index))
                        .debug_selector(move || format!("file-choice-{label}"))
                        .label(label)
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
    pub(super) fn send_file_mutation(
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
            self.report_error(format!("{label} is not connected"), cx);
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
                format!(
                    "{path} was not saved: it is larger than {} KiB",
                    MAX_FILE_BYTES / 1024
                ),
                cx,
            );
            if let SaveOwner::Unsaved(unsaved) = owner {
                self.files_view
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
                self.files_view.saves.insert(
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
                    self.files_view
                        .prompts
                        .push_back(FilePrompt::Unsaved(unsaved));
                }
            }
        }
        request_id
    }

    /// Saves the Preview Tab's text, then runs `then` once the save leaves nothing unsaved.
    pub(super) fn save_file(
        &mut self,
        key: ConnectionKey,
        tab_id: TabId,
        then: Option<Continuation>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.write_file(key, tab_id, true, then, window, cx);
    }

    fn write_file(
        &mut self,
        key: ConnectionKey,
        tab_id: TabId,
        versioned: bool,
        then: Option<Continuation>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.files_view.file_editors.get(&(key, tab_id)) else {
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
            SaveOwner::Tab(tab_id),
            cx,
        );
        if let Some(editor) = self.files_view.file_editors.get_mut(&(key, tab_id))
            && let Some(request_id) = request_id
        {
            editor.saving = Some(request_id);
            editor.after_save = then;
            cx.notify();
        }
    }

    /// The Server's answer to a write, create, rename or delete.
    pub(super) fn file_mutated(
        &mut self,
        key: ConnectionKey,
        request_id: u64,
        result: FileMutationResult,
        cx: &mut Context<Self>,
    ) {
        if let Some(save) = self.files_view.saves.remove(&(key, request_id)) {
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
        let tab_id = match save.owner {
            SaveOwner::Tab(tab_id) => tab_id,
            SaveOwner::Unsaved(mut unsaved) => {
                match result {
                    FileMutationResult::Done { .. } => {}
                    FileMutationResult::Conflict => {
                        unsaved.conflicted = true;
                        self.files_view
                            .prompts
                            .push_back(FilePrompt::Unsaved(unsaved));
                    }
                    FileMutationResult::Invalid(error) | FileMutationResult::Failed(error) => {
                        self.report_error(format!("Couldn't save {}: {error}", save.path), cx);
                        self.files_view
                            .prompts
                            .push_back(FilePrompt::Unsaved(unsaved));
                    }
                }
                cx.notify();
                return;
            }
        };
        let Some(editor) = self
            .files_view
            .file_editors
            .get_mut(&(key, tab_id))
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
                    self.files_view.deferred.push(after);
                }
            }
            FileMutationResult::Conflict => {
                self.files_view
                    .prompts
                    .push_back(FilePrompt::Conflict { key, tab_id, after });
            }
            FileMutationResult::Invalid(error) | FileMutationResult::Failed(error) => {
                self.report_error(format!("Couldn't save {}: {error}", save.path), cx);
            }
        }
        cx.notify();
    }

    /// A connection that dropped will not answer its saves: the text stays unsaved, and
    /// text whose Tab was already gone is asked about again.
    pub(super) fn forget_file_saves(&mut self, key: ConnectionKey) {
        let lost: Vec<_> = self
            .files_view
            .saves
            .keys()
            .filter(|(save_key, _)| *save_key == key)
            .copied()
            .collect();
        for slot in lost {
            let Some(save) = self.files_view.saves.remove(&slot) else {
                continue;
            };
            match save.owner {
                SaveOwner::Tab(tab_id) => {
                    if let Some(editor) = self.files_view.file_editors.get_mut(&(key, tab_id)) {
                        editor.saving = None;
                        editor.after_save = None;
                    }
                }
                SaveOwner::Unsaved(unsaved) => self
                    .files_view
                    .prompts
                    .push_back(FilePrompt::Unsaved(unsaved)),
            }
        }
        self.forget_entry_requests(key);
    }

    /// Runs what waited for a window and asks what arrived without one, one dialog at a
    /// time. Called from `render`, so the work itself is deferred past the frame.
    pub(super) fn ask_file_prompts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let idle = self.files_view.prompt_open || self.files_view.prompts.is_empty();
        if self.files_view.deferred.is_empty() && idle {
            return;
        }
        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let _ = owner.update(cx, |this, cx| this.ask_file_prompts_now(window, cx));
        });
    }

    fn ask_file_prompts_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for deferred in std::mem::take(&mut self.files_view.deferred) {
            deferred(self, window, cx);
        }
        if self.files_view.prompt_open {
            return;
        }
        let Some(prompt) = self.files_view.prompts.pop_front() else {
            return;
        };
        self.files_view.prompt_open = true;
        match prompt {
            FilePrompt::Unsaved(unsaved) => self.ask_unsaved(unsaved, window, cx),
            FilePrompt::Conflict { key, tab_id, after } => {
                self.ask_conflict(key, tab_id, after, window, cx)
            }
        }
    }

    fn prompt_answered(&mut self, cx: &mut Context<Self>) {
        self.files_view.prompt_open = false;
        cx.notify();
    }

    /// The file changed on disk since the Editor read it.
    fn ask_conflict(
        &mut self,
        key: ConnectionKey,
        tab_id: TabId,
        after: Option<Continuation>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, _)) = self
            .files_view
            .file_editors
            .get(&(key, tab_id))
            .and_then(|editor| editor.shown.clone())
        else {
            self.prompt_answered(cx);
            return;
        };
        let after = Rc::new(RefCell::new(after));
        let (overwrite, reload) = (after.clone(), after);
        self.open_choice(
            format!("{path} changed on disk"),
            "Something else changed it since you opened it. Overwrite it with your text, or \
             discard your edits and load it again."
                .into(),
            vec![
                Choice::new("Cancel", ChoiceStyle::Plain, true, |this, _, cx| {
                    this.prompt_answered(cx)
                }),
                Choice::new(
                    "Discard and Reload",
                    ChoiceStyle::Plain,
                    true,
                    move |this, window, cx| {
                        this.prompt_answered(cx);
                        this.discard_file_edits(key, tab_id, cx);
                        if let Some(after) = reload.borrow_mut().take() {
                            after(this, window, cx);
                        }
                    },
                ),
                Choice::new(
                    "Overwrite",
                    ChoiceStyle::Danger,
                    true,
                    move |this, window, cx| {
                        this.prompt_answered(cx);
                        let after = overwrite.borrow_mut().take();
                        this.write_file(key, tab_id, false, after, window, cx);
                    },
                ),
            ],
            window,
            cx,
        );
    }

    /// Drops the Editor's edits and shows the file as it now is.
    fn discard_file_edits(&mut self, key: ConnectionKey, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(editor) = self.files_view.file_editors.get_mut(&(key, tab_id)) else {
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

    /// Text whose Tab another Client or a reconnect took away: save it where it was read,
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
        let title = format!("{} has unsaved changes", unsaved.path);
        let description = if unsaved.conflicted {
            "It changed on disk since you opened it. Overwrite it with your text, copy your \
             text, or discard it."
        } else {
            "Its Preview Tab no longer shows it. Save your text to the file, copy it, or \
             discard it."
        };
        let text = unsaved.text.clone();
        let unsaved = Rc::new(RefCell::new(Some(unsaved)));
        self.open_choice(
            title,
            description.into(),
            vec![
                Choice::new("Discard", ChoiceStyle::Plain, true, |this, _, cx| {
                    this.prompt_answered(cx)
                }),
                Choice::new("Copy", ChoiceStyle::Plain, true, move |this, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    this.prompt_answered(cx);
                }),
                Choice::new(
                    if unsaved
                        .borrow()
                        .as_ref()
                        .is_some_and(|unsaved| unsaved.conflicted)
                    {
                        "Overwrite"
                    } else {
                        "Save"
                    },
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

    /// Runs `then` once no Preview Tab in `scope` holds unsaved text, asking about each
    /// in turn: save, don't save, or cancel the whole action.
    pub(super) fn confirm_unsaved(
        &mut self,
        scope: UnsavedScope,
        then: Continuation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dirty = self
            .files_view
            .file_editors
            .iter()
            .find(|((key, tab_id), editor)| {
                editor.dirty && scope.covers(*key, editor.workspace_id, *tab_id)
            })
            .and_then(|((key, tab_id), editor)| {
                Some((*key, *tab_id, editor.shown.as_ref()?.0.clone()))
            });
        let Some((key, tab_id, path)) = dirty else {
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
                "Save",
                ChoiceStyle::Primary,
                true,
                move |this, window, cx| {
                    let after = resume(&save);
                    this.save_file(key, tab_id, after, window, cx);
                },
            )
        } else {
            Choice::new(
                "Copy",
                ChoiceStyle::Primary,
                true,
                move |this, window, cx| {
                    if let Some(editor) = this.files_view.file_editors.get_mut(&(key, tab_id)) {
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
            format!("Save changes to {path}?"),
            if connected {
                "Your changes are lost if you don't save them."
            } else {
                "The device is not connected, so the file cannot be saved now. Copy your \
                 text to keep it."
            }
            .into(),
            vec![
                Choice::new("Cancel", ChoiceStyle::Plain, true, |_, _, _| {}),
                Choice::new(
                    "Don't Save",
                    ChoiceStyle::Plain,
                    true,
                    move |this, window, cx| {
                        if let Some(editor) = this.files_view.file_editors.get_mut(&(key, tab_id)) {
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

    /// The text of a Preview Tab that is going away without this Client's say, kept for
    /// the question `ask_file_prompts` puts.
    pub(super) fn keep_unsaved(&mut self, key: ConnectionKey, tab_id: TabId, cx: &App) {
        let Some(editor) = self.files_view.file_editors.get_mut(&(key, tab_id)) else {
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
        self.files_view
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
