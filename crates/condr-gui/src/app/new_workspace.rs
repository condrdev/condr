//! New Workspace's chooser, and the Clone Repository and New Directory forms it turns into
//! in place (ADR 0059).

use super::dialogs::field_error;
use super::sidebar::CondrIconName;
use super::*;
use gpui_kit::component::kbd::Kbd;
use std::collections::HashSet;

actions!(new_workspace, [ChoosePrevious, ChooseNext]);

/// The key context around the chooser's rows, so Up and Down move the highlight.
const CONTEXT: &str = "NewWorkspaceChooser";

pub(super) fn key_bindings() -> [KeyBinding; 2] {
    let context = Some(CONTEXT);
    [
        KeyBinding::new("up", ChoosePrevious, context),
        KeyBinding::new("down", ChooseNext, context),
    ]
}

/// The ways a New Workspace starts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NewWorkspaceWay {
    OpenProject,
    CloneRepository,
    NewDirectory,
}

impl NewWorkspaceWay {
    const ALL: [Self; 3] = [Self::OpenProject, Self::CloneRepository, Self::NewDirectory];

    pub(super) fn id(self) -> &'static str {
        match self {
            Self::OpenProject => "open-project",
            Self::CloneRepository => "clone-repository",
            Self::NewDirectory => "new-directory",
        }
    }

    pub(super) fn title(self) -> &'static str {
        match self {
            Self::OpenProject => "Open Project",
            Self::CloneRepository => "Clone Repository",
            Self::NewDirectory => "New Directory",
        }
    }

    fn description(self, device: &str) -> String {
        match self {
            Self::OpenProject => format!("Choose a directory on {device}"),
            Self::CloneRepository => {
                format!("Clone a Git repository into a new directory on {device}")
            }
            Self::NewDirectory => format!("Create an empty directory on {device}"),
        }
    }

    pub(super) fn icon(self) -> Icon {
        match self {
            Self::OpenProject => Icon::new(IconName::FolderOpen),
            Self::CloneRepository => Icon::new(CondrIconName::GitBranch),
            Self::NewDirectory => Icon::new(CondrIconName::FolderPlus),
        }
    }
}

/// What the form asked the Server about its parent directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Lookup {
    /// The presented Workspace's Root Directory, whose parent the Server names.
    DefaultParent,
    /// The home directory, when no Root Directory gives a parent.
    Home,
    /// The chosen parent, for the names already in it.
    Names,
}

/// The open New Workspace dialog. The next one replaces it, so a closed one is harmless.
pub(super) struct NewWorkspaceDialog {
    key: ConnectionKey,
    device: String,
    /// The form shown, or `None` for the chooser.
    pub(super) form: Option<NewWorkspaceWay>,
    highlighted: usize,
    focus: FocusHandle,
    url: Entity<InputState>,
    pub(super) name: Entity<InputState>,
    /// The name the URL last filled in; a field that differs is the person's own.
    derived_name: String,
    pub(super) parent: Option<PathBuf>,
    /// The directories already in `parent`, once the Server listed them.
    pub(super) taken: Option<HashSet<String>>,
    lookup: Option<(u64, Lookup)>,
    parent_error: Option<String>,
    url_error: Option<String>,
    name_error: Option<String>,
}

