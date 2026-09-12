mod drag_drop;
mod icon;
mod item;

use super::*;
use item::*;

pub(super) use drag_drop::*;
pub(super) use icon::*;

/// The Workspace row's trailing column: its agents' states beside the name and, on a
/// repository row, `↑ ↓` beside the branch. The two boxes take `DETAIL_LINE_HEIGHTS` so
/// they sit level with the row's own two lines.
fn workspace_suffix(
    summary: &[(SidebarStatusVisual, usize)],
    upstream: Option<&str>,
    two_lines: bool,
    cx: &App,
) -> AnyElement {
    let states = h_flex()
        .items_center()
        .gap_x_1p5()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .children(summary.iter().map(|(status, count)| {
            h_flex()
                .items_center()
                .gap_x_0p5()
                .children(status_badge(status.glyph, status.tone, cx))
                .child(count.to_string())
        }));
    if !two_lines {
        return states.into_any_element();
    }
    v_flex()
        .items_end()
        .justify_center()
        .child(states.h(DETAIL_LINE_HEIGHTS.0))
        .child(
            div()
                .h(DETAIL_LINE_HEIGHTS.1)
                .line_height(DETAIL_LINE_HEIGHTS.1)
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .children(upstream.map(str::to_owned)),
        )
        .into_any_element()
}

/// Moves the dragged connection before or after the target connection. Returns
/// whether the order changed.
pub(super) fn reorder_connection(
    connections: &mut Vec<ServerConnection>,
    dragged: ConnectionKey,
    target: ConnectionKey,
    after: bool,
) -> bool {
    let Some(from) = connections.iter().position(|c| c.key == dragged) else {
        return false;
    };
    let Some(to) = connections.iter().position(|c| c.key == target) else {
        return false;
    };
    let Some(to) = drop_index(from, to, after) else {
        return false;
    };
    let connection = connections.remove(from);
    connections.insert(to, connection);
    true
}

impl Condr {
    fn move_server(
        &mut self,
        dragged: ConnectionKey,
        target: ConnectionKey,
        after: bool,
        cx: &mut Context<Self>,
    ) {
        if reorder_connection(&mut self.connections, dragged, target, after) {
            // ponytail: the saved order only covers TCP servers, so the local
            // server always loads first again after a restart.
            self.save_servers(cx);
        }
    }

    /// Disclosure state lives in the view so both chevron clicks and AgentChanged can
    /// update it, including for Server groups the Sidebar has not rendered yet. Call
    /// after every snapshot change; Workspaces keep their state across reconnects.
    pub(super) fn sync_sidebar_workspace_open(&mut self, cx: &mut Context<Self>) {
        let sessions = self.restored_sessions();
        self.sidebar_workspace_open
            .retain(|(key, workspace_id), _| {
                sessions
                    .get(key)
                    .is_some_and(|session| session.workspace(*workspace_id).is_some())
            });
        for (&key, session) in &sessions {
            let active_workspace = self.presented_workspace_id(key, session);
            for workspace in session.workspaces() {
                self.sidebar_workspace_open
                    .entry((key, workspace.id()))
                    .or_insert_with(|| {
                        cx.new(|_| {
                            key == self.active_connection
                                && active_workspace == Some(workspace.id())
                        })
                    });
            }
        }
    }

