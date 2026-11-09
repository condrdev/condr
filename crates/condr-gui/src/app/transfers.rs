//! Downloads and uploads in the GUI (ADR 0061): Download… on a file in Files, Changes and
//! a Preview Tab's menu, Upload by a drop or Upload Files… on a directory, at most four
//! at once to a Device, and their progress in a strip at the bottom of the right sidebar.
//! They ride the Device's Stream connection, the one its forwards use.

use super::dialogs::{DangerConfirm, confirm_danger};
use super::*;
use condr_client::{StreamConnection, StreamLink, TransferCancel};
use condr_core::protocol::{StreamError, StreamFile};
use std::sync::{Arc, Mutex, PoisonError};

/// Transfers to one Device that run at once; the rest wait in the strip.
const RUNNING_PER_DEVICE: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TransferState {
    Queued,
    Running,
    Done,
    Failed(String),
    Cancelled,
}

#[derive(Clone, Debug)]
enum Direction {
    Download { destination: PathBuf },
    Upload { source: PathBuf, replace: bool },
}

/// One file on its way, this Client's own (ADR 0061).
pub(super) struct Transfer {
    id: u64,
    pub(super) workspace: WorkspaceId,
    path: RelativePathBuf,
    direction: Direction,
    /// Bytes done and the total, overwritten by the transfer, read when the GUI repaints.
    progress: Arc<Mutex<(u64, Option<u64>)>>,
    cancel: TransferCancel,
    pub(super) state: TransferState,
}

impl Transfer {
    fn name(&self) -> String {
        self.path.file_name().unwrap_or_default().to_owned()
    }

    fn live(&self) -> bool {
        matches!(self.state, TransferState::Queued | TransferState::Running)
    }
}

