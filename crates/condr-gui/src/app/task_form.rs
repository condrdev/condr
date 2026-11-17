//! Tasks in Tasks mode (ADR 0068): the form that writes a Task and starts it, the
//! archive dialog, and the requests behind them. The cards are `tasks.rs`'s.

use super::sidebar::CondrIconName;
use super::*;
use condr_core::agent_discovery::AgentInstallation;
use condr_core::{StartPlace, TaskCommand, TaskError, TaskId, TaskResponse, task_error};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::radio::Radio;
use rust_i18n::t;
use std::borrow::Cow;
use std::collections::VecDeque;
use std::path::Path;

/// `gpui_kit::Task` is the executor's.
type TaskRecord = condr_core::Task;

/// How long a started Task's card reads "Starting…" before, with still no Agent in its
/// Workspace, it reads "No agent running" (ADR 0068).
pub(super) const AGENT_WAIT: Duration = Duration::from_secs(30);

/// An Agent request still owed an `AgentResult`, which carries no request id; the Server
/// answers these in the order asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AgentRequest {
    Hooks,
    Available,
    /// Install from the Start half, whose failure is a toast there (ADR 0020).
    InstallHooks,
}

/// What a Task request was for, so its answer lands where it belongs.
pub(super) enum TaskRequest {
    /// The description an open form fetches.
    Get(TaskId),
    ListArchived,
    /// The form's save of a new Task; `moved_from` is the copy a Device change leaves
    /// behind, deleted once this one exists.
    Create {
        moved_from: Option<(ConnectionKey, TaskId)>,
        title: String,
    },
    Edit {
        title: String,
    },
    /// The old copy of a Task the form moved to another Device.
    DeleteMoved {
        title: String,
        device: String,
    },
    Start(TaskId),
    /// Delete, archive, restore and reorder, whose results arrive as events.
    Change,
}

/// A Device's Task requests in flight and what their answers brought beyond the
/// Session model.
#[derive(Default)]
pub(super) struct DeviceTasks {
    pub(super) pending: HashMap<u64, TaskRequest>,
    pub(super) agent_requests: VecDeque<AgentRequest>,
    /// The Agents on the Device's PATH that can start a Task, in `AgentKind::ALL`'s
    /// order; `None` until asked.
    pub(super) available: Option<Vec<AgentKind>>,
    /// The archived Tasks, fetched each time To do's Archived list unfolds.
    pub(super) archived: Option<Vec<TaskRecord>>,
    /// Started Tasks whose Agent may still be coming: when each was seen to start, and
    /// whether this Client started it.
    pub(super) waiting: HashMap<TaskId, (Instant, bool)>,
}

impl DeviceTasks {
    /// The stream is gone, and every answer it owed with it.
    pub(super) fn connection_closed(&mut self) {
        self.pending.clear();
        self.agent_requests.clear();
        self.available = None;
    }

    pub(super) fn agents_available(&mut self, installations: Vec<AgentInstallation>) {
        self.available = Some(
            AgentKind::ALL
                .into_iter()
                .filter(|kind| {
                    starts_tasks(*kind)
                        && installations
                            .iter()
                            .any(|installation| installation.kind == *kind)
                })
                .collect(),
        );
    }

    fn asked(&self, request: Option<u64>) -> bool {
        request.is_some_and(|request| self.pending.contains_key(&request))
    }
}

/// A fully supported Agent (ADR 0035) whose CLI takes the prompt it starts on.
pub(super) fn starts_tasks(kind: AgentKind) -> bool {
    kind.spec().hooks().is_some() && kind.spec().initial_prompt.args("").is_some()
}

/// Where a Start runs, as the form offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Where {
    Continue,
    Existing,
    NewWorktree,
}

/// Why Continue cannot be chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ContinueBlock {
    /// Another Task holds the Workspace open on the folder; its title.
    Held(String),
    /// The Server said the folder is gone.
    Missing,
}

/// Why New worktree cannot be chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum WorktreeBlock {
    NoRepository,
    /// The repository's Workspace is not open; the repository's name.
    NotOpen(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WorkspaceOption {
    pub(super) id: WorkspaceId,
    pub(super) name: String,
    /// The title of the other Task that holds it.
    pub(super) holder: Option<String>,
}

/// What Where offers a Task on its Device, and which it chooses by default (ADR 0068).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WhereOptions {
    /// The last place's branch, or its folder's name, and why it is blocked.
    pub(super) continue_in: Option<(String, Option<ContinueBlock>)>,
    pub(super) workspaces: Vec<WorkspaceOption>,
    /// The Workspace Existing takes unless the person picks another.
    pub(super) existing: Option<WorkspaceId>,
    /// The repository Workspace's name.
    pub(super) new_worktree: Result<String, WorktreeBlock>,
    pub(super) default: Option<Where>,
}

impl WhereOptions {
    pub(super) fn allows(&self, place: Where) -> bool {
        match place {
            Where::Continue => matches!(self.continue_in, Some((_, None))),
            Where::Existing => self.workspaces.iter().any(|each| each.holder.is_none()),
            Where::NewWorktree => self.new_worktree.is_ok(),
        }
    }
}

/// The defaults' order: Continue where it was worked; else the repository's Workspace
/// when no Task holds it; else a new worktree of it; else the first free Workspace.
/// A folder the Server reported gone makes a new worktree the default.
pub(super) fn where_options(
    session: &Session,
    tasks: &BTreeMap<TaskId, TaskRecord>,
    task: &TaskRecord,
    folder_missing: bool,
) -> WhereOptions {
    let holder = |workspace_id: WorkspaceId| {
        tasks
            .values()
            .find(|other| other.id != task.id && other.workspace == Some(workspace_id))
            .map(|other| other.title.clone())
    };
    let continue_in = task.last.as_ref().map(|last| {
        let name = last
            .branch
            .clone()
            .unwrap_or_else(|| folder_name(&last.path));
        let block = if folder_missing {
            Some(ContinueBlock::Missing)
        } else {
            session
                .workspace_by_root(&last.path)
                .and_then(|workspace| holder(workspace.id()))
                .map(ContinueBlock::Held)
        };
        (name, block)
    });
    let workspaces: Vec<WorkspaceOption> = session
        .workspaces()
        .iter()
        .map(|workspace| WorkspaceOption {
            id: workspace.id(),
            name: workspace.name().to_owned(),
            holder: holder(workspace.id()),
        })
        .collect();
    let repository = task
        .repository
        .as_deref()
        .and_then(|root| session.workspace_by_root(root));
    let repository_free = repository.filter(|workspace| holder(workspace.id()).is_none());
    let first_free = workspaces
        .iter()
        .find(|each| each.holder.is_none())
        .map(|each| each.id);
    let new_worktree = match (&task.repository, repository) {
        (None, _) => Err(WorktreeBlock::NoRepository),
        (Some(root), None) => Err(WorktreeBlock::NotOpen(folder_name(root))),
        (Some(_), Some(workspace)) => Ok(workspace.name().to_owned()),
    };
    let default = match &continue_in {
        Some((_, None)) => Some(Where::Continue),
        Some((_, Some(ContinueBlock::Missing))) if new_worktree.is_ok() => Some(Where::NewWorktree),
        _ if repository_free.is_some() => Some(Where::Existing),
        _ if repository.is_some() => Some(Where::NewWorktree),
        _ => first_free.map(|_| Where::Existing),
    };
    WhereOptions {
        continue_in,
        workspaces,
        existing: repository_free.map(Workspace::id).or(first_free),
        new_worktree,
        default,
    }
}

fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// What a saved Task held when the form last heard of it.
pub(super) struct SavedTask {
    pub(super) title: String,
    /// `None` until the form's request for it is answered.
    pub(super) description: Option<String>,
    pub(super) repository: Option<PathBuf>,
}

/// The Start half's choices; `None` follows the default.
pub(super) struct StartChoice {
    pub(super) place: Option<Where>,
    pub(super) workspace: Option<WorkspaceId>,
    pub(super) branch: Entity<InputState>,
    pub(super) agent: Option<AgentKind>,
    /// The Server's refusal of the branch, shown under its field.
    pub(super) branch_error: Option<String>,
    pub(super) folder_missing: bool,
    pub(super) request: Option<u64>,
}

