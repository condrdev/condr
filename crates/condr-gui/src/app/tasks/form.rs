//! The form that writes a Task in the interaction area (ADR 0068): opening it, saving,
//! leaving with changes, moving a Task to another Device, and keeping it in step with
//! the Server. Drawing it is `form_area`'s; starting, `start`'s.

use super::*;
use condr_core::{TaskCommand, TaskId};
use gpui_kit::component::input::TextareaState;
use rust_i18n::t;

/// What a saved Task held when the form last heard of it.
pub(in crate::app) struct SavedTask {
    pub(in crate::app) title: String,
    /// `None` until the form's request for it is answered.
    pub(in crate::app) description: Option<String>,
    pub(in crate::app) repository: Option<PathBuf>,
}

/// The Start half's choices; `None` follows the default.
pub(in crate::app) struct StartChoice {
    pub(in crate::app) place: Option<Where>,
    pub(in crate::app) workspace: Option<WorkspaceId>,
    pub(in crate::app) branch: Entity<InputState>,
    pub(in crate::app) agent: Option<AgentKind>,
    /// The Server's refusal of the branch, shown under its field.
    pub(in crate::app) branch_error: Option<String>,
    pub(in crate::app) folder_missing: bool,
    pub(in crate::app) request: Option<u64>,
}

/// Where a refusal of the form shows (ADR 0020).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum FormField {
    Title,
    Description,
    /// Under the form's buttons, for what no one field caused.
    Form,
}

/// The form in the interaction area's left half, for a To do Task or a new one; nothing
/// is kept until Save (ADR 0068).
pub(in crate::app) struct TaskForm {
    /// The saved Task's Device and number; `None` for a new Task, which has no card yet.
    pub(in crate::app) home: Option<(ConnectionKey, TaskId)>,
    /// The Device chosen in the form.
    pub(in crate::app) device: ConnectionKey,
    pub(in crate::app) repository: Option<PathBuf>,
    /// The repository chosen on each Device the form left, given back on returning.
    pub(in crate::app) repositories: HashMap<ConnectionKey, Option<PathBuf>>,
    pub(in crate::app) title: Entity<InputState>,
    pub(in crate::app) description: Entity<TextareaState>,
    pub(in crate::app) saved: Option<SavedTask>,
    pub(in crate::app) description_request: Option<u64>,
    /// Why the description could not be fetched; Retry asks again.
    pub(in crate::app) description_error: Option<String>,
    /// The save in flight, by Device and request.
    pub(in crate::app) saving: Option<(ConnectionKey, u64)>,
    /// Where the leave dialog's Save was going, once that save is in.
    pub(in crate::app) after_save: Option<Continuation>,
    /// The last save's refusal, under the field it is about.
    pub(in crate::app) error: Option<(FormField, String)>,
    pub(in crate::app) start: StartChoice,
}

impl TaskForm {
    fn title_text(&self, cx: &App) -> String {
        self.title.read(cx).value().trim().to_owned()
    }

    fn description_text(&self, cx: &App) -> String {
        self.description.read(cx).value().to_string()
    }

    /// Whether leaving loses something: a new Task's text, or a saved one's changes.
    pub(in crate::app) fn changed(&self, cx: &App) -> bool {
        match (&self.saved, self.home) {
            (Some(saved), Some((home, _))) => {
                self.title_text(cx) != saved.title
                    || saved
                        .description
                        .as_ref()
                        .is_some_and(|description| *description != self.description_text(cx))
                    || self.device != home
                    || self.repository != saved.repository
            }
            _ => !self.title_text(cx).is_empty() || !self.description_text(cx).trim().is_empty(),
        }
    }

    /// A title, and for a saved Task its description fetched, so a save never blanks it.
    pub(super) fn can_save(&self, cx: &App) -> bool {
        !self.title_text(cx).is_empty() && self.changed(cx) && self.description_loaded()
    }

    /// A new Task's description is there from the start; a saved one's once fetched.
    pub(in crate::app) fn description_loaded(&self) -> bool {
        self.saved
            .as_ref()
            .is_none_or(|saved| saved.description.is_some())
    }

    pub(super) fn error_for(&self, field: FormField) -> Option<String> {
        self.error
            .as_ref()
            .filter(|(at, _)| *at == field)
            .map(|(_, message)| message.clone())
    }
}

/// What the Server would refuse in a form, said under its field before anything is sent.
fn form_refusal(title: &str, description: &str) -> Option<(FormField, String)> {
    if title.chars().any(char::is_control) {
        Some((
            FormField::Title,
            t!("tasks.form.title_one_line").into_owned(),
        ))
    } else if title.len() > condr_core::MAX_TASK_TITLE_BYTES {
        Some((
            FormField::Title,
            t!(
                "tasks.form.title_too_long",
                bytes = condr_core::MAX_TASK_TITLE_BYTES
            )
            .into_owned(),
        ))
    } else if description.len() > condr_core::MAX_TASK_DESCRIPTION_BYTES {
        Some((
            FormField::Description,
            t!("tasks.form.description_too_long").into_owned(),
        ))
    } else {
        None
    }
}