impl Condr {
    /// Asks where to save `path`, then queues its download.
    pub(super) fn download_file(
        &mut self,
        key: ConnectionKey,
        workspace: WorkspaceId,
        path: RelativePathBuf,
        cx: &mut Context<Self>,
    ) {
        let directory = self
            .last_download_directory
            .clone()
            .or_else(|| std::env::home_dir().map(|home| home.join("Downloads")))
            .unwrap_or_default();
        let suggested = condr_core::local_name(path.file_name().unwrap_or("download"));
        let chosen = cx.prompt_for_new_path(&directory, Some(&suggested));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(destination))) = chosen.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.last_download_directory =
                    destination.parent().map(std::path::Path::to_path_buf);
                // The save dialog asked about replacing an existing file.
                this.queue_transfer(
                    key,
                    workspace,
                    path,
                    Direction::Download { destination },
                    cx,
                );
            });
        })
        .detach();
    }

    /// Upload Files… on a directory or the space below the tree.
    pub(super) fn pick_uploads(
        &mut self,
        key: ConnectionKey,
        workspace: WorkspaceId,
        directory: RelativePathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Upload".into()),
        });
        let owner = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let Ok(Ok(Some(paths))) = picked.await else {
                    return;
                };
                let _ = cx.update(|window, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.upload_files(key, workspace, directory, paths, window, cx)
                    });
                });
            })
            .detach();
    }

    /// One batch, dropped or picked: folders are refused, names the directory already
    /// lists are asked about once, and the rest queue at once; one the Device refuses as
    /// existing, in a folded directory or made meanwhile, is asked about then.
    pub(super) fn upload_files(
        &mut self,
        key: ConnectionKey,
        workspace: WorkspaceId,
        directory: RelativePathBuf,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (folders, files): (Vec<_>, Vec<_>) = paths.into_iter().partition(|path| path.is_dir());
        for folder in folders {
            let name = folder.file_name().unwrap_or_default().to_string_lossy();
            self.report_error(
                format!("“{name}” is a folder. Only files can be uploaded; archive it first."),
                cx,
            );
        }
        let listed: HashSet<String> = self
            .connection(key)
            .and_then(|connection| {
                connection
                    .resources
                    .directories
                    .get(&(workspace, directory.clone()))
            })
            .and_then(|listing| listing.as_ref().ok())
            .map(|listing| {
                listing
                    .entries
                    .iter()
                    .map(|entry| entry.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let named = |source: &PathBuf| {
            source
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        };
        let (clashing, fresh): (Vec<_>, Vec<_>) = files
            .into_iter()
            .filter(|source| named(source).is_some())
            .partition(|source| named(source).is_some_and(|name| listed.contains(&name)));
        for source in fresh {
            self.queue_upload(key, workspace, &directory, source, false, cx);
        }
        self.confirm_replace(key, workspace, directory, clashing, window, cx);
    }

    /// Asks once whether to replace `clashing`, files that exist in `directory`.
    fn confirm_replace(
        &mut self,
        key: ConnectionKey,
        workspace: WorkspaceId,
        directory: RelativePathBuf,
        clashing: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let named = |source: &PathBuf| {
            source
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        };
        if clashing.is_empty() {
            return;
        }
        let names: Vec<String> = clashing.iter().filter_map(named).collect();
        let place = if directory.as_str().is_empty() {
            "the Root Directory".to_owned()
        } else {
            directory.to_string()
        };
        let title = match names.as_slice() {
            [one] => format!("“{one}” already exists in {place}"),
            many => format!("{} files already exist in {place}", many.len()),
        };
        let description = format!(
            "{}\n\nReplacing overwrites {} on the Device. This cannot be undone.",
            names.join(", "),
            if names.len() == 1 { "it" } else { "them" }
        );
        let confirm = DangerConfirm {
            title,
            description,
            ok: "Replace",
            cancel: if names.len() == 1 {
                "Skip"
            } else {
                "Skip These"
            },
        };
        confirm_danger(
            cx.weak_entity(),
            confirm,
            move |this, _, cx| {
                for source in &clashing {
                    this.queue_upload(key, workspace, &directory, source.clone(), true, cx);
                }
            },
            window,
            cx,
        );
    }

    fn queue_upload(
        &mut self,
        key: ConnectionKey,
        workspace: WorkspaceId,
        directory: &RelativePath,
        source: PathBuf,
        replace: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = source.file_name().and_then(|name| name.to_str()) else {
            return;
        };
        let path = directory.join(name);
        self.queue_transfer(
            key,
            workspace,
            path,
            Direction::Upload { source, replace },
            cx,
        );
    }

    fn queue_transfer(
        &mut self,
        key: ConnectionKey,
        workspace: WorkspaceId,
        path: RelativePathBuf,
        direction: Direction,
        cx: &mut Context<Self>,
    ) {
        self.next_transfer_id += 1;
        let id = self.next_transfer_id;
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        // The Stream connection lives only while the terminal connection is up (ADR 0061).
        if connection.status != ConnectionStatus::Connected {
            let error = format!("{} is not connected", connection.label);
            return self.report_error(error, cx);
        }
        connection.transfers.push(Transfer {
            id,
            workspace,
            path,
            direction,
            progress: Arc::default(),
            cancel: TransferCancel::default(),
            state: TransferState::Queued,
        });
        self.start_transfers(key, cx);
        cx.notify();
    }

    /// Starts queued transfers while fewer than four run to this Device.
    fn start_transfers(&mut self, key: ConnectionKey, cx: &mut Context<Self>) {
        let events = self.ports_events(cx);
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        // Lost meanwhile: what waits would dial a Device that is not there.
        if connection.status != ConnectionStatus::Connected {
            let reason = format!("{} disconnected", connection.label);
            for transfer in &mut connection.transfers {
                if transfer.state == TransferState::Queued {
                    transfer.state = TransferState::Failed(reason.clone());
                }
            }
            return;
        }
        let running = connection
            .transfers
            .iter()
            .filter(|transfer| transfer.state == TransferState::Running)
            .count();
        let link = connection
            .forwards
            .link(&connection.label, &connection.endpoint, &events);
        let mut started = Vec::new();
        for transfer in connection
            .transfers
            .iter_mut()
            .filter(|transfer| transfer.state == TransferState::Queued)
            .take(RUNNING_PER_DEVICE.saturating_sub(running))
        {
            transfer.state = TransferState::Running;
            started.push((
                transfer.id,
                StreamFile {
                    workspace: transfer.workspace,
                    path: transfer.path.clone(),
                },
                transfer.direction.clone(),
                Arc::clone(&transfer.progress),
                transfer.cancel.clone(),
            ));
        }
        for (id, file, direction, progress, cancel) in started {
            let link = Arc::clone(&link);
            let events = events.clone();
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_spawn(async move {
                        run_transfer(&link, file, &direction, &progress, &cancel, &events)
                    })
                    .await;
                let _ = this.update(cx, |this, cx| this.transfer_ended(key, id, result, cx));
            })
            .detach();
        }
    }

    fn transfer_ended(
        &mut self,
        key: ConnectionKey,
        id: u64,
        result: Result<(), condr_client::StreamFailure>,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let label = connection.label.clone();
        let Some(index) = connection
            .transfers
            .iter()
            .position(|transfer| transfer.id == id)
        else {
            return;
        };
        let exists = matches!(&result, Err(failure) if failure.error == StreamError::AlreadyExists);
        if let Direction::Upload {
            source,
            replace: false,
        } = &connection.transfers[index].direction
            && exists
        {
            let clashing = vec![source.clone()];
            let transfer = connection.transfers.remove(index);
            let directory = transfer
                .path
                .parent()
                .map(RelativePath::to_relative_path_buf)
                .unwrap_or_default();
            let handle = self.active_view().handle;
            let owner = cx.weak_entity();
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        let workspace = transfer.workspace;
                        this.confirm_replace(key, workspace, directory, clashing, window, cx);
                    });
                });
            });
            self.start_transfers(key, cx);
            cx.notify();
            return;
        }
        let transfer = &mut connection.transfers[index];
        let name = transfer.name();
        let downloaded = match &transfer.direction {
            Direction::Download { destination } => Some(destination.clone()),
            Direction::Upload { .. } => None,
        };
        transfer.state = match &result {
            Ok(()) => TransferState::Done,
            Err(failure) if failure.error == StreamError::Cancelled => TransferState::Cancelled,
            Err(failure) => TransferState::Failed(failure.to_string()),
        };
        match result {
            Ok(()) => {
                if let Some(destination) = downloaded {
                    self.report_downloaded(&name, destination, cx);
                }
            }
            Err(failure) if failure.error == StreamError::Cancelled => {}
            Err(failure) => {
                let verb = if downloaded.is_some() {
                    "download"
                } else {
                    "upload"
                };
                self.report_error(format!("Could not {verb} {name} on {label}: {failure}"), cx);
            }
        }
        self.start_transfers(key, cx);
        cx.notify();
    }

    /// A finished download's toast, with the reveal action of ADR 0045.
    pub(super) fn report_downloaded(&self, name: &str, destination: PathBuf, cx: &mut App) {
        struct DownloadToast;
        let message: SharedString = format!("Downloaded {name}").into();
        let handle = self.active_view().handle;
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let destination = destination.clone();
                let note = Notification::new()
                    .message(message.clone())
                    .id1::<DownloadToast>(message.clone())
                    .action(move |_, _, _| {
                        let destination = destination.clone();
                        Button::new("reveal-download")
                            .icon(IconName::FolderOpen)
                            .tooltip(super::file_entries::reveal_label())
                            .small()
                            .ghost()
                            .on_click(move |_, _, _| {
                                let _ = super::open_in::reveal_entry(&destination);
                            })
                    });
                window.push_notification(note, cx);
            });
        });
    }

    pub(super) fn cancel_transfer(&mut self, key: ConnectionKey, id: u64, cx: &mut Context<Self>) {
        if let Some(transfer) = self.connection_mut(key).and_then(|connection| {
            connection
                .transfers
                .iter_mut()
                .find(|transfer| transfer.id == id)
        }) {
            transfer.cancel.cancel();
            if transfer.state == TransferState::Queued {
                transfer.state = TransferState::Cancelled;
            }
        }
        cx.notify();
    }

    fn clear_transfers(
        &mut self,
        key: ConnectionKey,
        workspace: WorkspaceId,
        cx: &mut Context<Self>,
    ) {
        if let Some(connection) = self.connection_mut(key) {
            connection
                .transfers
                .retain(|transfer| transfer.workspace != workspace || transfer.live());
        }
        cx.notify();
    }

    /// Quitting cancels every transfer and gives them up to two seconds to remove what
    /// they wrote beside their targets, which their threads do once woken.
    pub(super) fn cancel_transfers_before_quit(&self) {
        let cancels: Vec<_> = self
            .connections
            .iter()
            .flat_map(|connection| &connection.transfers)
            .filter(|transfer| transfer.live())
            .map(|transfer| transfer.cancel.clone())
            .collect();
        for cancel in &cancels {
            cancel.cancel();
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while cancels.iter().any(|cancel| !cancel.settled()) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Transfers running or waiting, which quitting asks about.
    pub(super) fn live_transfers(&self) -> usize {
        self.connections
            .iter()
            .flat_map(|connection| &connection.transfers)
            .filter(|transfer| transfer.live())
            .count()
    }

    /// The strip at the bottom of the right sidebar: nothing without transfers, one line
    /// while they run or wait, a row per file unfolded, "finished" until Clear.
    pub(super) fn render_transfer_strip(
        &self,
        key: ConnectionKey,
        workspace: WorkspaceId,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let connection = self.connection(key)?;
        let transfers: Vec<&Transfer> = connection
            .transfers
            .iter()
            .filter(|transfer| transfer.workspace == workspace)
            .collect();
        if transfers.is_empty() {
            return None;
        }
        let live: Vec<&&Transfer> = transfers
            .iter()
            .filter(|transfer| transfer.live())
            .collect();
        // The percentage follows what moves; a file waiting its turn counts, not weighs.
        let (done, total) = live
            .iter()
            .filter(|transfer| transfer.state == TransferState::Running)
            .fold((0u64, 0u64), |(done, total), transfer| {
                let (bytes, size) = *transfer
                    .progress
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                (done + bytes, total + size.unwrap_or(bytes))
            });
        let headline = if live.is_empty() {
            format!("⇅ {} finished", transfers.len())
        } else {
            let percent = (done * 100).checked_div(total).unwrap_or(0);
            match live.len() {
                1 => format!("⇅ 1 transfer · {percent}%"),
                count => format!("⇅ {count} transfers · {percent}%"),
            }
        };
        let open = self.transfer_strip_open;
        let toggle_owner = cx.weak_entity();
        let clear_owner = cx.weak_entity();
        let mut strip = v_flex()
            .debug_selector(|| "transfer-strip".into())
            .w_full()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .id("transfer-strip-head")
                    .debug_selector(|| "transfer-strip-head".into())
                    .w_full()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .items_center()
                    .cursor_pointer()
                    .child(div().text_xs().child(headline))
                    .child(div().flex_1())
                    .when(live.is_empty(), |row| {
                        row.child(
                            Button::new("transfer-clear")
                                .debug_selector(|| "transfer-clear".into())
                                .ghost()
                                .xsmall()
                                .label("Clear")
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    let _ = clear_owner.update(cx, |this, cx| {
                                        this.clear_transfers(key, workspace, cx)
                                    });
                                }),
                        )
                    })
                    .on_click(move |_, _, cx| {
                        let _ = toggle_owner.update(cx, |this, cx| {
                            this.transfer_strip_open = !this.transfer_strip_open;
                            cx.notify();
                        });
                    }),
            );
        if open {
            for transfer in transfers {
                strip = strip.child(self.render_transfer_row(key, transfer, cx));
            }
        }
        Some(strip.into_any_element())
    }

    fn render_transfer_row(
        &self,
        key: ConnectionKey,
        transfer: &Transfer,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let (done, total) = *transfer
            .progress
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let arrow = match transfer.direction {
            Direction::Download { .. } => "↓",
            Direction::Upload { .. } => "↑",
        };
        let id = transfer.id;
        let owner = cx.weak_entity();
        let status = match &transfer.state {
            TransferState::Queued => Some("waiting".to_owned()),
            TransferState::Running => None,
            TransferState::Done => Some(match transfer.direction {
                Direction::Download { .. } => "downloaded".to_owned(),
                Direction::Upload { .. } => "uploaded".to_owned(),
            }),
            TransferState::Failed(_) => Some("failed".to_owned()),
            TransferState::Cancelled => Some("cancelled".to_owned()),
        };
        let fraction = total
            .filter(|total| *total > 0)
            .map_or(0.0, |total| done as f32 / total as f32);
        h_flex()
            .w_full()
            .gap_2()
            .px_2()
            .py_0p5()
            .items_center()
            .child(div().text_xs().child(arrow))
            .child(
                div()
                    .text_xs()
                    .truncate()
                    .max_w(rems(9.))
                    .child(transfer.name()),
            )
            .child(match status {
                Some(status) => div()
                    .flex_1()
                    .text_xs()
                    .text_color(muted)
                    .child(status)
                    .into_any_element(),
                None => div()
                    .flex_1()
                    .h(px(3.))
                    .rounded(px(2.))
                    .bg(cx.theme().border)
                    .child(
                        div()
                            .h_full()
                            .w(relative(fraction))
                            .rounded(px(2.))
                            .bg(cx.theme().primary),
                    )
                    .into_any_element(),
            })
            .when(transfer.live(), |row| {
                row.child(
                    Button::new(("transfer-cancel", id))
                        .debug_selector(move || format!("transfer-cancel-{id}"))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .tooltip("Cancel")
                        .on_click(move |_, _, cx| {
                            let _ = owner.update(cx, |this, cx| this.cancel_transfer(key, id, cx));
                        }),
                )
            })
            .into_any_element()
    }
}

