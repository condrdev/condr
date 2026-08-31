use super::*;

impl Condr {
    pub(super) fn render_workspace(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(connection) = self.active_connection() else {
            return div().size_full().into_any_element();
        };
        let error = self.app_error.clone().or_else(|| connection.error.clone());
        let can_mutate = connection.can_mutate()
            && self
                .pending_workspace_selection_for(connection.key)
                .is_none()
            && !self.has_pending_projection_for(connection.key);
        let Ok(session) = Session::restore(connection.snapshot.clone()) else {
            return div()
                .size_full()
                .child("Invalid Session state")
                .into_any_element();
        };
        let key = connection.key;
        let Some(workspace_id) = self.presented_workspace_id(key, &session) else {
            let new_owner = cx.weak_entity();
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .when_some(error, |view, error| {
                    view.child(div().text_sm().text_color(cx.theme().danger).child(error))
                })
                .child(
                    Button::new("new-terminal-workspace")
                        .debug_selector(|| "new-terminal-workspace".into())
                        .primary()
                        .icon(IconName::SquareTerminal)
                        .label("New Workspace…")
                        .disabled(!can_mutate)
                        .on_click(move |_, window, cx| {
                            let _ = new_owner.update(cx, |this, cx| {
                                this.choose_workspace_directory_on(key, window, cx)
                            });
                        }),
                )
                .into_any_element();
        };
        let workspace = session
            .workspace(workspace_id)
            .expect("presented Workspace belongs to the restored Session");

        let active_tab = self
            .presented_tab_id(key, &session, workspace_id)
            .expect("presented Workspace has an active Tab");
        let surface_key = DockSurfaceKey {
            connection_key: key,
            tab_id: active_tab,
        };
        let dock_area = (self.active_dock_surface == Some(surface_key))
            .then(|| self.dock_surfaces.get(&surface_key))
            .flatten()
            .map(|surface| surface.area.clone());
        let closes_workspace = workspace.tabs().len() == 1;
        let tab_count = workspace.tabs().len();
        let tab_buttons = workspace.tabs().iter().enumerate().map(|(tab_index, tab)| {
            let tab_id = tab.id();
            let tab_name = tab.name().to_owned();
            let activate_owner = cx.weak_entity();
            let menu_owner = cx.weak_entity();
            h_flex()
                .id(("tab-menu", tab_id.as_u64()))
                .child(
                    Button::new(("tab", tab_id.as_u64()))
                        .debug_selector(move || format!("tab-{}", tab_id.as_u64()))
                        .ghost()
                        .small()
                        .selected(tab_id == active_tab)
                        .label(tab.name().to_owned())
                        .disabled(!can_mutate)
                        .on_click(move |_, window, cx| {
                            let _ = activate_owner.update(cx, |this, cx| {
                                this.activate_tab_on(key, tab_id, window, cx)
                            });
                        }),
                )
                .context_menu(move |menu, _, _| {
                    let rename_owner = menu_owner.clone();
                    let move_left_owner = menu_owner.clone();
                    let move_right_owner = menu_owner.clone();
                    let close_owner = menu_owner.clone();
                    let rename_name = tab_name.clone();
                    menu.item(
                        PopupMenuItem::new("Rename Tab…")
                            .disabled(!can_mutate)
                            .on_click(move |_, window, cx| {
                                let name = rename_name.clone();
                                let _ = rename_owner.update(cx, |this, cx| {
                                    this.prompt_rename_tab_on(key, tab_id, name, window, cx)
                                });
                            }),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new("Move Left")
                            .disabled(!can_mutate || tab_index == 0)
                            .on_click(move |_, _, cx| {
                                let _ = move_left_owner.update(cx, |this, _| {
                                    this.send_layout_to(
                                        key,
                                        LayoutCommand::MoveTab {
                                            tab_id,
                                            target_index: tab_index.saturating_sub(1) as u32,
                                        },
                                    );
                                });
                            }),
                    )
                    .item(
                        PopupMenuItem::new("Move Right")
                            .disabled(!can_mutate || tab_index + 1 >= tab_count)
                            .on_click(move |_, _, cx| {
                                let _ = move_right_owner.update(cx, |this, _| {
                                    this.send_layout_to(
                                        key,
                                        LayoutCommand::MoveTab {
                                            tab_id,
                                            target_index: (tab_index + 1) as u32,
                                        },
                                    );
                                });
                            }),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new("Close Tab")
                            .disabled(!can_mutate)
                            .on_click(move |_, window, cx| {
                                let _ = close_owner.update(cx, |this, cx| {
                                    this.close_tab_id(key, tab_id, closes_workspace, window, cx)
                                });
                            }),
                    )
                })
        });
        let new_tab_owner = cx.weak_entity();

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(WORKSPACE_TAB_BAR_HEIGHT)
                    .flex_shrink_0()
                    .gap_1()
                    .px_2()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .children(tab_buttons)
                    .child(
                        Button::new("new-tab")
                            .debug_selector(|| "new-tab".into())
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .tooltip("New Tab")
                            .disabled(!can_mutate)
                            .on_click(move |_, window, cx| {
                                let _ = new_tab_owner.update(cx, |this, cx| {
                                    this.new_tab_on(key, workspace_id, window, cx)
                                });
                            }),
                    )
                    .when_some(error, |row, error| {
                        row.child(
                            div()
                                .ml_auto()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().danger)
                                .child(error),
                        )
                    }),
            )
            .child(
                div()
                    .min_h_0()
                    .flex_1()
                    .when_some(dock_area, |view, dock_area| view.child(dock_area)),
            )
            .into_any_element()
    }
}

