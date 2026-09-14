use super::sidebar::{CondrIconName, DragPreview, DropTarget, attach_drop_target, drop_index};
use super::*;
use gpui_fps::fps_monitor;

#[derive(Clone)]
struct DraggedTab {
    key: ConnectionKey,
    workspace_id: WorkspaceId,
    tab_id: TabId,
    name: SharedString,
}

pub(super) fn tab_label(tab_index: usize, name: &str) -> String {
    let number = tab_index + 1;
    if name.is_empty() {
        number.to_string()
    } else {
        format!("{number}  {name}")
    }
}

impl Condr {
    /// The active Workspace as two pieces: its Tab strip, which the title bar hosts so
    /// the Panes get the full body height, and the body under it. No strip without a
    /// Workspace; the empty state carries the error itself.
    pub(super) fn render_workspace(
        &self,
        cx: &mut Context<Self>,
    ) -> (Option<AnyElement>, AnyElement) {
        let Some(connection) = self.active_connection() else {
            return (None, div().size_full().into_any_element());
        };
        let status = self.render_connection_status(connection, cx);
        // The status line carries the connection error itself; the strip shows the rest.
        let error = self
            .app_error
            .clone()
            .or_else(|| status.is_none().then(|| connection.error.clone()).flatten());
        let can_mutate = connection.can_mutate()
            && self
                .pending_workspace_selection_for(connection.key)
                .is_none()
            && !self.has_pending_projection_for(connection.key);
        let Ok(session) = Session::restore(connection.snapshot.clone()) else {
            return (
                None,
                div()
                    .size_full()
                    .child("Invalid Session state")
                    .into_any_element(),
            );
        };
        let key = connection.key;
        let Some(workspace_id) = self.presented_workspace_id(key, &session) else {
            return (
                None,
                self.render_welcome(key, can_mutate, status, error, cx),
            );
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
        // Drop handlers need the source index of the dragged Tab.
        let tab_ids = Rc::new(
            workspace
                .tabs()
                .iter()
                .map(|tab| tab.id())
                .collect::<Vec<_>>(),
        );
        let tab_buttons = workspace.tabs().iter().enumerate().map(|(tab_index, tab)| {
            let tab_id = tab.id();
            let tab_name = tab.name().to_owned();
            let tab_label = tab_label(tab_index, &tab_name);
            let activate_owner = cx.weak_entity();
            let menu_owner = cx.weak_entity();
            let target = DropTarget::Tab {
                key,
                tab_id,
                after: false,
            };
            let tab_row = h_flex()
                .id(("tab-menu", tab_id.as_u64()))
                .flex_shrink_0()
                // Inside the title bar a press would otherwise start a window move;
                // the Tab's own drag needs it.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .when(can_mutate, |this| {
                    let drop_owner = cx.weak_entity();
                    let tab_ids = tab_ids.clone();
                    this.on_drag(
                        DraggedTab {
                            key,
                            workspace_id,
                            tab_id,
                            name: tab_label.clone().into(),
                        },
                        move |drag, _, _, cx| {
                            cx.stop_propagation();
                            let name = drag.name.clone();
                            cx.new(|_| DragPreview { icon: None, name })
                        },
                    )
                    .on_drop(move |dragged: &DraggedTab, _, cx| {
                        let _ = drop_owner.update(cx, |this, _| {
                            let after = this.take_drop_after(target);
                            if dragged.key != key || dragged.workspace_id != workspace_id {
                                return;
                            }
                            let Some(source) = tab_ids.iter().position(|id| *id == dragged.tab_id)
                            else {
                                return;
                            };
                            if let Some(index) = drop_index(source, tab_index, after) {
                                this.send_layout_to(
                                    key,
                                    LayoutCommand::MoveTab {
                                        tab_id: dragged.tab_id,
                                        target_index: index as u32,
                                    },
                                );
                            }
                        });
                    })
                });
            let move_owner = cx.weak_entity();
            let tab_row = if can_mutate {
                attach_drop_target::<DraggedTab, _>(
                    tab_row,
                    true,
                    self.drop_indicator_for(target, cx),
                    move |half, cx| {
                        let _ = move_owner
                            .update(cx, |this, cx| this.set_drop_target(target, half, cx));
                    },
                    cx,
                )
            } else {
                tab_row
            };
            tab_row
                .child(
                    Button::new(("tab", tab_id.as_u64()))
                        .debug_selector(move || format!("tab-{}", tab_id.as_u64()))
                        .ghost()
                        .small()
                        .min_w(px(64.))
                        .max_w(px(128.))
                        .selected(tab_id == active_tab)
                        .label(tab_label.clone())
                        .tooltip(tab_label)
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
                        PopupMenuItem::new("Rename Tab")
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

        let strip = h_flex()
            .id("workspace-tabs")
            .debug_selector(|| "workspace-tabs".into())
            .h_full()
            .w_full()
            .min_w_0()
            .gap_1()
            .px_2()
            .items_center()
            .children(tab_buttons)
            .child(
                Button::new("new-tab")
                    .debug_selector(|| "new-tab".into())
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .tooltip("New Tab")
                    .disabled(!can_mutate)
                    // Like the Tab rows: inside the title bar an unclaimed press starts a
                    // window move on Windows, and the move swallows the click.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
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
            });
        // The banner floats over the Panes rather than reflowing them: the dock keeps
        // its geometry, and the frozen output under it is what the user is waiting on.
        let body = div()
            .size_full()
            .relative()
            .when_some(dock_area, |view, dock_area| view.child(dock_area))
            .when_some(status, |view, status| {
                view.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .px_3()
                        .py_1()
                        .bg(cx.theme().background)
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(status),
                )
            })
            .into_any_element();
        (Some(strip.into_any_element()), body)
    }
}