    fn restored_sessions(&self) -> HashMap<ConnectionKey, Session> {
        self.connections
            .iter()
            .filter_map(|connection| {
                Session::restore(connection.snapshot.clone())
                    .ok()
                    .map(|session| (connection.key, session))
            })
            .collect()
    }

    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.sidebar_collapsed {
            return self.render_collapsed_sidebar(cx);
        }
        let owner = cx.weak_entity();
        let sessions = self.restored_sessions();
        let items = self.connections.iter().map(|connection| {
            let key = connection.key;
            let server_target = DropTarget::Server { key, after: false };
            let active_server = key == self.active_connection;
            let connected = connection.can_mutate();
            let workspaces = sessions
                .get(&key)
                .map(|session| {
                    let active_workspace = self.presented_workspace_id(key, session);
                    // Drop handlers need the source index of the dragged Workspace.
                    let workspace_ids = Rc::new(
                        session
                            .workspaces()
                            .iter()
                            .map(|workspace| workspace.id())
                            .collect::<Vec<_>>(),
                    );
                    session
                        .workspaces()
                        .iter()
                        .enumerate()
                        .map(|(workspace_index, workspace)| {
                            let workspace_id = workspace.id();
                            let workspace_name = workspace.name().to_owned();
                            let git = connection.workspace_git.get(&workspace_id);
                            // A repository always gets its second line, `detached` included,
                            // so Workspaces of one Server keep one row height.
                            let detail = git.map(|git| {
                                git.branch.clone().unwrap_or_else(|| "detached".to_owned())
                            });
                            let upstream =
                                git.and_then(|git| git.upstream).and_then(upstream_label);
                            let mut statuses = Vec::new();
                            let supports_worktrees = git.is_some_and(|git| !git.linked_worktree)
                                && workspace.worktree().is_none();
                            let managed_worktree = workspace
                                .worktree()
                                .is_some_and(|association| association.is_managed());
                            let agents = workspace
                                .tabs()
                                .iter()
                                .flat_map(|tab| tab.panes())
                                .filter_map(|pane| {
                                    let pane_id = pane.id();
                                    let agent = connection.agents.get(&pane_id)?;
                                    let state = connection
                                        .agent_trackers
                                        .get(&pane_id)
                                        .map(|tracker| tracker.display_state())
                                        .unwrap_or_else(|| {
                                            AgentTracker::new(agent.state).display_state()
                                        });
                                    let status = if connection.controlling
                                        && connection.attention.contains(&pane_id)
                                    {
                                        BELL_SIDEBAR_STATUS
                                    } else {
                                        agent_sidebar_status(state)
                                    };
                                    statuses.push(status);
                                    let agent_label = agent.kind.label();
                                    // The Pane's title is what the agent is doing; the
                                    // mark already says which agent, so the name is only
                                    // the fallback before a title arrives.
                                    let row_label = connection
                                        .terminal_titles
                                        .get(&pane_id)
                                        .cloned()
                                        .unwrap_or_else(|| agent_label.to_owned());
                                    let agent_owner = owner.clone();
                                    Some(
                                        CondrSidebarTreeItem::new(
                                            format!("sidebar-agent-{key}-{}", pane_id.as_u64()),
                                            format!("agent-{key}-{}", pane_id.as_u64()),
                                            format!("agent-label-{key}-{}", pane_id.as_u64()),
                                            row_label,
                                        )
                                        .icon(CondrSidebarIcon::agent(
                                            agent.kind,
                                            status,
                                            format!(
                                                "agent-status-{key}-{}-{}",
                                                pane_id.as_u64(),
                                                status.key
                                            ),
                                            format!("{agent_label}: {}", status.label),
                                        ))
                                        .active(
                                            active_server
                                                && self.target_pane == Some((key, pane_id)),
                                        )
                                        .disable(!connected)
                                        .on_click(
                                            move |_, window, cx| {
                                                let _ = agent_owner.update(cx, |this, cx| {
                                                    this.select_pane(key, pane_id, window, cx)
                                                });
                                            },
                                        ),
                                    )
                                })
                                .collect::<Vec<_>>();
                            let summary = agent_status_summary(statuses);
                            let owner = owner.clone();
                            let menu_owner = owner.clone();
                            let agent_selected = agents.iter().any(|agent| agent.active);
                            CondrSidebarTreeItem::new(
                                format!("sidebar-workspace-{key}-{}", workspace_id.as_u64()),
                                format!("workspace-{key}-{}", workspace_id.as_u64()),
                                format!("workspace-label-{key}-{}", workspace_id.as_u64()),
                                workspace_name.clone(),
                            )
                            .icon(CondrSidebarIcon::avatar(
                                &workspace_name,
                                &workspace.root_directory().to_string_lossy(),
                                format!("workspace-icon-{key}-{}", workspace_id.as_u64()),
                            ))
                            .active(
                                active_server
                                    && active_workspace == Some(workspace_id)
                                    && !agent_selected,
                            )
                            .tree_parent(
                                format!("workspace-toggle-{key}-{}", workspace_id.as_u64()),
                                self.sidebar_workspace_open[&(key, workspace_id)].clone(),
                            )
                            .children(agents)
                            .disable(!connected)
                            .when(connected, |item| {
                                let drop_owner = owner.clone();
                                let move_owner = owner.clone();
                                let target = DropTarget::Workspace {
                                    key,
                                    workspace_id,
                                    after: false,
                                };
                                let workspace_ids = workspace_ids.clone();
                                item.draggable(DraggedWorkspace {
                                    key,
                                    workspace_id,
                                    name: workspace_name.clone().into(),
                                })
                                .drop_target(
                                    self.drop_indicator_for(target, cx),
                                    move |half, cx| {
                                        let _ = move_owner.update(cx, |this, cx| {
                                            this.set_drop_target(target, half, cx)
                                        });
                                    },
                                )
                                .on_drop(move |dragged, _, cx| {
                                    let _ = drop_owner.update(cx, |this, _| {
                                        let after = this.take_drop_after(target);
                                        if dragged.key != key {
                                            return;
                                        }
                                        let Some(source) = workspace_ids
                                            .iter()
                                            .position(|id| *id == dragged.workspace_id)
                                        else {
                                            return;
                                        };
                                        if let Some(index) =
                                            drop_index(source, workspace_index, after)
                                        {
                                            this.send_layout_to(
                                                key,
                                                LayoutCommand::MoveWorkspace {
                                                    workspace_id: dragged.workspace_id,
                                                    target_index: index as u32,
                                                },
                                            );
                                        }
                                    });
                                })
                            })
                            .context_menu(move |menu, _, _| {
                                let rename_owner = menu_owner.clone();
                                let create_owner = menu_owner.clone();
                                let open_owner = menu_owner.clone();
                                let remove_owner = menu_owner.clone();
                                let close_owner = menu_owner.clone();
                                let rename_name = workspace_name.clone();
                                let create_name = workspace_name.clone();
                                let open_name = workspace_name.clone();
                                let remove_name = workspace_name.clone();
                                let menu = menu.item(
                                    PopupMenuItem::new("Rename Workspace")
                                        .disabled(!connected)
                                        .on_click(move |_, window, cx| {
                                            let name = rename_name.clone();
                                            let _ = rename_owner.update(cx, |this, cx| {
                                                this.prompt_rename_workspace_on(
                                                    key,
                                                    workspace_id,
                                                    name,
                                                    window,
                                                    cx,
                                                )
                                            });
                                        }),
                                );
                                let menu = if supports_worktrees {
                                    menu.separator()
                                        .item(
                                            PopupMenuItem::new("Create Worktree")
                                                .disabled(!connected)
                                                .on_click(move |_, window, cx| {
                                                    let name = create_name.clone();
                                                    let _ = create_owner.update(cx, |this, cx| {
                                                        this.prompt_create_worktree_on(
                                                            key,
                                                            workspace_id,
                                                            name,
                                                            window,
                                                            cx,
                                                        )
                                                    });
                                                }),
                                        )
                                        .item(
                                            PopupMenuItem::new("Open Existing Worktree")
                                                .disabled(!connected)
                                                .on_click(move |_, window, cx| {
                                                    let name = open_name.clone();
                                                    let _ = open_owner.update(cx, |this, cx| {
                                                        this.choose_worktree_directory_on(
                                                            key,
                                                            workspace_id,
                                                            name,
                                                            window,
                                                            cx,
                                                        )
                                                    });
                                                }),
                                        )
                                } else {
                                    menu
                                };
                                let menu = if managed_worktree {
                                    menu.separator().item(
                                        PopupMenuItem::new("Remove Worktree")
                                            .disabled(!connected)
                                            .on_click(move |_, window, cx| {
                                                let name = remove_name.clone();
                                                let _ = remove_owner.update(cx, |this, cx| {
                                                    this.confirm_remove_worktree_on(
                                                        key,
                                                        workspace_id,
                                                        name,
                                                        window,
                                                        cx,
                                                    )
                                                });
                                            }),
                                    )
                                } else {
                                    menu
                                };
                                menu.separator().item(
                                    PopupMenuItem::new("Close Workspace")
                                        .disabled(!connected)
                                        .on_click(move |_, window, cx| {
                                            let _ = close_owner.update(cx, |this, cx| {
                                                this.close_workspace_id(
                                                    key,
                                                    workspace_id,
                                                    window,
                                                    cx,
                                                )
                                            });
                                        }),
                                )
                            })
                            .when_some(detail.clone(), |item, detail| item.detail(detail))
                            .when(!summary.is_empty() || upstream.is_some(), |item| {
                                let two_lines = detail.is_some();
                                item.suffix(move |_, cx| {
                                    workspace_suffix(&summary, upstream.as_deref(), two_lines, cx)
                                })
                            })
                            .on_click(move |_, window, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    this.select_workspace(key, workspace_id, window, cx)
                                });
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let select_owner = owner.clone();
            let new_workspace_owner = owner.clone();
            let server_menu_owner = owner.clone();
            let server_name = connection.label.clone();
            let status = connection.status;
            let is_local = connection.endpoint.as_local_path().is_some();
            let new_workspace_label = connection.label.clone();
            let descendant_selected = workspaces.iter().any(CondrSidebarTreeItem::subtree_active);
            CondrSidebarSection::new(
                connection.label.clone(),
                format!("server-heading-{key}"),
                move |_, _| {
                    let owner = new_workspace_owner.clone();
                    let tooltip = format!("New Workspace on {new_workspace_label}");
                    Button::new(("new-workspace", key))
                        .debug_selector(move || format!("new-workspace-server-{key}"))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(CondrIconName::FolderPlus))
                        .tooltip(tooltip)
                        .disabled(!connected)
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            let _ = owner.update(cx, |this, cx| {
                                this.choose_workspace_directory_on(key, window, cx)
                            });
                        })
                        .into_any_element()
                },
                workspaces,
            )
            .active(active_server && !descendant_selected)
            .draggable(DraggedServer {
                key,
                name: connection.label.clone().into(),
            })
            .drop_target(self.drop_indicator_for(server_target, cx), {
                let move_owner = owner.clone();
                move |half, cx| {
                    let _ = move_owner
                        .update(cx, |this, cx| this.set_drop_target(server_target, half, cx));
                }
            })
            .on_drop({
                let drop_owner = owner.clone();
                move |dragged, _, cx| {
                    let _ = drop_owner.update(cx, |this, cx| {
                        let after = this.take_drop_after(server_target);
                        this.move_server(dragged.key, key, after, cx);
                        cx.notify();
                    });
                }
            })
            .context_menu(move |menu, _, _| {
                let edit_owner = server_menu_owner.clone();
                let connection_owner = server_menu_owner.clone();
                let delete_owner = server_menu_owner.clone();
                let delete_name = server_name.clone();
                let (connection_label, connection_disabled) = match status {
                    ConnectionStatus::Connected => ("Disconnect", false),
                    ConnectionStatus::Disconnected => ("Connect", false),
                    ConnectionStatus::Connecting => ("Connecting…", true),
                };
                // The Local Server is this machine's own: nothing to edit, nothing to
                // delete, only its connection to toggle.
                let menu = if is_local {
                    menu
                } else {
                    menu.item(PopupMenuItem::new("Edit").on_click(move |_, window, cx| {
                        let _ = edit_owner
                            .update(cx, |this, cx| this.prompt_edit_server_on(key, window, cx));
                    }))
                };
                let menu = menu.item(
                    PopupMenuItem::new(connection_label)
                        .disabled(connection_disabled)
                        .on_click(move |_, window, cx| {
                            let _ = connection_owner.update(cx, |this, cx| {
                                match status {
                                    ConnectionStatus::Connected => {
                                        if this.disconnect_server(key) {
                                            this.refresh_target_pane(this.active_connection);
                                            this.rebuild_dock(window, cx);
                                        }
                                    }
                                    ConnectionStatus::Disconnected => {
                                        if this.start_connect(key) {
                                            this.refresh_target_pane(this.active_connection);
                                            this.rebuild_dock(window, cx);
                                        }
                                    }
                                    ConnectionStatus::Connecting => {}
                                }
                                cx.notify();
                            });
                        }),
                );
                if is_local {
                    menu
                } else {
                    menu.separator().item(PopupMenuItem::new("Delete").on_click(
                        move |_, window, cx| {
                            let name = delete_name.clone();
                            let _ = delete_owner.update(cx, |this, cx| {
                                this.confirm_delete_server_on(key, name, window, cx)
                            });
                        },
                    ))
                }
            })
            .on_click(move |_, window, cx| {
                let _ = select_owner.update(cx, |this, cx| this.select_server(key, window, cx));
            })
        });