/// One transfer on a background thread, holding the Device's Stream connection.
fn run_transfer(
    link: &Arc<StreamLink>,
    file: StreamFile,
    direction: &Direction,
    progress: &Arc<Mutex<(u64, Option<u64>)>>,
    cancel: &TransferCancel,
    events: &async_channel::Sender<()>,
) -> Result<(), condr_client::StreamFailure> {
    link.retain()
        .map_err(|detail| condr_client::StreamFailure {
            error: StreamError::Other,
            detail,
        })?;
    let _held = super::ports::Retained::new(Arc::clone(link));
    let report = |done: u64, total: Option<u64>| {
        *progress.lock().unwrap_or_else(PoisonError::into_inner) = (done, total);
        let _ = events.try_send(());
    };
    match direction {
        Direction::Download { destination } => {
            condr_client::download(link.as_ref(), file, destination, true, cancel, report)
        }
        Direction::Upload { source, replace } => {
            condr_client::upload(link.as_ref(), source, file, *replace, cancel, report)
        }
    }
    .map(drop)
}

/// The Stream link a Device's forwards and transfers share.
pub(super) fn new_link(
    device: &str,
    endpoint: &Endpoint,
    events: &async_channel::Sender<()>,
) -> Arc<StreamLink> {
    let endpoint = endpoint.for_streams();
    let changed = events.clone();
    Arc::new(StreamLink::new(
        device,
        move || StreamConnection::connect(&endpoint, "condr-gui").map(|(connection, _)| connection),
        move || {
            let _ = changed.try_send(());
        },
    ))
}