impl Condr {
    /// Runs `then` once the open form may go: at once when nothing would be lost, else
    /// after asking Save, Discard or Cancel (ADR 0068).
    pub(in crate::app) fn leave_task_form(
        &mut self,
        then: Continuation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = &self.task_form else {
            return then(self, window, cx);
        };
        if !form.changed(cx) {
            self.task_form = None;
            cx.notify();
            return then(self, window, cx);
        }
        let title = match form.title_text(cx) {
            title if title.is_empty() => t!("tasks.form.new_title").into_owned(),
            title => title,
        };
        let can_save = form.can_save(cx);
        let save = shared(then);
        let discard = save.clone();
        self.open_choice(
            t!("tasks.form.unsaved.title", title = title).into_owned(),
            t!("tasks.form.unsaved.description").into_owned(),
            vec![
                Choice::new(
                    "discard",
                    t!("tasks.form.unsaved.discard"),
                    ChoiceStyle::Plain,
                    true,
                    move |this, window, cx| {
                        this.task_form = None;
                        cx.notify();
                        if let Some(then) = discard.borrow_mut().take() {
                            then(this, window, cx);
                        }
                    },
                ),
                Choice::new(
                    "cancel",
                    t!("tasks.form.cancel"),
                    ChoiceStyle::Plain,
                    true,
                    |_, _, _| {},
                ),
                Choice::new(
                    "save",
                    t!("tasks.form.save"),
                    ChoiceStyle::Primary,
                    can_save,
                    move |this, _, cx| {
                        if let Some(then) = save.borrow_mut().take() {
                            this.save_then_leave(then, cx);
                        }
                    },
                ),
            ],
            window,
            cx,
        );
    }