        let add_owner = cx.weak_entity();
        let settings_owner = cx.weak_entity();
        Sidebar::new("condr-sidebar")
            .collapsible(SidebarCollapsible::None)
            .w_full()
            .children(items)
            // The app name lives in the title bar; the actions sit at the bottom right,
            // Connect Remote Device left of Settings. Reconnect is the `ReconnectServer`
            // action; it has no button here.
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap_1()
                    .child(
                        Button::new("add-server")
                            .debug_selector(|| "add-server".into())
                            .ghost()
                            .small()
                            .icon(Icon::new(CondrIconName::ServerPlus))
                            .tooltip("Connect Remote Device")
                            .accessibility_label("Connect Remote Device")
                            .dropdown_menu(move |menu, _, _| {
                                Condr::add_server_menu(menu, add_owner.clone())
                            })
                            .anchor(Anchor::BottomLeft),
                    )
                    .child(
                        Button::new("open-settings")
                            .debug_selector(|| "open-settings".into())
                            .ghost()
                            .small()
                            .icon(IconName::Settings)
                            .tooltip("Settings")
                            .accessibility_label("Settings")
                            .on_click(move |_, window, cx| {
                                let _ = settings_owner
                                    .update(cx, |this, cx| this.open_settings(window, cx));
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_collapsed_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let owner = cx.weak_entity();
        let sessions = self.restored_sessions();
        let mut avatars = Vec::new();
        for connection in &self.connections {
            let connection_key = connection.key;
            let Some(session) = sessions.get(&connection.key) else {
                continue;
            };
            let active_workspace = self.presented_workspace_id(connection.key, session);
            for workspace in session.workspaces() {
                let workspace_id = workspace.id();
                let active = connection.key == self.active_connection
                    && active_workspace == Some(workspace_id);
                let workspace_name = workspace.name().to_owned();
                let icon = CondrSidebarIcon::avatar(
                    &workspace_name,
                    &workspace.root_directory().to_string_lossy(),
                    format!(
                        "collapsed-workspace-icon-{}-{}",
                        connection_key,
                        workspace_id.as_u64()
                    ),
                );
                let item_owner = owner.clone();
                let button = Button::new(format!(
                    "collapsed-workspace-{}-{}",
                    connection_key,
                    workspace_id.as_u64()
                ))
                .debug_selector(move || {
                    format!(
                        "collapsed-workspace-{}-{}",
                        connection_key,
                        workspace_id.as_u64()
                    )
                })
                .ghost()
                .small()
                .tooltip(workspace_name.clone())
                .when(active, |this| {
                    this.bg(cx.theme().sidebar_accent)
                        .text_color(cx.theme().sidebar_accent_foreground)
                })
                .on_click(move |_, window, cx| {
                    let _ = item_owner.update(cx, |this, cx| {
                        this.select_workspace(connection_key, workspace_id, window, cx)
                    });
                })
                .child(icon.render(cx));
                avatars.push(button.into_any_element());
            }
        }
        v_flex()
            .size_full()
            .items_center()
            .gap_1()
            .pt_2()
            .bg(cx.theme().tokens.sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .children(avatars)
            .into_any_element()
    }
}