/// The width of the welcome page's action column: wide enough for the longest row,
/// narrow enough that the labels stay one short reading column in a maximized window.
const WELCOME_COLUMN_WIDTH: Pixels = px(400.);

/// One welcome row. A Button centers its content, so the trailing spacer takes the
/// leftover width and leaves the icons and labels on one left spine.
fn welcome_row(id: &'static str, icon: impl Into<Icon>, label: &'static str) -> Button {
    Button::new(id)
        .debug_selector(move || id.into())
        .ghost()
        .w_full()
        .icon(icon)
        .label(label)
        .child(div().flex_1())
}

impl Condr {
    /// What a Server with no Workspace shows: the brand, then the ways in. The
    /// connection's error belongs here too, since this page replaces the Tab strip
    /// that would otherwise carry it.
    fn render_welcome(
        &self,
        key: ConnectionKey,
        can_mutate: bool,
        status: Option<AnyElement>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open_owner = cx.weak_entity();
        let connect_owner = cx.weak_entity();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_8()
            .child(
                h_flex()
                    .gap_4()
                    .items_center()
                    .child(img(APP_LOGO).size_12())
                    .child(
                        v_flex()
                            .child(
                                div()
                                    .text_xl()
                                    .child(format!("Welcome to {}", condr_core::APP_NAME)),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .italic()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Keep your agents running"),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .w(WELCOME_COLUMN_WIDTH)
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_3()
                            .pb_1()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("GET STARTED"),
                            )
                            .child(Separator::horizontal().flex_1()),
                    )
                    .child(
                        welcome_row("open-project", IconName::FolderOpen, "Open Project")
                            .disabled(!can_mutate)
                            .on_click(move |_, window, cx| {
                                let _ = open_owner.update(cx, |this, cx| {
                                    this.choose_workspace_directory_on(key, window, cx)
                                });
                            }),
                    )
                    .child(
                        welcome_row(
                            "clone-repository",
                            CondrIconName::GitBranch,
                            "Clone Repository",
                        )
                        .disabled(true)
                        .tooltip("Not available yet"),
                    )
                    .child(
                        welcome_row(
                            "connect-remote-device",
                            CondrIconName::ServerPlus,
                            "Connect Remote Device",
                        )
                        .dropdown_menu(move |menu, _, _| {
                            Condr::add_server_menu(menu, connect_owner.clone())
                        }),
                    )
                    .when_some(status, |column, status| {
                        column.child(div().pt_2().child(status))
                    })
                    .when_some(error, |column, error| {
                        column.child(
                            div()
                                .pt_2()
                                .text_sm()
                                .text_color(cx.theme().danger)
                                .child(error),
                        )
                    }),
            )
            .into_any_element()
    }
}

