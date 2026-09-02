use super::sidebar::DragPreview;
use super::*;

#[derive(Clone)]
struct DraggedTab {
    key: ConnectionKey,
    workspace_id: WorkspaceId,
    tab_id: TabId,
    name: SharedString,
}

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
        let tab_buttons = workspace.tabs().iter().enumerate().map(|(tab_index, tab)| {
            let tab_id = tab.id();
            let tab_name = tab.name().to_owned();
            let activate_owner = cx.weak_entity();
            let menu_owner = cx.weak_entity();
            h_flex()
                .id(("tab-menu", tab_id.as_u64()))
                .when(can_mutate, |this| {
                    let drop_owner = cx.weak_entity();
                    this.on_drag(
                        DraggedTab {
                            key,
                            workspace_id,
                            tab_id,
                            name: tab.name().to_owned().into(),
                        },
                        move |drag, _, _, cx| {
                            cx.stop_propagation();
                            let name = drag.name.clone();
                            cx.new(|_| DragPreview { icon: None, name })
                        },
                    )
                    .drag_over::<DraggedTab>(|style, _, _, cx| {
                        style.bg(cx.theme().accent.opacity(0.8))
                    })
                    .on_drop(move |dragged: &DraggedTab, _, cx| {
                        if dragged.key != key
                            || dragged.workspace_id != workspace_id
                            || dragged.tab_id == tab_id
                        {
                            return;
                        }
                        let dragged_tab = dragged.tab_id;
                        let _ = drop_owner.update(cx, |this, _| {
                            this.send_layout_to(
                                key,
                                LayoutCommand::MoveTab {
                                    tab_id: dragged_tab,
                                    target_index: tab_index as u32,
                                },
                            );
                        });
                    })
                })
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

#[derive(Debug, Eq, PartialEq)]
struct ClampedUtf16Range {
    bytes: Range<usize>,
    utf16: Range<usize>,
}

fn clamp_utf16_boundary(text: &str, offset: usize, round_up: bool) -> (usize, usize) {
    let mut utf16_offset = 0;
    for (byte_offset, character) in text.char_indices() {
        if offset <= utf16_offset {
            return (byte_offset, utf16_offset);
        }
        let next_utf16_offset = utf16_offset + character.len_utf16();
        if offset < next_utf16_offset {
            return if round_up {
                (byte_offset + character.len_utf8(), next_utf16_offset)
            } else {
                (byte_offset, utf16_offset)
            };
        }
        utf16_offset = next_utf16_offset;
    }
    (text.len(), utf16_offset)
}

fn clamp_utf16_range(text: &str, range: Range<usize>) -> ClampedUtf16Range {
    let (start_byte, start_utf16) = clamp_utf16_boundary(text, range.start, false);
    let (mut end_byte, mut end_utf16) = clamp_utf16_boundary(text, range.end, true);
    if end_byte < start_byte {
        end_byte = start_byte;
        end_utf16 = start_utf16;
    }
    ClampedUtf16Range {
        bytes: start_byte..end_byte,
        utf16: start_utf16..end_utf16,
    }
}

fn terminal_ime_cell(cursor: TerminalCursor, size: TerminalSize, offset: usize) -> (usize, usize) {
    let columns = usize::from(size.columns.max(1));
    let cell = usize::from(cursor.row)
        .saturating_mul(columns)
        .saturating_add(usize::from(cursor.column))
        .saturating_add(offset);
    (cell / columns, cell % columns)
}

impl TerminalComposition {
    pub(super) fn belongs_to(&self, connection_key: ConnectionKey, pane_id: PaneId) -> bool {
        self.connection_key == connection_key && self.pane_id == pane_id
    }

    pub(super) fn belongs_to_target(&self, target: Option<(ConnectionKey, PaneId)>) -> bool {
        target == Some((self.connection_key, self.pane_id))
    }

    fn replace(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
    ) {
        let text_len = self.text.encode_utf16().count();
        let replaced = clamp_utf16_range(&self.text, range.unwrap_or(0..text_len));
        let replacement_start = replaced.utf16.start;
        self.text.replace_range(replaced.bytes, new_text);

        let inserted_len = new_text.encode_utf16().count();
        let selected = clamp_utf16_range(
            new_text,
            new_selected_range.unwrap_or(inserted_len..inserted_len),
        )
        .utf16;
        let text_len = self.text.encode_utf16().count();
        self.selected_range = replacement_start
            .saturating_add(selected.start)
            .min(text_len)
            ..replacement_start.saturating_add(selected.end).min(text_len);
    }
}

fn commit_terminal_composition(
    mut composition: Option<TerminalComposition>,
    target: Option<(ConnectionKey, PaneId)>,
    range: Option<Range<usize>>,
    text: &str,
) -> Option<((ConnectionKey, PaneId), String)> {
    match composition.as_mut() {
        Some(composition) if composition.belongs_to_target(target) => {
            composition.replace(range, text, None);
            Some((
                (composition.connection_key, composition.pane_id),
                std::mem::take(&mut composition.text),
            ))
        }
        Some(_) => None,
        None => target.map(|target| (target, text.to_owned())),
    }
}

fn update_terminal_composition(
    composition: Option<TerminalComposition>,
    target: Option<(ConnectionKey, PaneId)>,
    range: Option<Range<usize>>,
    new_text: &str,
    new_selected_range: Option<Range<usize>>,
) -> Option<TerminalComposition> {
    let (connection_key, pane_id) = target?;
    let mut composition = composition
        .filter(|composition| composition.belongs_to_target(target))
        .unwrap_or(TerminalComposition {
            connection_key,
            pane_id,
            text: String::new(),
            selected_range: 0..0,
        });
    composition.replace(range, new_text, new_selected_range);
    (!composition.text.is_empty()).then_some(composition)
}

impl Condr {
    fn active_terminal_composition(&self) -> Option<&TerminalComposition> {
        self.terminal_composition
            .as_ref()
            .filter(|composition| composition.belongs_to_target(self.target_pane))
    }

    pub(super) fn marked_text_for(
        &self,
        connection_key: ConnectionKey,
        pane_id: PaneId,
    ) -> Option<String> {
        self.active_terminal_composition()
            .filter(|composition| composition.belongs_to(connection_key, pane_id))
            .map(|composition| composition.text.clone())
    }
}

impl EntityInputHandler for Condr {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let Some(composition) = self.active_terminal_composition() else {
            adjusted_range.replace(0..0);
            return Some(String::new());
        };
        let range = clamp_utf16_range(&composition.text, range);
        adjusted_range.replace(range.utf16);
        Some(composition.text[range.bytes].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self
                .active_terminal_composition()
                .map_or(0..0, |composition| composition.selected_range.clone()),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.active_terminal_composition()
            .map(|composition| 0..composition.text.encode_utf16().count())
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal_composition.take().is_some() {
            window.invalidate_character_coordinates();
            cx.notify();
        }
    }

    fn paste(&mut self, item: ClipboardItem, _: &mut Window, cx: &mut Context<Self>) {
        if let (Some(text), Some((key, pane_id))) = (item.text(), self.target_pane) {
            self.clear_selection(cx);
            self.terminal_command(key, pane_id, TerminalCommand::Paste(text));
        }
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let composition = self.terminal_composition.take();
        let had_composition = composition.is_some();
        let committed = commit_terminal_composition(composition, self.target_pane, range, text);
        if had_composition {
            window.invalidate_character_coordinates();
            cx.notify();
        }
        if let Some(((key, pane_id), text)) = committed
            && !text.is_empty()
        {
            self.clear_selection(cx);
            self.terminal_command(key, pane_id, TerminalCommand::Text(text));
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let composition = self.terminal_composition.take();
        let previous_composition = composition.clone();
        let composition = update_terminal_composition(
            composition,
            self.target_pane,
            range,
            new_text,
            new_selected_range,
        );
        let changed = previous_composition != composition;
        self.terminal_composition = composition;
        if changed {
            window.invalidate_character_coordinates();
            cx.notify();
        }
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (key, offset) = self.active_terminal_composition().map_or_else(
            || self.target_pane.map(|key| (key, 0)),
            |composition| {
                let (_, offset) = clamp_utf16_boundary(&composition.text, range.start, false);
                Some(((composition.connection_key, composition.pane_id), offset))
            },
        )?;
        let geometry = self.terminal_geometry.get(&key)?;
        let terminal = &self.terminal(key.0, key.1)?.view;
        let cursor = terminal.cursor?;
        let (row, column) = terminal_ime_cell(cursor, terminal.size, offset);
        Some(Bounds::new(
            point(
                geometry.bounds.left() + geometry.cell_size.width * column,
                geometry.bounds.top() + geometry.cell_size.height * row,
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
            .on_action(cx.listener(Self::action_terminal_tab))
            .on_action(cx.listener(Self::action_terminal_back_tab))
            .on_action(cx.listener(Self::action_add_server))
            .on_action(cx.listener(Self::action_reconnect))
            .on_action(cx.listener(Self::action_new_workspace))
            .on_action(cx.listener(Self::action_new_tab))
            .on_action(cx.listener(Self::action_open_settings))
            .on_action(cx.listener(Self::action_rename_workspace))
            .on_action(cx.listener(Self::action_rename_tab))
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
                // The sidebar keeps an absolute width, the way Zed sizes its docks: a
                // resizable group would rescale it with the window on every resize.
                // Dragging the handle on its right edge is the only thing that moves it.
                h_flex()
                    .size_full()
                    .on_drag_move(cx.listener(
                        |this, event: &DragMoveEvent<DraggedSidebar>, _, cx| {
                            let width = event.event.position.x - event.bounds.origin.x;
                            this.sidebar_width =
                                width.max(MIN_SIDEBAR_WIDTH).min(MAX_SIDEBAR_WIDTH).round();
                            cx.notify();
                        },
                    ))
                    .child(
                        div()
                            .debug_selector(|| "condr-sidebar".into())
                            .relative()
                            .w(self.sidebar_width)
                            .flex_none()
                            .h_full()
                            .child(self.render_sidebar(cx))
                            .child(
                                div()
                                    .id("condr-sidebar-resize")
                                    .debug_selector(|| "condr-sidebar-resize".into())
                                    .absolute()
                                    .top_0()
                                    .bottom_0()
                                    .right(-SIDEBAR_RESIZE_HANDLE_WIDTH / 2.)
                                    .w(SIDEBAR_RESIZE_HANDLE_WIDTH)
                                    .cursor_col_resize()
                                    .on_drag(DraggedSidebar, |_, _, _, cx| cx.new(|_| EmptyView)),
                            ),
                    )
                    .child(workspace),
            )
            .children(dialog_layer)
    }
}

#[cfg(test)]
mod ime_tests {
    use condr_core::{
        PaneId, Session, SplitDirection, TerminalCursor, TerminalCursorShape, TerminalSize,
    };

    use super::{
        TerminalComposition, commit_terminal_composition, terminal_ime_cell,
        update_terminal_composition,
    };

    fn pane_ids() -> (PaneId, PaneId) {
        let mut session = Session::new();
        session.create_workspace(std::env::temp_dir()).unwrap();
        let first = session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        let second = session
            .split_pane(first, SplitDirection::Horizontal, 0.5)
            .unwrap();
        (first, second)
    }

    fn composition(text: &str) -> TerminalComposition {
        let (pane_id, _) = pane_ids();
        TerminalComposition {
            connection_key: 7,
            pane_id,
            text: text.to_owned(),
            selected_range: 0..0,
        }
    }

    #[test]
    fn composition_owner_does_not_follow_the_selected_pane() {
        let (owner, other) = pane_ids();
        let composition = TerminalComposition {
            connection_key: 7,
            pane_id: owner,
            text: "draft".into(),
            selected_range: 5..5,
        };

        assert!(composition.belongs_to(7, owner));
        assert!(!composition.belongs_to(7, other));
        assert!(!composition.belongs_to(8, owner));
    }

    #[test]
    fn commit_after_switching_panes_is_discarded() {
        let (owner, other) = pane_ids();
        let composition = TerminalComposition {
            connection_key: 7,
            pane_id: owner,
            text: "draft".into(),
            selected_range: 5..5,
        };

        assert_eq!(
            commit_terminal_composition(Some(composition), Some((7, other)), None, "committed"),
            None
        );
    }

    #[test]
    fn new_preedit_after_switching_panes_belongs_to_the_new_pane() {
        let (owner, other) = pane_ids();
        let composition = TerminalComposition {
            connection_key: 7,
            pane_id: owner,
            text: "old".into(),
            selected_range: 3..3,
        };

        let composition = update_terminal_composition(
            Some(composition),
            Some((7, other)),
            None,
            "new",
            Some(3..3),
        )
        .unwrap();

        assert!(composition.belongs_to(7, other));
        assert_eq!(composition.text, "new");
        assert_eq!(composition.selected_range, 3..3);
    }

    #[test]
    fn composition_replacement_uses_utf16_start_and_end() {
        let mut composition = composition("a😀b");

        composition.replace(Some(1..3), "界", Some(1..1));

        assert_eq!(composition.text, "a界b");
        assert_eq!(composition.selected_range, 2..2);
    }

    #[test]
    fn composition_replacement_clamps_inside_surrogate_pairs() {
        let mut composition = composition("a😀b");

        composition.replace(Some(2..2), "x", None);

        assert_eq!(composition.text, "axb");
        assert_eq!(composition.selected_range, 2..2);
    }

    #[test]
    fn candidate_cell_uses_the_requested_utf16_start() {
        let cursor = TerminalCursor {
            row: 0,
            column: 3,
            shape: TerminalCursorShape::Block,
        };

        assert_eq!(
            terminal_ime_cell(cursor, TerminalSize::new(3, 5), 0),
            (0, 3)
        );
        assert_eq!(
            terminal_ime_cell(cursor, TerminalSize::new(3, 5), 3),
            (1, 1)
        );
    }
}