impl Condr {
    /// Opens the chooser on `key`'s Device, or straight into one of its forms.
    pub(super) fn open_new_workspace(
        &mut self,
        key: ConnectionKey,
        way: Option<NewWorkspaceWay>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if way == Some(NewWorkspaceWay::OpenProject) {
            return self.choose_workspace_directory_on(key, window, cx);
        }
        let Some(device) = self
            .connection(key)
            .map(|connection| connection.label.clone())
        else {
            return;
        };
        // A Managed Worktree's parent would be its `<repo>.worktrees/`; its repository's
        // parent is where a person keeps repositories.
        let root = self
            .presented()
            .filter(|(presented, ..)| *presented == key)
            .and_then(|(_, session, workspace_id, _)| session.workspace(workspace_id))
            .map(|workspace| {
                workspace
                    .worktree()
                    .map_or(workspace.root_directory(), |association| {
                        association.parent_root_directory()
                    })
                    .to_path_buf()
            });
        let url = cx.new(|cx| {
            InputState::new(window, cx).placeholder("https://github.com/owner/repository.git")
        });
        let name = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe_in(&url, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change) {
                this.new_workspace_url_changed(window, cx);
            }
        })
        .detach();
        cx.subscribe(&name, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                if let Some(dialog) = this.new_workspace.as_mut() {
                    dialog.name_error = None;
                }
                cx.notify();
            }
        })
        .detach();
        self.new_workspace = Some(NewWorkspaceDialog {
            key,
            device,
            form: way,
            highlighted: 0,
            focus: cx.focus_handle(),
            url,
            name,
            derived_name: String::new(),
            parent: None,
            taken: None,
            lookup: None,
            parent_error: None,
            url_error: None,
            name_error: None,
        });
        match root {
            Some(root) => self.request_new_workspace_lookup(root, Lookup::DefaultParent),
            None => self.request_new_workspace_lookup(PathBuf::new(), Lookup::Home),
        }
        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            open_new_workspace_dialog(owner.clone(), window, cx);
            let _ = owner.update(cx, |this, cx| this.focus_new_workspace(window, cx));
        });
    }

    fn focus_new_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.new_workspace.as_ref() else {
            return;
        };
        match dialog.form {
            None => dialog.focus.focus(window, cx),
            Some(NewWorkspaceWay::CloneRepository) => {
                dialog.url.update(cx, |input, cx| input.focus(window, cx))
            }
            Some(_) => dialog.name.update(cx, |input, cx| input.focus(window, cx)),
        }
    }

    /// The name follows the URL until the person writes one of their own.
    fn new_workspace_url_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.new_workspace.as_mut() else {
            return;
        };
        dialog.url_error = None;
        if dialog.name.read(cx).value() != dialog.derived_name {
            return cx.notify();
        }
        let url = dialog.url.read(cx).value().to_string();
        let derived = condr_core::clone_directory_name(&url).unwrap_or_default();
        dialog.derived_name = derived.clone();
        dialog
            .name
            .update(cx, |input, cx| input.set_value(derived, window, cx));
        cx.notify();
    }

    fn request_new_workspace_lookup(&mut self, path: PathBuf, lookup: Lookup) {
        let Some(key) = self.new_workspace.as_ref().map(|dialog| dialog.key) else {
            return;
        };
        let Some(connection) = self
            .connection_mut(key)
            .filter(|connection| connection.status == ConnectionStatus::Connected)
        else {
            return;
        };
        let request_id = connection.next_request_id();
        if connection.send(ClientMessage::BrowseDirectory { request_id, path })
            && let Some(dialog) = self.new_workspace.as_mut()
        {
            dialog.lookup = Some((request_id, lookup));
        }
    }

    /// Takes the Server's answer to the form's own lookup, and says whether it was one.
    pub(super) fn apply_new_workspace_lookup(
        &mut self,
        key: ConnectionKey,
        request_id: u64,
        result: &Result<BrowsedDirectory, String>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(dialog) = self
            .new_workspace
            .as_mut()
            .filter(|dialog| dialog.key == key)
        else {
            return false;
        };
        let Some((_, lookup)) = dialog.lookup.take_if(|(id, _)| *id == request_id) else {
            return false;
        };
        let listed = |browsed: &BrowsedDirectory| {
            browsed
                .listing
                .entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect::<HashSet<_>>()
        };
        match (lookup, result) {
            (Lookup::DefaultParent, Ok(browsed)) => match browsed.parent.clone() {
                Some(parent) => {
                    dialog.parent = Some(parent.clone());
                    self.request_new_workspace_lookup(parent, Lookup::Names);
                }
                // A root directory is its own parent.
                None => {
                    dialog.parent = Some(browsed.path.clone());
                    dialog.taken = Some(listed(browsed));
                }
            },
            (Lookup::DefaultParent, Err(_)) => {
                self.request_new_workspace_lookup(PathBuf::new(), Lookup::Home);
            }
            (Lookup::Home | Lookup::Names, Ok(browsed)) => {
                dialog.parent = Some(browsed.path.clone());
                dialog.taken = Some(listed(browsed));
                dialog.parent_error = None;
            }
            (Lookup::Home, Err(reason)) => dialog.parent_error = Some(reason.clone()),
            // The Server refuses a name it finds anyway.
            (Lookup::Names, Err(_)) => dialog.taken = Some(HashSet::new()),
        }
        cx.notify();
        true
    }

    fn change_new_workspace_parent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((key, device)) = self
            .new_workspace
            .as_ref()
            .map(|dialog| (dialog.key, dialog.device.clone()))
        else {
            return;
        };
        self.prompt_directory_on(
            key,
            format!("Choose Where on {device}"),
            "Choose",
            move |this, parent, _, cx| {
                let Some(dialog) = this
                    .new_workspace
                    .as_mut()
                    .filter(|dialog| dialog.key == key)
                else {
                    return;
                };
                dialog.parent = Some(parent.clone());
                dialog.taken = None;
                dialog.parent_error = None;
                this.request_new_workspace_lookup(parent, Lookup::Names);
                cx.notify();
            },
            window,
            cx,
        );
    }

    fn move_new_workspace_highlight(&mut self, forward: bool, cx: &mut Context<Self>) {
        if let Some(dialog) = self.new_workspace.as_mut() {
            dialog.highlighted = if forward {
                (dialog.highlighted + 1).min(NewWorkspaceWay::ALL.len() - 1)
            } else {
                dialog.highlighted.saturating_sub(1)
            };
            cx.notify();
        }
    }

    /// Enter or the primary button: the chooser opens its highlighted way, a form sends
    /// its request. Returns whether the dialog closes.
    fn confirm_new_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(dialog) = self.new_workspace.as_mut() else {
            return true;
        };
        let key = dialog.key;
        let Some(way) = dialog.form else {
            let way = NewWorkspaceWay::ALL[dialog.highlighted];
            if way == NewWorkspaceWay::OpenProject {
                self.new_workspace = None;
                self.choose_workspace_directory_on(key, window, cx);
                return true;
            }
            dialog.form = Some(way);
            self.focus_new_workspace(window, cx);
            cx.notify();
            return false;
        };
        let clone_url = (way == NewWorkspaceWay::CloneRepository)
            .then(|| {
                condr_core::validate_clone_url(&dialog.url.read(cx).value())
                    .map(str::to_owned)
                    .map_err(|reason| dialog.url_error = Some(reason))
            })
            .transpose();
        let name = dialog.name.read(cx).value().trim().to_owned();
        let named = condr_core::validate_directory_name(&name).and_then(|()| match &dialog.taken {
            Some(taken) if taken.contains(&name) => {
                Err("A directory with this name is already here".to_owned())
            }
            _ => Ok(()),
        });
        if let Err(reason) = &named {
            dialog.name_error = Some(reason.clone());
        }
        if dialog.parent.is_none() {
            dialog.parent_error = Some("Choose where to create it".into());
        }
        let (Ok(clone_url), Ok(()), Some(parent)) = (clone_url, named, dialog.parent.clone())
        else {
            cx.notify();
            return false;
        };
        self.new_workspace = None;
        self.send_presenting_layout_to(
            key,
            LayoutCommand::CreateWorkspaceDirectory {
                parent_directory: parent,
                directory_name: name,
                name: None,
                clone_url,
            },
            window,
            cx,
        );
        true
    }

    /// Esc: a form goes back to the chooser, which closes. Returns whether it closes.
    fn cancel_new_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(dialog) = self.new_workspace.as_mut() else {
            return true;
        };
        if let Some(way) = dialog.form.take() {
            dialog.highlighted = NewWorkspaceWay::ALL
                .iter()
                .position(|candidate| *candidate == way)
                .unwrap_or(0);
            self.focus_new_workspace(window, cx);
            cx.notify();
            return false;
        }
        self.new_workspace = None;
        true
    }
}