/// Where a refusal of the form shows (ADR 0020).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FormField {
    Title,
    Description,
    /// Under the form's buttons, for what no one field caused.
    Form,
}

/// The form in the interaction area's left half, for a To do Task or a new one; nothing
/// is kept until Save (ADR 0068).
pub(super) struct TaskForm {
    /// The saved Task's Device and number; `None` for a new Task, which has no card yet.
    pub(super) home: Option<(ConnectionKey, TaskId)>,
    /// The Device chosen in the form.
    pub(super) device: ConnectionKey,
    pub(super) repository: Option<PathBuf>,
    /// The repository chosen on each Device the form left, given back on returning.
    pub(super) repositories: HashMap<ConnectionKey, Option<PathBuf>>,
    pub(super) title: Entity<InputState>,
    pub(super) description: Entity<TextareaState>,
    pub(super) saved: Option<SavedTask>,
    pub(super) description_request: Option<u64>,
    /// Why the description could not be fetched; Retry asks again.
    pub(super) description_error: Option<String>,
    /// The save in flight, by Device and request.
    pub(super) saving: Option<(ConnectionKey, u64)>,
    /// Where the leave dialog's Save was going, once that save is in.
    pub(super) after_save: Option<Continuation>,
    /// The last save's refusal, under the field it is about.
    pub(super) error: Option<(FormField, String)>,
    pub(super) start: StartChoice,
}

impl TaskForm {
    fn title_text(&self, cx: &App) -> String {
        self.title.read(cx).value().trim().to_owned()
    }

    fn description_text(&self, cx: &App) -> String {
        self.description.read(cx).value().to_string()
    }

    /// Whether leaving loses something: a new Task's text, or a saved one's changes.
    pub(super) fn changed(&self, cx: &App) -> bool {
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
    fn can_save(&self, cx: &App) -> bool {
        !self.title_text(cx).is_empty() && self.changed(cx) && self.description_loaded()
    }

    /// A new Task's description is there from the start; a saved one's once fetched.
    pub(super) fn description_loaded(&self) -> bool {
        self.saved
            .as_ref()
            .is_none_or(|saved| saved.description.is_some())
    }

    fn error_for(&self, field: FormField) -> Option<String> {
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
    /// The connected Devices that keep Tasks: a Server that sent no Tasks record, older
    /// or built without them, offers no To do and takes no new Task (ADR 0068).
    pub(super) fn task_devices(&self) -> impl Iterator<Item = &ServerConnection> {
        self.connections.iter().filter(|connection| {
            connection.status == ConnectionStatus::Connected && connection.model.tasks.is_some()
        })
    }

    /// How many Tasks wait in To do across the connected Devices, for the sidebar's row.
    pub(super) fn todo_count(&self) -> usize {
        self.task_devices()
            .flat_map(|connection| {
                connection
                    .model
                    .tasks
                    .iter()
                    .flat_map(|tasks| tasks.values())
            })
            .filter(|task| task.workspace.is_none())
            .count()
    }

    /// Runs `then` once the open form may go: at once when nothing would be lost, else
    /// after asking Save, Discard or Cancel (ADR 0068).
    pub(super) fn leave_task_form(
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
    pub(super) fn new_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        if !self.main_view().tasks.on {
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

    /// Asks the Device which Agents can start, and how their hooks stand, unless known.
    fn ask_task_agents(&mut self, key: ConnectionKey) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let asked = |request| connection.device_tasks.agent_requests.contains(&request);
        let ask_hooks = connection.hooks.is_empty()
            && !asked(AgentRequest::Hooks)
            && !asked(AgentRequest::InstallHooks);
        if connection.device_tasks.available.is_none() && !asked(AgentRequest::Available) {
            connection.send_agents_available();
        }
        if ask_hooks {
            self.request_agent_hooks(key);
        }
    }

    /// Opens a To do Task's form, fetching its description, which the cards never carry.
    pub(super) fn open_task_form(
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
            form.description_error =
                Some(t!("tasks.band.not_connected", device = label).into_owned());
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
    pub(super) fn click_todo_card(
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
    pub(super) fn save_task_form(&mut self, cx: &mut Context<Self>) -> bool {
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
                        t!("tasks.band.not_connected", device = label).into_owned(),
                    ))
                }
            }
        }
        cx.notify();
        sent.is_some()
    }

