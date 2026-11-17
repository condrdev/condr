//! The interaction area's form, drawn: a Task's fields on the left and Start on the
//! right once it is saved (ADR 0068).

use super::*;
use gpui_kit::component::input::Textarea;
use gpui_kit::component::radio::Radio;
use rust_i18n::t;
use std::borrow::Cow;

impl Condr {
    /// The interaction area for the form: a header, the form's fields on the left, and
    /// on the right Start for a saved Task (ADR 0068).
    pub(in crate::app) fn render_task_form_area(
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

    pub(super) fn render_task_fields(
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

    pub(super) fn render_task_start(
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