fn open_new_workspace_dialog(owner: WeakEntity<Condr>, window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, move |dialog, _, cx| {
        let Some(condr) = owner.upgrade() else {
            return dialog;
        };
        let Some(state) = condr.read(cx).new_workspace.as_ref() else {
            return dialog;
        };
        let title = match state.form {
            None => format!("New Workspace on {}", state.device),
            Some(way) => format!("{} on {}", way.title(), state.device),
        };
        let footer = match state.form {
            None => chooser_footer(cx).into_any_element(),
            Some(way) => form_footer(way).into_any_element(),
        };
        let (content, ok, cancel, close) =
            (owner.clone(), owner.clone(), owner.clone(), owner.clone());
        dialog
            .title(title)
            .content(move |body, _, cx| {
                body.children(
                    content
                        .upgrade()
                        .and_then(|condr| render_new_workspace(&content, &condr, cx)),
                )
            })
            .footer(footer)
            .on_ok(move |_, window, cx| {
                ok.update(cx, |this, cx| this.confirm_new_workspace(window, cx))
                    .unwrap_or(true)
            })
            .on_cancel(move |_, window, cx| {
                cancel
                    .update(cx, |this, cx| this.cancel_new_workspace(window, cx))
                    .unwrap_or(true)
            })
            .on_close(move |_, _, cx| {
                let _ = close.update(cx, |this, _| this.new_workspace = None);
            })
    });
}

fn chooser_footer(cx: &App) -> impl IntoElement {
    let hint = |keys: &'static [&'static str], label: &'static str| {
        h_flex()
            .gap_1()
            .items_center()
            .children(
                keys.iter()
                    .filter_map(|key| Keystroke::parse(key).ok())
                    .map(Kbd::new),
            )
            .child(label)
    };
    h_flex()
        .w_full()
        .gap_4()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(hint(&["up", "down"], "Navigate"))
        .child(hint(&["enter"], "Select"))
        .child(hint(&["escape"], "Close"))
}