impl Focusable for Condr {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for Condr {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        adjusted_range.replace(
            0..self
                .marked_text
                .as_ref()
                .map_or(0, |text| text.encode_utf16().count()),
        );
        Some(self.marked_text.clone().unwrap_or_default())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_text
            .as_ref()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_text = None;
        cx.notify();
    }

    fn paste(&mut self, item: ClipboardItem, _: &mut Window, cx: &mut Context<Self>) {
        if let (Some(text), Some((key, pane_id))) = (item.text(), self.target_pane) {
            self.clear_selection(cx);
            self.terminal_command(key, pane_id, TerminalCommand::Paste(text));
        }
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = None;
        if !text.is_empty()
            && let Some((key, pane_id)) = self.target_pane
        {
            self.clear_selection(cx);
            self.terminal_command(key, pane_id, TerminalCommand::Text(text.into()));
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        new_text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = (!new_text.is_empty()).then(|| new_text.to_owned());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let key = self.target_pane?;
        let geometry = self.terminal_geometry.get(&key)?;
        let cursor = self.terminal(key.0, key.1)?.view.cursor?;
        Some(Bounds::new(
            point(
                geometry.bounds.left() + geometry.cell_size.width * f32::from(cursor.column),
                geometry.bounds.top() + geometry.cell_size.height * f32::from(cursor.row),
            ),
            geometry.cell_size,
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }
}

impl Render for Condr {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dialog_layer = Root::render_dialog_layer(window, cx);
        let workspace_owner = cx.weak_entity();
        let workspace = div()
            .size_full()
            .on_prepaint(move |bounds, _, cx| {
                let _ = workspace_owner.update(cx, |this, _| {
                    this.workspace_size = bounds.size;
                });
            })
            .child(self.render_workspace(cx))
            .into_any_element();
        div()
            .key_context("Condr")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::action_add_server))
            .on_action(cx.listener(Self::action_reconnect))
            .on_action(cx.listener(Self::action_new_workspace))
            .on_action(cx.listener(Self::action_new_tab))
            .on_action(cx.listener(Self::action_rename_workspace))
            .on_action(cx.listener(Self::action_rename_tab))
            .on_action(cx.listener(Self::action_move_workspace_up))
            .on_action(cx.listener(Self::action_move_workspace_down))
            .on_action(cx.listener(Self::action_move_tab_left))
            .on_action(cx.listener(Self::action_move_tab_right))
            .on_action(cx.listener(Self::action_close_pane))
            .on_action(cx.listener(Self::action_close_tab))
            .on_action(cx.listener(Self::action_close_workspace))
            .on_action(cx.listener(Self::action_next_tab))
            .on_action(cx.listener(Self::action_previous_tab))
            .on_action(cx.listener(Self::action_split_right))
            .on_action(cx.listener(Self::action_split_down))
            .on_action(cx.listener(Self::action_focus_left))
            .on_action(cx.listener(Self::action_focus_right))
            .on_action(cx.listener(Self::action_focus_up))
            .on_action(cx.listener(Self::action_focus_down))
            .on_action(cx.listener(Self::action_resize_left))
            .on_action(cx.listener(Self::action_resize_right))
            .on_action(cx.listener(Self::action_resize_up))
            .on_action(cx.listener(Self::action_resize_down))
            .on_action(cx.listener(Self::action_swap_left))
            .on_action(cx.listener(Self::action_swap_right))
            .on_action(cx.listener(Self::action_swap_up))
            .on_action(cx.listener(Self::action_swap_down))
            .on_action(cx.listener(Self::action_toggle_zoom))
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_resizable("condr-shell")
                    .child(
                        resizable_panel()
                            .size(INITIAL_SIDEBAR_WIDTH)
                            .size_range(px(180.)..px(360.))
                            .flex_none()
                            .child(
                                div()
                                    .debug_selector(|| "condr-sidebar".into())
                                    .size_full()
                                    .child(self.render_sidebar(cx)),
                            ),
                    )
                    .child(workspace),
            )
            .children(dialog_layer)
    }
}