// For tests and the offscreen screenshots, which build on macOS alone.
#[cfg(any(test, all(feature = "test-support", target_os = "macos")))]
impl Condr {
    /// A transfer drawn in a given state without running it: the offscreen screenshots
    /// cannot catch one mid-flight, since their dispatcher runs background work to the end.
    #[cfg(all(feature = "test-support", target_os = "macos"))]
    pub(super) fn show_transfer(
        &mut self,
        key: ConnectionKey,
        workspace: WorkspaceId,
        name: &str,
        upload: bool,
        progress: (u64, Option<u64>),
        state: TransferState,
    ) {
        self.next_transfer_id += 1;
        let id = self.next_transfer_id;
        let path = RelativePathBuf::from(name);
        let direction = if upload {
            Direction::Upload {
                source: PathBuf::from(name),
                replace: false,
            }
        } else {
            Direction::Download {
                destination: PathBuf::from(name),
            }
        };
        if let Some(connection) = self.connection_mut(key) {
            connection.transfers.push(Transfer {
                id,
                workspace,
                path,
                direction,
                progress: Arc::new(Mutex::new(progress)),
                cancel: TransferCancel::default(),
                state,
            });
        }
    }

    /// The states of a Device's transfers, oldest first, for tests.
    pub(super) fn transfer_states(&self, key: ConnectionKey) -> Vec<TransferState> {
        self.connection(key).map_or_else(Vec::new, |connection| {
            connection
                .transfers
                .iter()
                .map(|transfer| transfer.state.clone())
                .collect()
        })
    }
}