impl Focusable for Condr {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// The drawn title bar every Condr window uses: logo, title, and the theme's flat
/// title-bar color rather than the Kit's default gradient.
pub(super) fn title_bar(title: &'static str, cx: &App) -> TitleBar {
    TitleBar::new().bg(cx.theme().title_bar).child(
        h_flex()
            .gap_2()
            .items_center()
            .child(img(APP_LOGO).size_4().flex_shrink_0())
            .child(title),
    )
}

/// The Kit's own inset before the title bar's content: room for macOS traffic lights,
/// a small margin elsewhere. Private to the Kit, so mirrored here to line the sidebar
/// segment up with the sidebar below it.
#[cfg(target_os = "macos")]
const TITLE_BAR_LEFT_PADDING: Pixels = px(80.);
#[cfg(not(target_os = "macos"))]
const TITLE_BAR_LEFT_PADDING: Pixels = px(12.);

/// The main window's title bar continues the sidebar and the Workspace: its left segment
/// is the sidebar's width and color, its right segment hosts the Tab strip, so the two
/// columns read as one surface each and the Panes get the height a separate strip took.
/// The Kit still draws the window controls and moves the window from empty space.
fn workspace_title_bar(
    sidebar_width: Pixels,
    sidebar_collapsed: bool,
    tab_strip: Option<AnyElement>,
    owner: WeakEntity<Condr>,
    cx: &App,
) -> TitleBar {
    let theme = cx.theme();
    let title_sidebar_width = if sidebar_collapsed {
        COLLAPSED_SIDEBAR_WIDTH
    } else {
        sidebar_width
    };
    let toggle_label = if sidebar_collapsed {
        "Expand Sidebar"
    } else {
        "Collapse Sidebar"
    };
    let toggle_icon = if sidebar_collapsed {
        IconName::PanelLeftOpen
    } else {
        IconName::PanelLeftClose
    };
    let toggle_owner = owner.clone();
    TitleBar::new()
        .h(WORKSPACE_TITLE_BAR_HEIGHT)
        // The sidebar's color reaches the window edge, traffic-light inset included.
        .bg(theme.sidebar)
        // The Kit draws the bottom edge, so it runs under its window controls too.
        .border_color(theme.border)
        .child(
            h_flex()
                .h_full()
                .w_full()
                .min_w_0()
                .child(
                    h_flex()
                        .debug_selector(|| "title-sidebar".into())
                        .w(title_sidebar_width - TITLE_BAR_LEFT_PADDING)
                        .flex_none()
                        .h_full()
                        .gap_2()
                        .items_center()
                        .justify_between()
                        .border_r_1()
                        .border_color(theme.sidebar_border)
                        .when(!sidebar_collapsed, |this| {
                            this.child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .flex_shrink_0()
                                    .child(img(APP_LOGO).size_4().flex_shrink_0())
                                    .child(condr_core::APP_NAME),
                            )
                        })
                        .child(
                            Button::new("toggle-sidebar")
                                .debug_selector(|| "toggle-sidebar".into())
                                .ghost()
                                .small()
                                .icon(Icon::new(toggle_icon))
                                .tooltip(toggle_label)
                                .accessibility_label(toggle_label)
                                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation();
                                })
                                .on_click(move |_, _, cx| {
                                    let _ = toggle_owner.update(cx, |this, cx| {
                                        this.sidebar_collapsed = !this.sidebar_collapsed;
                                        cx.notify();
                                    });
                                }),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .bg(theme.background)
                        .children(tab_strip),
                ),
        )
}

impl Condr {
    pub(super) fn sidebar_layout_width(&self) -> Pixels {
        if self.sidebar_collapsed {
            COLLAPSED_SIDEBAR_WIDTH
        } else {
            self.sidebar_width
        }
    }
}

impl Render for Condr {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dialog_layer = Root::render_dialog_layer(window, cx);
        let workspace_owner = cx.weak_entity();
        let sidebar_toggle_owner = cx.weak_entity();
        let (tab_strip, body) = self.render_workspace(cx);
        let workspace = div()
            .size_full()
            .on_prepaint(move |bounds, _, cx| {
                let _ = workspace_owner.update(cx, |this, _| {
                    this.workspace_size = bounds.size;
                });
            })
            .child(body)
            .into_any_element();
        div()
            .key_context("Condr")
            .relative()
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
            .on_action(cx.listener(Self::action_activate_tab))
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
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            // Client-side title bar on every platform, as Zed does; the OS title for the
            // taskbar is set separately when the window opens.
            .child(workspace_title_bar(
                self.sidebar_width,
                self.sidebar_collapsed,
                tab_strip,
                sidebar_toggle_owner,
                cx,
            ))
            .child(
                // The sidebar keeps an absolute width, the way Zed sizes its docks: a
                // resizable group would rescale it with the window on every resize.
                // Dragging the handle on its right edge is the only thing that moves it.
                h_flex()
                    .w_full()
                    .flex_1()
                    .min_h_0()
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
                            .w(self.sidebar_layout_width())
                            .flex_none()
                            .h_full()
                            .child(self.render_sidebar(cx))
                            // Deferred so it paints above the workspace it overlaps by
                            // half its width, occluding so the terminal underneath never
                            // sees the press, and swallowing the press so the pane does
                            // not start a selection.
                            .when(!self.sidebar_collapsed, |this| {
                                this.child(deferred(
                                    div()
                                        .id("condr-sidebar-resize")
                                        .debug_selector(|| "condr-sidebar-resize".into())
                                        .absolute()
                                        .top_0()
                                        .bottom_0()
                                        .right(-SIDEBAR_RESIZE_HANDLE_WIDTH / 2.)
                                        .w(SIDEBAR_RESIZE_HANDLE_WIDTH)
                                        .occlude()
                                        .cursor_col_resize()
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation()
                                        })
                                        .on_drag(DraggedSidebar, |_, _, _, cx| {
                                            cx.new(|_| EmptyView)
                                        }),
                                ))
                            }),
                    )
                    .child(workspace),
            )
            .children(dialog_layer)
            .when(self.fps_monitor, |this| this.child(fps_monitor(window, cx)))
    }
}

pub(super) fn default_worktree_branch(workspace_name: &str) -> String {
    let slug = workspace_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let slug = slug.trim_matches('-');
    format!("worktree/{}", if slug.is_empty() { "change" } else { slug })
}