fn form_footer(way: NewWorkspaceWay) -> DialogFooter {
    DialogFooter::new()
        .child(
            Button::new("dialog-cancel")
                .debug_selector(|| "dialog-cancel".into())
                .label("Back")
                .on_click(|_, window, cx| window.dispatch_action(Box::new(Cancel), cx)),
        )
        .child(
            Button::new("dialog-primary-action")
                .debug_selector(|| "dialog-primary-action".into())
                .primary()
                .label(match way {
                    NewWorkspaceWay::CloneRepository => "Clone",
                    _ => "Create",
                })
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(Confirm { secondary: false }), cx)
                }),
        )
}

fn render_new_workspace(
    owner: &WeakEntity<Condr>,
    condr: &Entity<Condr>,
    cx: &App,
) -> Option<AnyElement> {
    let state = condr.read(cx).new_workspace.as_ref()?;
    Some(match state.form {
        None => render_chooser(owner, state, cx),
        Some(way) => render_form(owner, state, way, cx),
    })
}

fn render_chooser(owner: &WeakEntity<Condr>, state: &NewWorkspaceDialog, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let rows = NewWorkspaceWay::ALL.iter().enumerate().map(|(index, way)| {
        let way = *way;
        let id = format!("new-workspace-{}", way.id());
        let owner = owner.clone();
        h_flex()
            .id(SharedString::from(id.clone()))
            .debug_selector(move || id.clone())
            .w_full()
            .px_3()
            .py_2()
            .gap_3()
            .items_center()
            .cursor_pointer()
            .rounded(theme.radius)
            .when(index == state.highlighted, |row| row.bg(theme.list_active))
            .hover(|row| row.bg(theme.list_hover))
            .on_click(move |_, window, cx| {
                let _ = owner.update(cx, |this, _| {
                    if let Some(dialog) = this.new_workspace.as_mut() {
                        dialog.highlighted = index;
                    }
                });
                window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
            })
            .child(way.icon().text_color(theme.muted_foreground))
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .child(div().text_sm().child(way.title()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(way.description(&state.device)),
                    ),
            )
    });
    let (previous, next) = (owner.clone(), owner.clone());
    v_flex()
        .id("new-workspace-chooser")
        .debug_selector(|| "new-workspace-chooser".into())
        .key_context(CONTEXT)
        .track_focus(&state.focus)
        .on_action(move |_: &ChoosePrevious, _, cx| {
            let _ = previous.update(cx, |this, cx| this.move_new_workspace_highlight(false, cx));
        })
        .on_action(move |_: &ChooseNext, _, cx| {
            let _ = next.update(cx, |this, cx| this.move_new_workspace_highlight(true, cx));
        })
        .gap_1()
        .children(rows)
        .into_any_element()
}

fn render_form(
    owner: &WeakEntity<Condr>,
    state: &NewWorkspaceDialog,
    way: NewWorkspaceWay,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let label = |text: &'static str| div().text_sm().child(text);
    let name = state.name.read(cx).value().trim().to_owned();
    let name_error = state.name_error.clone().or_else(|| {
        state
            .taken
            .as_ref()
            .filter(|taken| taken.contains(&name))
            .map(|_| "A directory with this name is already here".to_owned())
    });
    let parent = match (&state.parent, &state.parent_error) {
        (Some(parent), _) => parent.display().to_string(),
        (None, Some(_)) => String::new(),
        (None, None) => "Looking it up…".into(),
    };
    let change = owner.clone();
    v_flex()
        .gap_3()
        .when(way == NewWorkspaceWay::CloneRepository, |form| {
            form.child(
                v_flex()
                    .gap_1()
                    .child(label("Repository URL"))
                    .child(
                        div()
                            .debug_selector(|| "new-workspace-url".into())
                            .child(Input::new(&state.url).w_full()),
                    )
                    .children(field_error(state.url_error.clone(), cx)),
            )
        })
        .child(
            v_flex()
                .gap_1()
                .child(label("Location"))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .debug_selector(|| "new-workspace-parent".into())
                                .min_w_0()
                                .flex_1()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                // A path's last directories say the most.
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis_start()
                                .child(parent),
                        )
                        .child(
                            Button::new("new-workspace-change-parent")
                                .debug_selector(|| "new-workspace-change-parent".into())
                                .small()
                                .label("Change…")
                                .on_click(move |_, window, cx| {
                                    let _ = change.update(cx, |this, cx| {
                                        this.change_new_workspace_parent(window, cx)
                                    });
                                }),
                        ),
                )
                .children(field_error(state.parent_error.clone(), cx)),
        )
        .child(
            v_flex()
                .gap_1()
                .child(label("Name"))
                .child(
                    div()
                        .debug_selector(|| "new-workspace-name".into())
                        .child(Input::new(&state.name).w_full()),
                )
                .children(field_error(name_error, cx)),
        )
        .into_any_element()
}
