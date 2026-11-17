//! A Device's Task requests and what their answers bring: requests in flight, the
//! Server's replies, and the Task events and Bootstrap that keep the model (ADR 0068).

use super::*;
use condr_core::agent_discovery::AgentInstallation;
use condr_core::{TaskCommand, TaskError, TaskId, TaskResponse};
use rust_i18n::t;
use std::collections::VecDeque;

/// How long a started Task's card reads "Starting…" before, with still no Agent in its
/// Workspace, it reads "No agent running" (ADR 0068).
pub(in crate::app) const AGENT_WAIT: Duration = Duration::from_secs(30);

/// An Agent request still owed an `AgentResult`, which carries no request id; the Server
/// answers these in the order asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum AgentRequest {
    Hooks,
    Available,
    /// Install from the Start half, whose failure is a toast there (ADR 0020).
    InstallHooks,
}

/// What a Task request was for, so its answer lands where it belongs.
pub(in crate::app) enum TaskRequest {
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
pub(in crate::app) struct DeviceTasks {
    pub(in crate::app) pending: HashMap<u64, TaskRequest>,
    pub(in crate::app) agent_requests: VecDeque<AgentRequest>,
    /// The Agents on the Device's PATH that can start a Task, in `AgentKind::ALL`'s
    /// order; `None` until asked.
    pub(in crate::app) available: Option<Vec<AgentKind>>,
    /// The archived Tasks, fetched each time To do's Archived list unfolds.
    pub(in crate::app) archived: Option<Vec<TaskRecord>>,
    /// Started Tasks whose Agent may still be coming: when each was seen to start, and
    /// whether this Client started it.
    pub(in crate::app) waiting: HashMap<TaskId, (Instant, bool)>,
}

impl DeviceTasks {
    /// The stream is gone, and every answer it owed with it.
    pub(in crate::app) fn connection_closed(&mut self) {
        self.pending.clear();
        self.agent_requests.clear();
        self.available = None;
    }

    pub(in crate::app) fn agents_available(&mut self, installations: Vec<AgentInstallation>) {
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

    pub(super) fn asked(&self, request: Option<u64>) -> bool {
        request.is_some_and(|request| self.pending.contains_key(&request))
    }
}

impl Condr {
    /// The connected Devices that keep Tasks: a Server that sent no Tasks record, older
    /// or built without them, offers no To do and takes no new Task (ADR 0068).
    pub(in crate::app) fn task_devices(&self) -> impl Iterator<Item = &ServerConnection> {
        self.connections.iter().filter(|connection| {
            connection.status == ConnectionStatus::Connected && connection.model.tasks.is_some()
        })
    }

    /// How many Tasks wait in To do across the connected Devices, for the sidebar's row.
    pub(in crate::app) fn todo_count(&self) -> usize {
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

    /// A Task request's answer.
    pub(in crate::app) fn task_answered(
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
                        let error = t!("tasks.not_connected", device = old);
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
    pub(super) fn in_main_window(
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
    pub(in crate::app) fn task_changed(
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

    pub(in crate::app) fn task_removed(
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
    pub(in crate::app) fn tasks_bootstrapped(
        &mut self,
        key: ConnectionKey,
        cx: &mut Context<Self>,
    ) {
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
    pub(super) fn reconcile_task_form(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn send_task_change(
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
            self.report_error(t!("tasks.not_connected", device = label), cx);
        }
        cx.notify();
    }
}