    /// The Command Palette's New Task and To do's "+ New task": Tasks mode with an empty
    /// form on the scope's Device, else this machine's.
    pub(in crate::app) fn new_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_main(window) {
            return;
        }
        let Some(device) = self.default_task_device() else {
            self.report_error(t!("tasks.form.no_device"), cx);
            return;
        };
        self.leave_task_form(
            Box::new(move |this, window, cx| this.open_new_task(device, window, cx)),
            window,
            cx,
        );
    }

    fn default_task_device(&self) -> Option<ConnectionKey> {
        let scope = self.effective_tasks_scope(self.main_view().tasks.scope);
        let scoped = match scope {
            super::tasks::TasksScope::All => None,
            super::tasks::TasksScope::Device(server)
            | super::tasks::TasksScope::Workspace(server, _) => Some(server),
        };
        let devices: Vec<&ServerConnection> = self.task_devices().collect();
        devices
            .iter()
            .find(|connection| scoped.is_some() && connection.model.server_id == scoped)
            .or_else(|| {
                devices
                    .iter()
                    .find(|connection| connection.endpoint.as_local_path().is_some())
            })
            .or(devices.first())
            .map(|connection| connection.key)
    }

    /// The repository a new Task in the scope's Workspace belongs to.
    fn scope_repository(&self, device: ConnectionKey) -> Option<PathBuf> {
        let super::tasks::TasksScope::Workspace(server, workspace_id) =
            self.effective_tasks_scope(self.main_view().tasks.scope)
        else {
            return None;
        };
        let connection = self.connection(device)?;
        if connection.model.server_id != Some(server) {
            return None;
        }
        let session = connection.session()?;
        let workspace = session.workspace(workspace_id)?;
        let workspace = match workspace.worktree() {
            Some(association) => session.workspace(association.parent_workspace_id())?,
            None => workspace,
        };
        repository_workspaces(connection)
            .any(|(id, _, _)| id == workspace.id())
            .then(|| workspace.root_directory().to_path_buf())
    }

    fn open_new_task(
        &mut self,
        device: ConnectionKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.main_view().page != ContentPage::Tasks {
            self.toggle_tasks(window, cx);
        }
        self.clear_task_selection(window, cx);
        let repository = self.scope_repository(device);
        let form = self.make_task_form(
            None,
            device,
            repository,
            "",
            None,
            String::new(),
            window,
            cx,
        );
        let title = form.title.clone();
        self.task_form = Some(form);
        title.update(cx, |input, cx| input.focus(window, cx));
        self.ask_task_agents(device);
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn make_task_form(
        &mut self,
        home: Option<(ConnectionKey, TaskId)>,
        device: ConnectionKey,
        repository: Option<PathBuf>,
        title: &str,
        saved: Option<SavedTask>,
        branch: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TaskForm {
        let title = title.to_owned();
        let loading = saved
            .as_ref()
            .is_some_and(|saved| saved.description.is_none());
        TaskForm {
            home,
            device,
            repository,
            title: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(title)
                    .placeholder(t!("tasks.form.title_placeholder"))
            }),
            description: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(4, 12)
                    .placeholder(if loading {
                        t!("tasks.form.loading")
                    } else {
                        t!("tasks.form.description_placeholder")
                    })
            }),
            saved,
            repositories: HashMap::new(),
            description_request: None,
            description_error: None,
            saving: None,
            after_save: None,
            error: None,
            start: StartChoice {
                place: None,
                workspace: None,
                branch: cx.new(|cx| InputState::new(window, cx).default_value(branch)),
                agent: None,
                branch_error: None,
                folder_missing: false,
                request: None,
            },
        }
    }

    /// Opens a To do Task's form, fetching its description, which the cards never carry.
    pub(in crate::app) fn open_task_form(
        &mut self,
        key: ConnectionKey,
        task_id: TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = self
            .connection(key)
            .and_then(|connection| connection.model.tasks.as_ref()?.get(&task_id).cloned())
        else {
            return;
        };
        self.clear_task_selection(window, cx);
        let branch = default_branch(&task);
        let form = self.make_task_form(
            Some((key, task_id)),
            key,
            task.repository.clone(),
            &task.title,
            Some(SavedTask {
                title: task.title.clone(),
                description: None,
                repository: task.repository.clone(),
            }),
            branch,
            window,
            cx,
        );
        self.task_form = Some(form);
        self.fetch_form_description(cx);
        if let Some(form) = self
            .task_form
            .as_mut()
            .filter(|form| form.description_request.is_none())
        {
            let label = self
                .connections
                .iter()
                .find(|connection| connection.key == key)
                .map(|connection| connection.label.clone())
                .unwrap_or_default();
            form.description_error = Some(t!("tasks.not_connected", device = label).into_owned());
        }
        self.ask_task_agents(key);
        cx.notify();
    }

    /// Ends the Agent selection, so the area can show a form.
    fn clear_task_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.main_view_mut();
        view.tasks_selection = None;
        view.tasks_dock = None;
        self.terminal_input.target = None;
        self.main_view().tasks_focus.clone().focus(window, cx);
        self.sync_terminal_focus(window, cx);
    }

    /// A click on a To do card: its own form closes, another's opens.
    pub(in crate::app) fn click_todo_card(
        &mut self,
        key: ConnectionKey,
        task_id: TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open = self
            .task_form
            .as_ref()
            .is_some_and(|form| form.home == Some((key, task_id)));
        self.leave_task_form(
            Box::new(move |this, window, cx| {
                if !open {
                    this.open_task_form(key, task_id, window, cx);
                }
            }),
            window,
            cx,
        );
    }

    /// Whether the form's save is still on its way.
    fn save_in_flight(&self) -> bool {
        self.task_form.as_ref().is_some_and(|form| {
            form.saving.is_some_and(|(key, request)| {
                self.connection(key)
                    .is_some_and(|connection| connection.device_tasks.asked(Some(request)))
            })
        })
    }

    /// The leave dialog's Save: the form goes once its save is in, and stays with the
    /// refusal when the save is refused or cannot be sent.
    fn save_then_leave(&mut self, then: Continuation, cx: &mut Context<Self>) {
        if !self.save_in_flight() && !self.save_task_form(cx) {
            return;
        }
        if let Some(form) = &mut self.task_form {
            form.after_save = Some(then);
        }
    }

    /// Save: creates a new Task, edits a saved one, or moves a
    /// To do Task to the chosen Device by creating it there and deleting it here. Says
    /// whether the save went out; what stopped it shows in the form.
    pub(in crate::app) fn save_task_form(&mut self, cx: &mut Context<Self>) -> bool {
        if self.save_in_flight() {
            return false;
        }
        let Some(form) = &mut self.task_form else {
            return false;
        };
        if !form.can_save(cx) {
            return false;
        }
        let title = form.title_text(cx);
        let description = form.description_text(cx);
        form.error = form_refusal(&title, &description);
        if form.error.is_some() {
            cx.notify();
            return false;
        }
        let (device, repository, home) = (form.device, form.repository.clone(), form.home);
        let edit = match (home, &form.saved) {
            (Some((home_key, task)), Some(saved)) if home_key == device => {
                Some(TaskCommand::Edit {
                    task,
                    title: (title != saved.title).then(|| title.clone()),
                    description: (saved.description.as_ref() != Some(&description))
                        .then(|| description.clone()),
                    repository: (repository != saved.repository).then(|| repository.clone()),
                })
            }
            _ => None,
        };
        let (command, request) = match edit {
            Some(edit) => (
                edit,
                TaskRequest::Edit {
                    title: title.clone(),
                },
            ),
            None => (
                TaskCommand::Create {
                    title: title.clone(),
                    description,
                    repository,
                },
                TaskRequest::Create {
                    moved_from: home,
                    title,
                },
            ),
        };
        let Some(connection) = self.connection_mut(device) else {
            return false;
        };
        let label = connection.label.clone();
        let sent = connection.send_task(command, request);
        if let Some(form) = &mut self.task_form {
            match sent {
                Some(request) => form.saving = Some((device, request)),
                None => {
                    form.error = Some((
                        FormField::Form,
                        t!("tasks.not_connected", device = label).into_owned(),
                    ))
                }
            }
        }
        cx.notify();
        sent.is_some()
    }

    /// The form's save is in: where the leave dialog's Save was going, it goes now,
    /// asking again only about changes made since.
    pub(super) fn leave_after_save(&mut self, cx: &mut Context<Self>) {
        let Some(then) = self
            .task_form
            .as_mut()
            .and_then(|form| form.after_save.take())
        else {
            return;
        };
        self.in_main_window(cx, move |this, window, cx| {
            this.leave_task_form(then, window, cx)
        });
    }

    /// Fetches the description of the form's Task again, unless a fetch is on its way.
    pub(in crate::app) fn fetch_form_description(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &self.task_form else {
            return;
        };
        let (Some((key, task_id)), asked) = (form.home, form.description_request) else {
            return;
        };
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if connection.device_tasks.asked(asked) {
            return;
        }
        let request = connection.send_task(
            TaskCommand::Get { task: task_id },
            TaskRequest::Get(task_id),
        );
        if let Some(form) = &mut self.task_form {
            form.description_request = request;
            if request.is_some() {
                form.description_error = None;
            }
        }
        cx.notify();
    }

    /// The Server's copy of the form's Task, with its description: fields the person has
    /// not touched follow it, so a save never takes back another Client's change.
    pub(super) fn resync_task_form(
        &mut self,
        key: ConnectionKey,
        task: TaskRecord,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self
            .task_form
            .as_mut()
            .filter(|form| form.home == Some((key, task.id)))
        else {
            return;
        };
        let old = form.saved.take();
        let (title_now, description_now) = (form.title_text(cx), form.description_text(cx));
        let title_untouched = old.as_ref().is_none_or(|old| title_now == old.title);
        let loading = old.as_ref().is_some_and(|old| old.description.is_none());
        let description_untouched = old
            .as_ref()
            .and_then(|old| old.description.as_ref())
            .is_none_or(|old| *old == description_now);
        if form.device == key
            && old
                .as_ref()
                .is_none_or(|old| form.repository == old.repository)
        {
            form.repository = task.repository.clone();
        }
        let description = task
            .description
            .clone()
            .or_else(|| old.and_then(|old| old.description));
        form.saved = Some(SavedTask {
            title: task.title.clone(),
            description: description.clone(),
            repository: task.repository.clone(),
        });
        let title = (title_untouched && title_now != task.title).then(|| task.title.clone());
        let description = description
            .filter(|description| description_untouched && *description != description_now);
        let (title_input, description_input) = (form.title.clone(), form.description.clone());
        if title.is_none() && description.is_none() && !loading {
            return;
        }
        self.in_main_window(cx, move |_, window, cx| {
            if let Some(title) = title {
                title_input.update(cx, |input, cx| input.set_value(title, window, cx));
            }
            description_input.update(cx, |input, cx| {
                if loading {
                    input.set_placeholder(t!("tasks.form.description_placeholder"), window, cx);
                }
                if let Some(description) = description {
                    input.set_value(description, window, cx);
                }
            });
        });
    }

    pub(in crate::app) fn choose_task_device(
        &mut self,
        device: ConnectionKey,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = &mut self.task_form
            && form.device != device
        {
            // Each Device's repository is its own; coming back gives the choice back.
            let left = form.repository.take();
            form.repositories.insert(form.device, left);
            form.repository = form.repositories.remove(&device).flatten();
            form.device = device;
            cx.notify();
        }
        self.ask_task_agents(device);
    }

    pub(in crate::app) fn choose_task_repository(
        &mut self,
        repository: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = &mut self.task_form {
            form.repository = repository;
            cx.notify();
        }
    }
}

pub(super) fn saved_task(task: &TaskRecord) -> SavedTask {
    SavedTask {
        title: task.title.clone(),
        description: task.description.clone(),
        repository: task.repository.clone(),
    }
}