    /// The form's save is in: where the leave dialog's Save was going, it goes now,
    /// asking again only about changes made since.
    fn leave_after_save(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn fetch_form_description(&mut self, cx: &mut Context<Self>) {
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
    fn resync_task_form(&mut self, key: ConnectionKey, task: TaskRecord, cx: &mut Context<Self>) {
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

    /// A Task request's answer.
    pub(super) fn task_answered(
        &mut self,
        key: ConnectionKey,
        request_id: u64,
        result: Result<TaskResponse, TaskError>,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let Some(request) = connection.device_tasks.pending.remove(&request_id) else {
            return;
        };
        match (request, result) {
            (TaskRequest::Get(task_id), Ok(TaskResponse::Task(task))) => {
                if let Some(form) = self
                    .task_form
                    .as_mut()
                    .filter(|form| form.home == Some((key, task_id)))
                {
                    form.description_error = None;
                }
                self.resync_task_form(key, task, cx);
            }
            (TaskRequest::Get(task_id), Err(error)) => {
                if let Some(form) = self
                    .task_form
                    .as_mut()
                    .filter(|form| form.home == Some((key, task_id)))
                {
                    form.description_error = Some(error.message);
                }
            }
            (TaskRequest::ListArchived, Ok(TaskResponse::Tasks(tasks))) => {
                connection.device_tasks.archived = Some(tasks);
            }
            (TaskRequest::Create { moved_from, .. }, Ok(TaskResponse::Task(task))) => {
                if let Some((old_key, old_task)) = moved_from {
                    let device = connection.label.clone();
                    let title = task.title.clone();
                    let request = TaskRequest::DeleteMoved {
                        title: title.clone(),
                        device: device.clone(),
                    };
                    let sent = self.connection_mut(old_key).and_then(|old| {
                        old.send_task(TaskCommand::Delete { task: old_task }, request)
                    });
                    // Both copies stay; the person is told (ADR 0068).
                    if sent.is_none() {
                        let old = self
                            .connection(old_key)
                            .map(|old| old.label.clone())
                            .unwrap_or_default();
                        let error = t!("tasks.band.not_connected", device = old);
                        self.report_error(
                            t!(
                                "tasks.form.move_failed",
                                title = title,
                                device = device,
                                old = old,
                                error = error
                            ),
                            cx,
                        );
                    }
                }
                let ours = self
                    .task_form
                    .as_ref()
                    .is_some_and(|form| form.saving == Some((key, request_id)));
                if ours && let Some(form) = &mut self.task_form {
                    form.saving = None;
                    form.error = None;
                    form.home = Some((key, task.id));
                    form.device = key;
                    form.saved = Some(saved_task(&task));
                    form.start.place = None;
                    form.start.workspace = None;
                    let (branch, input) = (default_branch(&task), form.start.branch.clone());
                    self.in_main_window(cx, move |_, window, cx| {
                        input.update(cx, |input, cx| input.set_value(branch, window, cx));
                    });
                    self.ask_task_agents(key);
                    self.leave_after_save(cx);
                }
            }
            (TaskRequest::Edit { .. }, Ok(TaskResponse::Task(task))) => {
                let ours = self.task_form.as_mut().and_then(|form| {
                    (form.saving == Some((key, request_id))).then(|| {
                        form.saving = None;
                        form.error = None;
                    })
                });
                self.resync_task_form(key, task, cx);
                if ours.is_some() {
                    self.leave_after_save(cx);
                }
            }
            (TaskRequest::Start(task_id), result) => {
                self.task_start_answered(key, task_id, result, cx)
            }
            (TaskRequest::DeleteMoved { title, device }, Err(error)) => {
                let old = connection.label.clone();
                self.report_error(
                    t!(
                        "tasks.form.move_failed",
                        title = title,
                        device = device,
                        old = old,
                        error = error.message
                    ),
                    cx,
                );
            }
            // A refusal stays in the form it is about, which stays open; with the form
            // gone, a toast names the Task.
            (TaskRequest::Create { title, .. } | TaskRequest::Edit { title }, Err(error)) => {
                match self
                    .task_form
                    .as_mut()
                    .filter(|form| form.saving == Some((key, request_id)))
                {
                    Some(form) => {
                        form.saving = None;
                        form.after_save = None;
                        form.error = Some((FormField::Form, error.message));
                    }
                    None => self.report_error(
                        t!(
                            "tasks.form.save_failed",
                            title = title,
                            error = error.message
                        ),
                        cx,
                    ),
                }
            }
            (_, Err(error)) => {
                self.report_error(t!("tasks.request_failed", error = error.message), cx)
            }
            (_, Ok(_)) => {}
        }
        cx.notify();
    }

    /// Runs `update` in the main window once the current update is done: an answer that
    /// sets an input's text needs its `Window`, which an incoming message has not.
    fn in_main_window(
        &self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let handle = self.main_window();
        let owner = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ = owner.update(cx, |this, cx| update(this, window, cx));
            });
        });
    }

    /// A Task changed on `key`'s Device: the model follows, as do the Archived list, a
    /// Start's wait for its Agent and an open form (ADR 0068).
    pub(super) fn task_changed(
        &mut self,
        key: ConnectionKey,
        task: TaskRecord,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let was_started = connection
            .model
            .tasks
            .as_ref()
            .and_then(|tasks| tasks.get(&task.id))
            .is_some_and(|before| before.workspace.is_some());
        if let Some(archived) = &mut connection.device_tasks.archived {
            archived.retain(|each| each.id != task.id);
            if task.archived {
                archived.insert(0, task.clone());
            }
        }
        let (id, started) = (task.id, task.workspace.is_some() && !task.archived);
        if !started {
            // Back in To do: a later Start waits afresh.
            connection.device_tasks.waiting.remove(&id);
        }
        connection.model.apply_task(task);
        if started && !was_started {
            self.watch_task_start(key, id, false, cx);
        }
        self.reconcile_task_form(cx);
        // Another Client may have changed what the form shows, and an event carries no
        // description: the form fetches the Task again.
        if self
            .task_form
            .as_ref()
            .is_some_and(|form| form.home == Some((key, id)) && form.description_loaded())
        {
            self.fetch_form_description(cx);
        }
    }

    pub(super) fn task_removed(
        &mut self,
        key: ConnectionKey,
        task: TaskId,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if let Some(archived) = &mut connection.device_tasks.archived {
            archived.retain(|each| each.id != task);
        }
        connection.model.remove_task(task);
        self.reconcile_task_form(cx);
    }

    /// A Bootstrap replaced the Device's Tasks: the form follows, a description lost
    /// with the old stream is fetched again, and an unfolded Archived list too.
    pub(super) fn tasks_bootstrapped(&mut self, key: ConnectionKey, cx: &mut Context<Self>) {
        self.reconcile_task_form(cx);
        // Answers lost with the old stream, or to writer lag, never come.
        if let (Some(form), Some(connection)) = (
            &mut self.task_form,
            self.connections
                .iter()
                .find(|connection| connection.key == key),
        ) {
            let lost = |request: u64| !connection.device_tasks.asked(Some(request));
            if form
                .saving
                .is_some_and(|(at, request)| at == key && lost(request))
            {
                form.saving = None;
                form.after_save = None;
            }
            if form.home.is_some_and(|(at, _)| at == key) && form.start.request.is_some_and(lost) {
                form.start.request = None;
            }
        }
        if self
            .task_form
            .as_ref()
            .is_some_and(|form| form.device == key)
        {
            self.ask_task_agents(key);
        }
        let archived_open = self.tasks_archived_open;
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if archived_open && connection.model.tasks.is_some() {
            connection.send_task(
                TaskCommand::List { archived: true },
                TaskRequest::ListArchived,
            );
        }
        let Some(form) = &self.task_form else {
            return;
        };
        let refetch = form.home.filter(|(home, _)| *home == key).filter(|_| {
            form.saved
                .as_ref()
                .is_some_and(|saved| saved.description.is_none())
        });
        let asked = form.description_request;
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if let Some((_, task_id)) = refetch
            && !connection.device_tasks.asked(asked)
        {
            let request = connection.send_task(
                TaskCommand::Get { task: task_id },
                TaskRequest::Get(task_id),
            );
            if let Some(form) = &mut self.task_form {
                form.description_request = request;
            }
        }
    }

    /// A form whose Task another Client deleted or archived closes; one whose Task
    /// started follows it to its card (ADR 0068).
    fn reconcile_task_form(&mut self, cx: &mut Context<Self>) {
        let Some((key, task_id)) = self.task_form.as_ref().and_then(|form| form.home) else {
            return;
        };
        let Some(tasks) = self
            .connection(key)
            .and_then(|connection| connection.model.tasks.as_ref())
        else {
            return;
        };
        match tasks.get(&task_id) {
            None => {
                self.task_form = None;
                cx.notify();
            }
            // A form with changes stays for the person to save or leave; it follows the
            // Task once it has none.
            Some(task)
                if task.workspace.is_some()
                    && !self.task_form.as_ref().is_some_and(|form| form.changed(cx)) =>
            {
                self.task_form = None;
                self.in_main_window(cx, move |this, window, cx| {
                    if this.task_form.is_none() {
                        this.select_started_task(key, task_id, false, window, cx);
                    }
                });
                cx.notify();
            }
            Some(_) => {}
        }
    }

    /// Selects a started Task's card: its Agent's, or while it has none, the Task's own
    /// on its Workspace's focused Pane.
    pub(super) fn select_started_task(
        &mut self,
        key: ConnectionKey,
        task_id: TaskId,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection(key) else {
            return;
        };
        let Some(workspace_id) = connection
            .model
            .tasks
            .as_ref()
            .and_then(|tasks| tasks.get(&task_id))
            .and_then(|task| task.workspace)
        else {
            return;
        };
        let Some(session) = connection.session() else {
            return;
        };
        let Some(workspace) = session.workspace(workspace_id) else {
            return;
        };
        let agent = workspace
            .tabs()
            .iter()
            .flat_map(|tab| tab.panes())
            .map(|pane| pane.id())
            .find(|pane_id| connection.model.agents.contains_key(pane_id));
        match agent {
            Some(pane_id) => self.select_card(key, pane_id, None, keyboard, window, cx),
            None => {
                let Some(pane_id) = workspace_focused_pane(connection, session, workspace_id)
                else {
                    return;
                };
                self.select_card(key, pane_id, Some(task_id), keyboard, window, cx);
            }
        }
    }

    /// An Agent left its Pane: when its Task has no Agent left, the selection on that
    /// Pane moves to the Task's card (ADR 0068).
    pub(super) fn agent_left_task(&mut self, key: ConnectionKey, pane_id: PaneId) {
        let Some(selected) = self
            .main_view()
            .tasks_selection
            .filter(|selected| (selected.key, selected.pane_id) == (key, pane_id))
        else {
            return;
        };
        let Some(connection) = self.connection(key) else {
            return;
        };
        let Some(workspace) = connection
            .session()
            .and_then(|session| session.workspace_for_pane(pane_id))
        else {
            return;
        };
        let Some(task) = connection.model.workspace_task(workspace.id()) else {
            return;
        };
        let agents_left = workspace
            .tabs()
            .iter()
            .flat_map(|tab| tab.panes())
            .any(|pane| connection.model.agents.contains_key(&pane.id()));
        if !agents_left {
            let task = task.id;
            if let Some(selection) = self.main_view_mut().tasks_selection.as_mut() {
                *selection = super::tasks::TasksSelection {
                    task: Some(task),
                    column: None,
                    ..selected
                };
            }
        }
    }

    /// Notes that a Task started, so its card reads "Starting…" until its Agent comes
    /// or `AGENT_WAIT` passes; for this Client's own Start, a toast says when none came.
    fn watch_task_start(
        &mut self,
        key: ConnectionKey,
        task: TaskId,
        own: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if let Some((_, mine)) = connection.device_tasks.waiting.get_mut(&task) {
            *mine |= own;
            return;
        }
        let started = Instant::now();
        connection.device_tasks.waiting.insert(task, (started, own));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AGENT_WAIT).await;
            let _ = this.update(cx, |this, cx| {
                // Only this Start's wait: the Task may have gone back and started again.
                let current = this
                    .connection(key)
                    .and_then(|connection| connection.device_tasks.waiting.get(&task))
                    .map(|(at, _)| *at);
                if current == Some(started) {
                    this.task_start_settled(key, task, cx);
                }
            });
        })
        .detach();
    }

    /// `AGENT_WAIT` after a Start: the card reads "No agent running" if no Agent came,
    /// and for this Client's own Start one toast says so.
    pub(super) fn task_start_settled(
        &mut self,
        key: ConnectionKey,
        task_id: TaskId,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let Some((_, own)) = connection.device_tasks.waiting.remove(&task_id) else {
            return;
        };
        let no_agent = connection
            .model
            .tasks
            .as_ref()
            .and_then(|tasks| tasks.get(&task_id))
            .filter(|task| {
                task.workspace
                    .is_some_and(|workspace| !workspace_has_agent(connection, workspace))
            })
            .map(|task| task.title.clone());
        if own && let Some(title) = no_agent {
            self.report_error(t!("tasks.start.no_agent_started", title = title), cx);
        }
        cx.notify();
    }

    /// Whether a started Task's card still reads "Starting…".
    pub(super) fn task_starting(&self, key: ConnectionKey, task: TaskId) -> bool {
        self.connection(key).is_some_and(|connection| {
            connection
                .device_tasks
                .waiting
                .get(&task)
                .is_some_and(|(at, _)| at.elapsed() < AGENT_WAIT)
        })
    }

    /// Start: sends the Task, the place chosen and the Agent (ADR 0068).
    fn start_task(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &self.task_form else {
            return;
        };
        let Some((key, task_id)) = form.home else {
            return;
        };
        let Some(plan) = self.start_plan(form, cx) else {
            return;
        };
        let StartPlan {
            place,
            workspace,
            agent: Some(agent),
            blocked: None,
            ..
        } = plan
        else {
            return;
        };
        let place = match place {
            Where::Continue => StartPlace::Continue,
            Where::Existing => match workspace {
                Some(workspace) => StartPlace::Workspace(workspace),
                None => return,
            },
            Where::NewWorktree => StartPlace::NewWorktree {
                branch: Some(form.start.branch.read(cx).value().trim().to_owned())
                    .filter(|branch| !branch.is_empty()),
            },
        };
        self.last_task_agent = Some(agent);
        let request = self.connection_mut(key).and_then(|connection| {
            connection.send_task(
                TaskCommand::Start {
                    task: task_id,
                    place: Some(place),
                    agent,
                },
                TaskRequest::Start(task_id),
            )
        });
        if let Some(form) = &mut self.task_form {
            form.start.request = request;
            form.start.branch_error = None;
        }
        cx.notify();
    }

    fn task_start_answered(
        &mut self,
        key: ConnectionKey,
        task_id: TaskId,
        result: Result<TaskResponse, TaskError>,
        cx: &mut Context<Self>,
    ) {
        if let Ok(TaskResponse::Started { pane_id, .. }) = result {
            self.watch_task_start(key, task_id, true, cx);
            // The started Task's card takes the area only from a form that loses nothing
            // by it: none, or this Task's own without changes.
            let follow = self
                .task_form
                .as_ref()
                .is_none_or(|form| form.home == Some((key, task_id)) && !form.changed(cx));
            if follow {
                self.task_form = None;
                self.in_main_window(cx, move |this, window, cx| {
                    if this.task_form.is_none() {
                        this.select_card(key, pane_id, Some(task_id), true, window, cx);
                    }
                });
            }
            return;
        }
        let form = self
            .task_form
            .as_mut()
            .filter(|form| form.home == Some((key, task_id)));
        match result {
            Ok(_) => {}
            Err(error) if error.code == task_error::BRANCH_CHECKED_OUT && form.is_some() => {
                let form = form.expect("checked above");
                form.start.request = None;
                form.start.branch_error = Some(error.message);
            }
            Err(error) if error.code == task_error::FOLDER_MISSING && form.is_some() => {
                let form = form.expect("checked above");
                form.start.request = None;
                form.start.folder_missing = true;
                form.start.place = None;
                let branch = self
                    .connection(key)
                    .and_then(|connection| connection.model.tasks.as_ref()?.get(&task_id))
                    .and_then(|task| task.last.as_ref()?.branch.clone());
                if let Some(branch) = branch {
                    let input = self
                        .task_form
                        .as_ref()
                        .map(|form| form.start.branch.clone());
                    self.in_main_window(cx, move |_, window, cx| {
                        if let Some(input) = input {
                            input.update(cx, |input, cx| input.set_value(branch, window, cx));
                        }
                    });
                }
            }
            Err(error) => {
                if let Some(form) = form {
                    form.start.request = None;
                }
                let title = self
                    .connection(key)
                    .and_then(|connection| connection.model.tasks.as_ref()?.get(&task_id))
                    .map(|task| task.title.clone())
                    .unwrap_or_default();
                self.report_error(
                    t!("tasks.start.failed", title = title, error = error.message),
                    cx,
                );
            }
        }
    }

    /// Archive on a To do Task's form, which asks nothing (ADR 0068).
    fn archive_todo(&mut self, key: ConnectionKey, task: TaskId, cx: &mut Context<Self>) {
        self.task_form = None;
        self.send_task_change(
            key,
            TaskCommand::Archive {
                task,
                close_workspace: false,
                remove_worktree: false,
            },
            cx,
        );
    }

    fn send_task_change(
        &mut self,
        key: ConnectionKey,
        command: TaskCommand,
        cx: &mut Context<Self>,
    ) {
        let sent = self
            .connection_mut(key)
            .and_then(|connection| connection.send_task(command, TaskRequest::Change));
        if sent.is_none()
            && let Some(connection) = self.connection(key)
        {
            let label = connection.label.clone();
            self.report_error(t!("tasks.band.not_connected", device = label), cx);
        }
        cx.notify();
    }

    /// Delete, from To do or Archived, after "Delete ‘…’? This can't be undone."
    pub(super) fn confirm_delete_task(
        &mut self,
        key: ConnectionKey,
        task: TaskId,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        super::dialogs::confirm_danger(
            cx.weak_entity(),
            super::dialogs::DangerConfirm::new(
                t!("tasks.delete.title", title = title),
                t!("tasks.delete.description"),
                t!("tasks.form.delete"),
            ),
            move |this, _, cx| {
                if this
                    .task_form
                    .as_ref()
                    .is_some_and(|form| form.home == Some((key, task)))
                {
                    this.task_form = None;
                }
                this.send_task_change(key, TaskCommand::Delete { task }, cx);
            },
            window,
            cx,
        );
    }

    pub(super) fn restore_task(
        &mut self,
        key: ConnectionKey,
        task: TaskId,
        cx: &mut Context<Self>,
    ) {
        self.send_task_change(key, TaskCommand::Restore { task }, cx);
    }

    /// Drops a To do Task where another of the same Device was: the Server counts the
    /// place among that Device's To do Tasks, the dragged one included.
    pub(super) fn reorder_todo(
        &mut self,
        key: ConnectionKey,
        task: TaskId,
        onto: TaskId,
        cx: &mut Context<Self>,
    ) {
        if task == onto {
            return;
        }
        let Some(index) = self.connection(key).and_then(|connection| {
            todo_tasks(connection)
                .iter()
                .position(|each| each.id == onto)
        }) else {
            return;
        };
        self.send_task_change(
            key,
            TaskCommand::Reorder {
                task,
                index: index as u32,
            },
            cx,
        );
    }

    /// Folds or unfolds To do's Archived list, fetching it from each Device as it opens.
    pub(super) fn toggle_archived_tasks(&mut self, cx: &mut Context<Self>) {
        self.tasks_archived_open = !self.tasks_archived_open;
        if self.tasks_archived_open {
            for connection in &mut self.connections {
                if connection.status == ConnectionStatus::Connected
                    && connection.model.tasks.is_some()
                {
                    connection.send_task(
                        TaskCommand::List { archived: true },
                        TaskRequest::ListArchived,
                    );
                }
            }
        }
        cx.notify();
    }

    /// Asks before archiving a started Task: whether to close its Workspace, checked when
    /// a Start of it made the Workspace, and whether to remove its worktree (ADR 0068).
    pub(super) fn confirm_archive_task(
        &mut self,
        key: ConnectionKey,
        task_id: TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(facts) = self.archive_facts(key, task_id) else {
            return;
        };
        let choice = cx.new(|_| (facts.created, false));
        let error = cx.new(|_| None::<String>);
        let content = {
            let choice = choice.clone();
            let facts = facts.clone();
            move |_: &mut Window, cx: &mut App| render_archive_choice(&facts, &choice, cx)
        };
        let owner = cx.weak_entity();
        let submit = move |_: &mut Window, cx: &mut App| {
            let (close_workspace, remove_worktree) = *choice.read(cx);
            let command = TaskCommand::Archive {
                task: task_id,
                close_workspace,
                remove_worktree: close_workspace && remove_worktree,
            };
            let _ = owner.update(cx, |this, cx| this.send_task_change(key, command, cx));
            Ok::<(), String>(())
        };
        let title = t!("tasks.archive.title", title = facts.title).into_owned();
        window.defer(cx, move |window, cx| {
            super::dialogs::open_form_dialog(
                title.into(),
                t!("tasks.archive.ok").into(),
                error,
                content,
                submit,
                window,
                cx,
            );
        });
    }

    /// What the archive dialog says about a started Task's Workspace.
    pub(super) fn archive_facts(
        &self,
        key: ConnectionKey,
        task_id: TaskId,
    ) -> Option<ArchiveFacts> {
        let connection = self.connection(key)?;
        let task = connection.model.tasks.as_ref()?.get(&task_id)?;
        let workspace = connection.session()?.workspace(task.workspace?)?;
        let agents: Vec<(AgentKind, AgentDisplayState)> = workspace
            .tabs()
            .iter()
            .flat_map(|tab| tab.panes())
            .filter_map(|pane| {
                let agent = connection.model.agents.get(&pane.id())?;
                Some((agent.kind, connection.model.agent_display_state(pane.id())?))
            })
            .collect();
        let busy = agents
            .iter()
            .find(|(_, state)| matches!(state, AgentDisplayState::Blocked))
            .or_else(|| {
                agents
                    .iter()
                    .find(|(_, state)| matches!(state, AgentDisplayState::Working))
            })
            .copied();
        let managed = workspace
            .worktree()
            .is_some_and(|association| association.is_managed());
        let git = connection.model.workspace_git.get(&workspace.id());
        Some(ArchiveFacts {
            title: task.title.clone(),
            created: task.made_workspace,
            agents: agents.len(),
            busy,
            managed,
            dirty: git.is_some_and(|git| !git.changes.entries.is_empty()),
            branch: git.and_then(|git| git.branch.clone()),
        })
    }

    fn choose_task_where(&mut self, place: Where, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.task_form {
            form.start.place = Some(place);
            cx.notify();
        }
    }

    pub(super) fn choose_task_workspace(&mut self, workspace: WorkspaceId, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.task_form {
            form.start.place = Some(Where::Existing);
            form.start.workspace = Some(workspace);
            cx.notify();
        }
    }

    fn choose_task_agent(&mut self, agent: AgentKind, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.task_form {
            form.start.agent = Some(agent);
            cx.notify();
        }
    }

    pub(super) fn choose_task_device(&mut self, device: ConnectionKey, cx: &mut Context<Self>) {
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

    pub(super) fn choose_task_repository(
        &mut self,
        repository: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = &mut self.task_form {
            form.repository = repository;
            cx.notify();
        }
    }

    /// What Start would do now, and why it cannot when it cannot.
    pub(super) fn start_plan(&self, form: &TaskForm, cx: &App) -> Option<StartPlan> {
        let (key, task_id) = form.home?;
        let connection = self.connection(key)?;
        let tasks = connection.model.tasks.as_ref()?;
        let task = tasks.get(&task_id)?;
        let session = connection.session()?;
        let options = where_options(session, tasks, task, form.start.folder_missing);
        let place = form
            .start
            .place
            .filter(|place| options.allows(*place))
            .or(options.default);
        let workspace = form
            .start
            .workspace
            .filter(|workspace| {
                options
                    .workspaces
                    .iter()
                    .any(|each| each.id == *workspace && each.holder.is_none())
            })
            .or(options.existing);
        let available = connection.device_tasks.available.clone();
        let agent = available.as_ref().and_then(|available| {
            form.start
                .agent
                .or(self.last_task_agent)
                .filter(|agent| available.contains(agent))
                .or(available.first().copied())
        });
        let blocked = if form.changed(cx) {
            Some(t!("tasks.start.save_first").into_owned())
        } else if task.starting
            || task.workspace.is_some()
            || connection.device_tasks.asked(form.start.request)
        {
            Some(t!("tasks.todo.starting").into_owned())
        } else if place.is_none() {
            Some(t!("tasks.start.nowhere", device = connection.label).into_owned())
        } else {
            None
        };
        Some(StartPlan {
            options,
            place: place.unwrap_or(Where::Existing),
            workspace,
            available,
            agent,
            blocked,
        })
    }

    /// The interaction area for the form: a header, the form's fields on the left, and
    /// on the right Start for a saved Task (ADR 0068).
    pub(super) fn render_task_form_area(
        &self,
        form: &TaskForm,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let connection = self.connection(form.device);
        let device = connection
            .map(|connection| connection.label.clone())
            .unwrap_or_default();
        let task = form.home.and_then(|(key, task_id)| {
            self.connection(key)?
                .model
                .tasks
                .as_ref()?
                .get(&task_id)
                .cloned()
        });
        let heading = match &form.saved {
            Some(saved) => saved.title.clone(),
            None => t!("tasks.form.new_title").into_owned(),
        };
        let mut place = vec![t!("tasks.column.to_do").into_owned(), device.clone()];
        place.extend(
            form.repository
                .as_deref()
                .map(|root| repository_name(connection, root)),
        );
        let close_owner = cx.weak_entity();
        let header = h_flex()
            .debug_selector(|| "tasks-area-header".into())
            .flex_none()
            .h(rems(2.25))
            .w_full()
            .gap_2()
            .px_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(
                Icon::new(CondrIconName::Circle)
                    .size_3p5()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .font_semibold()
                    .child(heading),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(place.join(" · ")),
            )
            .when(task.as_ref().is_some_and(|task| task.starting), |this| {
                this.child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(t!("tasks.todo.starting").into_owned()),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("tasks-area-close")
                    .debug_selector(|| "tasks-area-close".into())
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::Close))
                    .tooltip(t!("tasks.area.close"))
                    .on_click(move |_, window, cx| {
                        let _ = close_owner.update(cx, |this, cx| this.close_task_area(window, cx));
                    }),
            );
        let fields = self.render_task_fields(form, &theme, cx);
        let start = match task.filter(|_| form.saved.is_some()) {
            Some(task) => self.render_task_start(form, &task, &theme, cx),
            None => div()
                .debug_selector(|| "task-start-unsaved".into())
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(t!("tasks.form.save_to_start").into_owned())
                .into_any_element(),
        };
        v_flex()
            .id("tasks-area")
            .debug_selector(|| "tasks-area".into())
            .size_full()
            .child(header)
            .child(
                h_flex()
                    .id("task-form-body")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .items_start()
                    .gap_5()
                    .p_3()
                    .overflow_y_scroll()
                    .child(fields)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .min_h_full()
                            .gap_2()
                            .pl_5()
                            .border_l_1()
                            .border_color(theme.border)
                            .child(start),
                    ),
            )
            .into_any_element()
    }

    fn render_task_fields(
        &self,
        form: &TaskForm,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let owner = cx.weak_entity();
        let save_owner = cx.weak_entity();
        let connection = self.connection(form.device);
        let devices: Vec<(ConnectionKey, String)> = self
            .task_devices()
            .map(|connection| (connection.key, connection.label.clone()))
            .collect();
        let chosen_device = form.device;
        let device_owner = owner.clone();
        let device = Button::new("task-form-device")
            .debug_selector(|| "task-form-device".into())
            .outline()
            .small()
            .label(
                connection
                    .map(|connection| connection.label.clone())
                    .unwrap_or_default(),
            )
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                devices.iter().fold(menu, |menu, (key, label)| {
                    let (owner, key) = (device_owner.clone(), *key);
                    menu.item(
                        PopupMenuItem::new(label.clone())
                            .checked(key == chosen_device)
                            .on_click(move |_, _, cx| {
                                let _ =
                                    owner.update(cx, |this, cx| this.choose_task_device(key, cx));
                            }),
                    )
                })
            });
        let repositories: Vec<(String, PathBuf)> = connection
            .map(|connection| {
                repository_workspaces(connection)
                    .map(|(_, name, root)| (name, root))
                    .collect()
            })
            .unwrap_or_default();
        let chosen_repository = form.repository.clone();
        let repository_owner = owner.clone();
        let repository = Button::new("task-form-repository")
            .debug_selector(|| "task-form-repository".into())
            .outline()
            .small()
            .label(match &form.repository {
                Some(root) => repository_name(connection, root),
                None => t!("tasks.form.no_repository").into_owned(),
            })
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                let none_owner = repository_owner.clone();
                let menu = menu.item(
                    PopupMenuItem::new(t!("tasks.form.no_repository"))
                        .checked(chosen_repository.is_none())
                        .on_click(move |_, _, cx| {
                            let _ = none_owner
                                .update(cx, |this, cx| this.choose_task_repository(None, cx));
                        }),
                );
                repositories.iter().fold(menu, |menu, (name, root)| {
                    let (owner, root) = (repository_owner.clone(), root.clone());
                    menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(chosen_repository.as_ref() == Some(&root))
                            .on_click(move |_, _, cx| {
                                let root = root.clone();
                                let _ = owner.update(cx, |this, cx| {
                                    this.choose_task_repository(Some(root), cx)
                                });
                            }),
                    )
                })
            });
        let saving = form.saving.is_some_and(|(key, request)| {
            self.connection(key)
                .is_some_and(|connection| connection.device_tasks.asked(Some(request)))
        });
        let label =
            |text: Cow<'static, str>| div().text_xs().font_semibold().child(text.into_owned());
        let buttons = h_flex()
            .gap_2()
            .items_center()
            .child(
                Button::new("task-form-save")
                    .debug_selector(|| "task-form-save".into())
                    .primary()
                    .small()
                    .label(t!("tasks.form.save"))
                    .disabled(saving || !form.can_save(cx))
                    .on_click(move |_, _, cx| {
                        let _ = save_owner.update(cx, |this, cx| this.save_task_form(cx));
                    }),
            )
            .child(div().flex_1())
            .map(|this| match form.home {
                Some((key, task_id)) => {
                    let (archive_owner, delete_owner) = (owner.clone(), owner.clone());
                    let title = form
                        .saved
                        .as_ref()
                        .map(|saved| saved.title.clone())
                        .unwrap_or_default();
                    this.child(
                        Button::new("task-form-archive")
                            .debug_selector(|| "task-form-archive".into())
                            .outline()
                            .small()
                            .label(t!("tasks.form.archive"))
                            .on_click(move |_, _, cx| {
                                let _ = archive_owner
                                    .update(cx, |this, cx| this.archive_todo(key, task_id, cx));
                            }),
                    )
                    .child(
                        Button::new("task-form-delete")
                            .debug_selector(|| "task-form-delete".into())
                            .outline()
                            .small()
                            .label(t!("tasks.form.delete"))
                            .on_click(move |_, window, cx| {
                                let title = title.clone();
                                let _ = delete_owner.update(cx, |this, cx| {
                                    this.confirm_delete_task(key, task_id, title, window, cx)
                                });
                            }),
                    )
                }
                None => {
                    let cancel_owner = owner.clone();
                    this.child(
                        Button::new("task-form-cancel")
                            .debug_selector(|| "task-form-cancel".into())
                            .outline()
                            .small()
                            .label(t!("tasks.form.cancel"))
                            .on_click(move |_, _, cx| {
                                // A new Task's Cancel discards it without asking.
                                let _ = cancel_owner.update(cx, |this, cx| {
                                    this.task_form = None;
                                    cx.notify();
                                });
                            }),
                    )
                }
            });
        v_flex()
            .id("task-form")
            .debug_selector(|| "task-form".into())
            .flex_1()
            .min_w_0()
            .gap_2()
            .child(label(t!("tasks.form.title")))
            .child(
                div()
                    .debug_selector(|| "task-form-title".into())
                    .child(Input::new(&form.title).small()),
            )
            .children(super::dialogs::field_error(
                form.error_for(FormField::Title),
                cx,
            ))
            .child(
                h_flex()
                    .gap_1()
                    .child(label(t!("tasks.form.description")))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(t!("tasks.form.description_hint").into_owned()),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "task-form-description".into())
                    // Typing before it arrives would be overwritten.
                    .child(Textarea::new(&form.description).disabled(!form.description_loaded())),
            )
            .children(super::dialogs::field_error(
                form.error_for(FormField::Description),
                cx,
            ))
            .children(form.description_error.clone().map(|error| {
                let retry_owner = owner.clone();
                h_flex()
                    .debug_selector(|| "task-form-description-error".into())
                    .gap_2()
                    .items_center()
                    .children(super::dialogs::field_error(
                        Some(t!("tasks.form.load_failed", error = error).into_owned()),
                        cx,
                    ))
                    .child(
                        Button::new("task-form-retry")
                            .debug_selector(|| "task-form-retry".into())
                            .ghost()
                            .xsmall()
                            .label(t!("tasks.form.retry"))
                            .on_click(move |_, _, cx| {
                                let _ = retry_owner
                                    .update(cx, |this, cx| this.fetch_form_description(cx));
                            }),
                    )
            }))
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(label(t!("tasks.form.device")))
                            .child(device),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(label(t!("tasks.form.repository")))
                            .child(repository),
                    ),
            )
            .child(buttons)
            .children(super::dialogs::field_error(
                form.error_for(FormField::Form),
                cx,
            ))
            .into_any_element()
    }

    fn render_task_start(
        &self,
        form: &TaskForm,
        task: &TaskRecord,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(plan) = self.start_plan(form, cx) else {
            return div().into_any_element();
        };
        let connection = self.connection(form.home.map_or(form.device, |(key, _)| key));
        let owner = cx.weak_entity();
        let options = &plan.options;
        let muted = |text: String| {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let note = |text: String| div().pl(rems(1.5)).child(muted(text));
        let radio = |id: &'static str, place: Where, label: String, enabled: bool| {
            let owner = owner.clone();
            div().debug_selector(move || id.into()).child(
                Radio::new(id)
                    .small()
                    .label(label)
                    .checked(plan.place == place && enabled)
                    .disabled(!enabled)
                    .on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| this.choose_task_where(place, cx));
                    }),
            )
        };
        let mut place = v_flex().gap_1();
        if let Some((name, block)) = &options.continue_in {
            place = place.child(radio(
                "task-where-continue",
                Where::Continue,
                t!("tasks.start.continue", branch = name).into_owned(),
                block.is_none(),
            ));
            place = place.children(match block {
                Some(ContinueBlock::Missing) => {
                    Some(note(t!("tasks.start.folder_missing").into_owned()))
                }
                Some(ContinueBlock::Held(title)) => {
                    Some(note(t!("tasks.start.held", title = title).into_owned()))
                }
                None if plan.place == Where::Continue => task
                    .last
                    .as_ref()
                    .map(|last| note(last.path.display().to_string())),
                None => None,
            });
        }
        place = place.child(radio(
            "task-where-existing",
            Where::Existing,
            t!("tasks.start.existing").into_owned(),
            options.allows(Where::Existing),
        ));
        if plan.place == Where::Existing && options.allows(Where::Existing) {
            let chosen = plan.workspace;
            let workspaces = options.workspaces.clone();
            let label = workspaces
                .iter()
                .find(|each| Some(each.id) == chosen)
                .map(|each| each.name.clone())
                .unwrap_or_default();
            let workspace_owner = owner.clone();
            let agents = chosen
                .and_then(|workspace| {
                    let connection = connection?;
                    let workspace = connection.session()?.workspace(workspace)?;
                    Some(
                        workspace
                            .tabs()
                            .iter()
                            .flat_map(|tab| tab.panes())
                            .filter(|pane| connection.model.agents.contains_key(&pane.id()))
                            .count(),
                    )
                })
                .unwrap_or(0);
            place = place.child(
                v_flex()
                    .pl(rems(1.5))
                    .gap_1()
                    .child(
                        Button::new("task-start-workspace")
                            .debug_selector(|| "task-start-workspace".into())
                            .outline()
                            .small()
                            .label(label)
                            .dropdown_caret(true)
                            .dropdown_menu(move |menu, _, _| {
                                workspaces.iter().fold(menu, |menu, each| {
                                    let (owner, id) = (workspace_owner.clone(), each.id);
                                    let name = match &each.holder {
                                        Some(holder) => format!("{} — {holder}", each.name),
                                        None => each.name.clone(),
                                    };
                                    menu.item(
                                        PopupMenuItem::new(name)
                                            .checked(Some(id) == chosen)
                                            .disabled(each.holder.is_some())
                                            .on_click(move |_, _, cx| {
                                                let _ = owner.update(cx, |this, cx| {
                                                    this.choose_task_workspace(id, cx)
                                                });
                                            }),
                                    )
                                })
                            }),
                    )
                    .when(agents > 0, |this| {
                        this.child(
                            div()
                                .debug_selector(|| "task-start-has-agents".into())
                                .text_xs()
                                .text_color(theme.warning)
                                .child(
                                    if agents == 1 {
                                        t!("tasks.start.has_agents.one", count = agents)
                                    } else {
                                        t!("tasks.start.has_agents.other", count = agents)
                                    }
                                    .into_owned(),
                                ),
                        )
                    }),
            );
        }
        place = place.child(radio(
            "task-where-new-worktree",
            Where::NewWorktree,
            t!("tasks.start.new_worktree").into_owned(),
            options.new_worktree.is_ok(),
        ));
        place = place.child(match &options.new_worktree {
            Ok(repository) if plan.place == Where::NewWorktree => v_flex()
                .pl(rems(1.5))
                .gap_1()
                .child(muted(
                    t!("tasks.start.from", repository = repository).into_owned(),
                ))
                .child(
                    div()
                        .debug_selector(|| "task-start-branch".into())
                        .child(Input::new(&form.start.branch).small()),
                )
                .children(super::dialogs::field_error(
                    form.start.branch_error.clone(),
                    cx,
                ))
                .into_any_element(),
            Ok(_) => div().into_any_element(),
            Err(WorktreeBlock::NoRepository) => {
                note(t!("tasks.start.no_repository").into_owned()).into_any_element()
            }
            Err(WorktreeBlock::NotOpen(name)) => {
                note(t!("tasks.start.open_first", repository = name).into_owned())
                    .debug_selector(|| "task-start-open-first".into())
                    .into_any_element()
            }
        });

        let device = connection
            .map(|connection| connection.label.clone())
            .unwrap_or_default();
        let agent_row = match &plan.available {
            None => muted(t!("tasks.start.checking").into_owned()).into_any_element(),
            Some(available) if available.is_empty() => {
                muted(t!("tasks.start.no_agents", device = device).into_owned()).into_any_element()
            }
            Some(available) => {
                let available = available.clone();
                let chosen = plan.agent;
                let agent_owner = owner.clone();
                let last = self.last_task_agent;
                let mut ordered = available.clone();
                ordered.sort_by_key(|agent| Some(*agent) != last);
                Button::new("task-start-agent")
                    .debug_selector(|| "task-start-agent".into())
                    .outline()
                    .small()
                    .label(chosen.map(AgentKind::label).unwrap_or_default())
                    .dropdown_caret(true)
                    .dropdown_menu(move |menu, _, _| {
                        ordered.iter().fold(menu, |menu, agent| {
                            let (owner, agent) = (agent_owner.clone(), *agent);
                            menu.item(
                                PopupMenuItem::new(agent.label())
                                    .checked(Some(agent) == chosen)
                                    .on_click(move |_, _, cx| {
                                        let _ = owner.update(cx, |this, cx| {
                                            this.choose_task_agent(agent, cx)
                                        });
                                    }),
                            )
                        })
                    })
                    .into_any_element()
            }
        };
        let hooks_missing = plan.agent.filter(|agent| {
            connection.is_some_and(|connection| {
                connection
                    .hooks
                    .iter()
                    .any(|report| report.agent == *agent && report.state == HooksState::Missing)
            })
        });
        let hooks = hooks_missing.map(|agent| {
            let (hooks_owner, key) = (
                owner.clone(),
                connection.map_or(form.device, |connection| connection.key),
            );
            h_flex()
                .debug_selector(|| "task-start-hooks".into())
                .gap_2()
                .items_center()
                .child(muted(t!("tasks.start.hooks_missing").into_owned()))
                .child(
                    Button::new("task-start-install-hooks")
                        .debug_selector(|| "task-start-install-hooks".into())
                        .ghost()
                        .xsmall()
                        .label(t!("tasks.start.install"))
                        .on_click(move |_, _, cx| {
                            let _ = hooks_owner.update(cx, |this, cx| {
                                if let Some(connection) = this.connection_mut(key) {
                                    connection.send_hooks_request(
                                        agent,
                                        HooksAction::Install,
                                        AgentRequest::InstallHooks,
                                    );
                                }
                                cx.notify();
                            });
                        }),
                )
        });
        let start_owner = owner.clone();
        let blocked = plan.blocked.clone();
        let label = match plan.agent {
            Some(agent) => t!("tasks.start.button", agent = agent.label()),
            None => t!("tasks.start.heading"),
        };
        let heading =
            |text: Cow<'static, str>| div().text_xs().font_semibold().child(text.into_owned());
        v_flex()
            .debug_selector(|| "task-start".into())
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child(t!("tasks.start.heading").into_owned()),
            )
            .child(heading(t!("tasks.start.where")))
            .child(place)
            .child(heading(t!("tasks.start.agent")))
            .child(agent_row)
            .children(hooks)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("task-start-button")
                            .debug_selector(|| "task-start-button".into())
                            .primary()
                            .small()
                            .label(label)
                            .disabled(blocked.is_some() || plan.agent.is_none())
                            .on_click(move |_, _, cx| {
                                let _ = start_owner.update(cx, |this, cx| this.start_task(cx));
                            }),
                    )
                    .children(blocked.map(|blocked| {
                        div()
                            .debug_selector(|| "task-start-blocked".into())
                            .text_xs()
                            .text_color(theme.warning)
                            .child(blocked)
                    })),
            )
            .into_any_element()
    }
}

/// What Start would do now; `blocked` says why it cannot.
pub(super) struct StartPlan {
    pub(super) options: WhereOptions,
    pub(super) place: Where,
    pub(super) workspace: Option<WorkspaceId>,
    pub(super) available: Option<Vec<AgentKind>>,
    pub(super) agent: Option<AgentKind>,
    pub(super) blocked: Option<String>,
}

/// What the archive dialog says about a started Task's Workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ArchiveFacts {
    pub(super) title: String,
    /// A Start of this Task made the Workspace, as the Server recorded at its commit.
    pub(super) created: bool,
    pub(super) agents: usize,
    /// One Agent that waits for the person, else one that works.
    pub(super) busy: Option<(AgentKind, AgentDisplayState)>,
    pub(super) managed: bool,
    pub(super) dirty: bool,
    pub(super) branch: Option<String>,
}

fn render_archive_choice(
    facts: &ArchiveFacts,
    choice: &Entity<(bool, bool)>,
    cx: &mut App,
) -> AnyElement {
    let (close, remove) = *choice.read(cx);
    let theme = cx.theme().clone();
    let muted = |text: String| {
        div()
            .pl(rems(1.5))
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(text)
    };
    let ending = if facts.agents == 1 {
        t!("tasks.archive.agents_end.one", count = facts.agents)
    } else {
        t!("tasks.archive.agents_end.other", count = facts.agents)
    };
    let busy = facts.busy.map(|(kind, state)| match state {
        AgentDisplayState::Blocked => t!("tasks.archive.waiting", agent = kind.label()),
        _ => t!("tasks.archive.working", agent = kind.label()),
    });
    let close_choice = choice.clone();
    let remove_choice = choice.clone();
    v_flex()
        .gap_2()
        .child(
            div().debug_selector(|| "task-archive-close".into()).child(
                Checkbox::new("task-archive-close")
                    .label(SharedString::from(t!("tasks.archive.close").into_owned()))
                    .checked(close)
                    .on_click(move |checked, _, cx| {
                        close_choice.update(cx, |choice, cx| {
                            choice.0 = *checked;
                            cx.notify();
                        });
                    }),
            ),
        )
        .child(
            h_flex()
                .gap_1()
                .child(muted(ending.into_owned()))
                .children(busy.map(|busy| {
                    div()
                        .text_sm()
                        .text_color(theme.warning)
                        .child(busy.into_owned())
                })),
        )
        .when(facts.managed, |this| {
            let label = match &facts.branch {
                Some(branch) => t!("tasks.archive.remove", branch = branch),
                None => t!("tasks.archive.remove_no_branch"),
            };
            this.child(
                div().debug_selector(|| "task-archive-remove".into()).child(
                    Checkbox::new("task-archive-remove")
                        .label(SharedString::from(label.into_owned()))
                        .checked(close && remove && !facts.dirty)
                        .disabled(!close || facts.dirty)
                        .on_click(move |checked, _, cx| {
                            remove_choice.update(cx, |choice, cx| {
                                choice.1 = *checked;
                                cx.notify();
                            });
                        }),
                ),
            )
            .when(facts.dirty, |this| {
                this.child(muted(t!("tasks.archive.dirty").into_owned()))
            })
        })
        .when(!close, |this| {
            this.child(muted(t!("tasks.archive.kept").into_owned()))
        })
        .into_any_element()
}

/// The Device's To do Tasks in the Server's order: not started, archived ones never
/// being in the model.
pub(super) fn todo_tasks(connection: &ServerConnection) -> Vec<&TaskRecord> {
    let mut todo: Vec<&TaskRecord> = connection
        .model
        .tasks
        .iter()
        .flat_map(|tasks| tasks.values())
        .filter(|task| task.workspace.is_none())
        .collect();
    todo.sort_by_key(|task| (task.order, task.id));
    todo
}

/// The Device's repository Workspaces, which a Task's repository is one of: the root of
/// a Git repository, not a worktree made from one.
pub(super) fn repository_workspaces(
    connection: &ServerConnection,
) -> impl Iterator<Item = (WorkspaceId, String, PathBuf)> + '_ {
    connection
        .session()
        .into_iter()
        .flat_map(|session| session.workspaces())
        .filter(|workspace| {
            workspace.worktree().is_none()
                && connection.model.workspace_git.contains_key(&workspace.id())
        })
        .map(|workspace| {
            (
                workspace.id(),
                workspace.name().to_owned(),
                workspace.root_directory().to_path_buf(),
            )
        })
}

/// A repository by its Workspace's name, or its folder's when none is open.
pub(super) fn repository_name(connection: Option<&ServerConnection>, root: &Path) -> String {
    connection
        .and_then(|connection| connection.session()?.workspace_by_root(root))
        .map(|workspace| workspace.name().to_owned())
        .unwrap_or_else(|| folder_name(root))
}

/// The Pane of `workspace_id` this Client shows focused, as the Workspace would open.
pub(super) fn workspace_focused_pane(
    connection: &ServerConnection,
    session: &Session,
    workspace_id: WorkspaceId,
) -> Option<PaneId> {
    let tab = connection.viewed_tab_id(session, workspace_id)?;
    session.tab(tab)?.focused_pane().map(|pane| pane.id())
}

pub(super) fn workspace_has_agent(
    connection: &ServerConnection,
    workspace_id: WorkspaceId,
) -> bool {
    connection
        .session()
        .and_then(|session| session.workspace(workspace_id))
        .is_some_and(|workspace| {
            workspace
                .tabs()
                .iter()
                .flat_map(|tab| tab.panes())
                .any(|pane| connection.model.agents.contains_key(&pane.id()))
        })
}

/// The branch New worktree offers: the Server's rule for the title, or for a Task worked
/// before, the branch it had.
fn default_branch(task: &TaskRecord) -> String {
    match task.repository.as_deref() {
        Some(repository) => condr_core::default_task_branch(&task.title, repository),
        None => condr_core::default_task_branch(&task.title, Path::new("")),
    }
}

fn saved_task(task: &TaskRecord) -> SavedTask {
    SavedTask {
        title: task.title.clone(),
        description: task.description.clone(),
        repository: task.repository.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ContinueBlock, Where, WorktreeBlock, where_options};
    use condr_core::{LastPlace, Session, Task, TaskId, WorkspaceId};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn task(id: u64, repository: Option<&str>, workspace: Option<WorkspaceId>) -> Task {
        Task {
            id: TaskId::from_u64(id),
            title: format!("task {id}"),
            description: None,
            repository: repository.map(PathBuf::from),
            order: 0,
            archived: false,
            workspace,
            made_workspace: false,
            starting: false,
            last: None,
        }
    }

    fn tasks(list: &[&Task]) -> BTreeMap<TaskId, Task> {
        list.iter().map(|task| (task.id, (*task).clone())).collect()
    }

    #[test]
    fn where_defaults_follow_the_adr_order() {
        let mut session = Session::new();
        let repository = session.create_workspace("/src/api".into()).unwrap();
        let other = session.create_workspace("/src/web".into()).unwrap();

        // The repository's own Workspace, free: Existing takes it.
        let todo = task(1, Some("/src/api"), None);
        let options = where_options(&session, &tasks(&[&todo]), &todo, false);
        assert_eq!(options.default, Some(Where::Existing));
        assert_eq!(options.existing, Some(repository));
        assert_eq!(options.new_worktree, Ok("api".into()));
        assert_eq!(options.continue_in, None);

        // Another Task holds it: listed greyed with that Task's title, and a new
        // worktree is the default.
        let holder = task(2, None, Some(repository));
        let options = where_options(&session, &tasks(&[&todo, &holder]), &todo, false);
        assert_eq!(options.default, Some(Where::NewWorktree));
        assert_eq!(options.workspaces[0].holder.as_deref(), Some("task 2"));
        assert_eq!(options.existing, Some(other));

        // Worked before: Continue, unless the folder is gone, which makes a new
        // worktree the default, or another Task holds the Workspace open on it.
        let mut worked = todo.clone();
        worked.last = Some(LastPlace {
            path: "/src/web".into(),
            branch: Some("worktree/fix".into()),
            base_branch: None,
            managed: true,
            repository: Some("/src/api".into()),
        });
        let options = where_options(&session, &tasks(&[&worked]), &worked, false);
        assert_eq!(options.default, Some(Where::Continue));
        assert_eq!(options.continue_in, Some(("worktree/fix".into(), None)));
        let options = where_options(&session, &tasks(&[&worked]), &worked, true);
        assert_eq!(
            options.continue_in,
            Some(("worktree/fix".into(), Some(ContinueBlock::Missing)))
        );
        assert_eq!(options.default, Some(Where::NewWorktree));
        let web_holder = task(3, None, Some(other));
        let options = where_options(&session, &tasks(&[&worked, &web_holder]), &worked, false);
        assert_eq!(
            options.continue_in.unwrap().1,
            Some(ContinueBlock::Held("task 3".into()))
        );

        // No repository: the first free Workspace, and no new worktree.
        let loose = task(4, None, None);
        let options = where_options(&session, &tasks(&[&loose, &holder]), &loose, false);
        assert_eq!(options.default, Some(Where::Existing));
        assert_eq!(options.existing, Some(other));
        assert_eq!(options.new_worktree, Err(WorktreeBlock::NoRepository));

        // A repository not open, and every Workspace held: nothing to offer.
        let closed = task(5, Some("/src/cli"), None);
        let all_held = task(6, None, Some(other));
        let options = where_options(
            &session,
            &tasks(&[&closed, &holder, &all_held]),
            &closed,
            false,
        );
        assert_eq!(
            options.new_worktree,
            Err(WorktreeBlock::NotOpen("cli".into()))
        );
        assert_eq!(options.default, None);
        assert!(!options.allows(Where::Existing));
    }
}
