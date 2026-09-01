use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SidebarIconTone {
    Default,
    Muted,
    Success,
    Warning,
    Danger,
}

impl SidebarIconTone {
    pub(super) fn color(self, cx: &App) -> Hsla {
        match self {
            Self::Default => cx.theme().sidebar_foreground,
            Self::Muted => cx.theme().muted_foreground,
            Self::Success => cx.theme().success,
            Self::Warning => cx.theme().warning,
            Self::Danger => cx.theme().danger,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CondrIconName {
    Circle,
    CircleAlert,
}

impl IconNamed for CondrIconName {
    fn path(self) -> SharedString {
        match self {
            Self::Circle => "icons/circle.svg",
            Self::CircleAlert => "icons/circle-alert.svg",
        }
        .into()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SidebarGlyph {
    Folder,
    HardDrive,
    Info,
    Circle,
    LoaderCircle,
    CircleAlert,
    CircleCheck,
}

impl SidebarGlyph {
    pub(super) fn icon(self) -> Icon {
        match self {
            Self::Folder => Icon::new(IconName::Folder),
            Self::HardDrive => Icon::new(IconName::HardDrive),
            Self::Info => Icon::new(IconName::Info),
            Self::Circle => Icon::new(CondrIconName::Circle),
            Self::LoaderCircle => Icon::new(IconName::LoaderCircle),
            Self::CircleAlert => Icon::new(CondrIconName::CircleAlert),
            Self::CircleCheck => Icon::new(IconName::CircleCheck),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SidebarStatusVisual {
    pub(super) glyph: SidebarGlyph,
    pub(super) tone: SidebarIconTone,
    pub(super) key: &'static str,
    pub(super) label: &'static str,
}

pub(super) fn server_sidebar_status(
    status: ConnectionStatus,
    synchronized: bool,
) -> SidebarStatusVisual {
    match (status, synchronized) {
        (ConnectionStatus::Connected, true) => SidebarStatusVisual {
            glyph: SidebarGlyph::HardDrive,
            tone: SidebarIconTone::Success,
            key: "connected",
            label: "Connected",
        },
        (ConnectionStatus::Connected, false) => SidebarStatusVisual {
            glyph: SidebarGlyph::HardDrive,
            tone: SidebarIconTone::Warning,
            key: "syncing",
            label: "Syncing",
        },
        (ConnectionStatus::Connecting, _) => SidebarStatusVisual {
            glyph: SidebarGlyph::HardDrive,
            tone: SidebarIconTone::Warning,
            key: "connecting",
            label: "Connecting",
        },
        (ConnectionStatus::Disconnected, _) => SidebarStatusVisual {
            glyph: SidebarGlyph::HardDrive,
            tone: SidebarIconTone::Danger,
            key: "offline",
            label: "Offline",
        },
    }
}

pub(super) fn agent_sidebar_status(state: AgentDisplayState) -> SidebarStatusVisual {
    match state {
        AgentDisplayState::Unknown => SidebarStatusVisual {
            glyph: SidebarGlyph::Info,
            tone: SidebarIconTone::Muted,
            key: "unknown",
            label: "Unknown",
        },
        AgentDisplayState::Idle => SidebarStatusVisual {
            glyph: SidebarGlyph::Circle,
            tone: SidebarIconTone::Muted,
            key: "idle",
            label: "Idle",
        },
        AgentDisplayState::Working => SidebarStatusVisual {
            glyph: SidebarGlyph::LoaderCircle,
            tone: SidebarIconTone::Warning,
            key: "working",
            label: "Working",
        },
        AgentDisplayState::Blocked => SidebarStatusVisual {
            glyph: SidebarGlyph::CircleAlert,
            tone: SidebarIconTone::Danger,
            key: "blocked",
            label: "Blocked",
        },
        AgentDisplayState::Done => SidebarStatusVisual {
            glyph: SidebarGlyph::CircleCheck,
            tone: SidebarIconTone::Success,
            key: "done",
            label: "Done",
        },
    }
}

#[derive(Clone)]
pub(super) struct CondrSidebarIcon {
    glyph: SidebarGlyph,
    tone: SidebarIconTone,
    selector: SharedString,
    tooltip: Option<SharedString>,
}

impl CondrSidebarIcon {
    pub(super) fn new(glyph: SidebarGlyph, selector: impl Into<SharedString>) -> Self {
        Self {
            glyph,
            tone: SidebarIconTone::Default,
            selector: selector.into(),
            tooltip: None,
        }
    }

    pub(super) fn status(
        visual: SidebarStatusVisual,
        selector: impl Into<SharedString>,
        tooltip: impl Into<SharedString>,
    ) -> Self {
        Self {
            glyph: visual.glyph,
            tone: visual.tone,
            selector: selector.into(),
            tooltip: Some(tooltip.into()),
        }
    }

    pub(super) fn render(self, cx: &mut App) -> AnyElement {
        let color = self.tone.color(cx);
        let graphic = self
            .glyph
            .icon()
            .size_4()
            .text_color(color)
            .into_any_element();
        let id = self.selector.clone();
        let debug_selector = self.selector;

        div()
            .id(id)
            .debug_selector(move || debug_selector.to_string())
            .size_4()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .when_some(self.tooltip, |this, tooltip| {
                this.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            })
            .child(graphic)
            .into_any_element()
    }
}

type SidebarClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type SidebarSuffixBuilder = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;
type SidebarContextMenuBuilder = Rc<dyn Fn(PopupMenu, &mut Window, &mut App) -> PopupMenu>;

#[derive(Clone)]
pub(super) struct CondrSidebarTreeItem {
    id: SharedString,
    row_selector: SharedString,
    label_selector: SharedString,
    toggle_selector: Option<SharedString>,
    label: SharedString,
    icon: CondrSidebarIcon,
    handler: SidebarClickHandler,
    active: bool,
    default_open: bool,
    reserve_toggle_space: bool,
    children: Vec<Self>,
    suffix: Option<SidebarSuffixBuilder>,
    disabled: bool,
    context_menu: Option<SidebarContextMenuBuilder>,
}

impl FluentBuilder for CondrSidebarTreeItem {}

impl CondrSidebarTreeItem {
    pub(super) fn new(
        id: impl Into<SharedString>,
        row_selector: impl Into<SharedString>,
        label_selector: impl Into<SharedString>,
        label: impl Into<SharedString>,
        icon: CondrSidebarIcon,
    ) -> Self {
        Self {
            id: id.into(),
            row_selector: row_selector.into(),
            label_selector: label_selector.into(),
            toggle_selector: None,
            label: label.into(),
            icon,
            handler: Rc::new(|_, _, _| {}),
            active: false,
            default_open: false,
            reserve_toggle_space: false,
            children: Vec::new(),
            suffix: None,
            disabled: false,
            context_menu: None,
        }
    }

    pub(super) fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub(super) fn tree_parent(mut self, toggle_selector: impl Into<SharedString>) -> Self {
        self.toggle_selector = Some(toggle_selector.into());
        self.reserve_toggle_space = true;
        self
    }

    pub(super) fn default_open(mut self, open: bool) -> Self {
        self.default_open = open;
        self
    }

    pub(super) fn children(mut self, children: impl IntoIterator<Item = Self>) -> Self {
        self.children = children.into_iter().collect();
        self
    }

    pub(super) fn suffix<F, E>(mut self, builder: F) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.suffix = Some(Rc::new(move |window, cx| {
            builder(window, cx).into_any_element()
        }));
        self
    }

    pub(super) fn disable(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub(super) fn context_menu(
        mut self,
        builder: impl Fn(PopupMenu, &mut Window, &mut App) -> PopupMenu + 'static,
    ) -> Self {
        self.context_menu = Some(Rc::new(builder));
        self
    }

    pub(super) fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.handler = Rc::new(handler);
        self
    }

    pub(super) fn render(self, window: &mut Window, cx: &mut App) -> AnyElement {
        let Self {
            id,
            row_selector,
            label_selector,
            toggle_selector,
            label,
            icon,
            handler,
            active,
            default_open,
            reserve_toggle_space,
            children,
            suffix,
            disabled,
            context_menu,
        } = self;
        let is_submenu = !children.is_empty();
        let open_state = reserve_toggle_space.then(|| {
            window.use_keyed_state(
                SharedString::from(format!("condr-sidebar-open-{id}")),
                cx,
                |_, _| default_open,
            )
        });
        let is_open = open_state.as_ref().is_some_and(|state| *state.read(cx));
        let show_children = is_open && is_submenu;
        let rendered_children = if show_children {
            children
                .into_iter()
                .map(|child| child.render(window, cx))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let row_debug_selector = row_selector.clone();
        let label_debug_selector = label_selector;
        let row = h_flex()
            .id(id.clone())
            .debug_selector(move || row_debug_selector.to_string())
            .w_full()
            .h_8()
            .min_w_0()
            .overflow_x_hidden()
            .flex_shrink_0()
            .px_1()
            .gap_x_2()
            .rounded(cx.theme().radius)
            .text_sm()
            .when(!active && !disabled, |this| {
                this.hover(|this| {
                    this.bg(cx.theme().sidebar_accent.opacity(0.8))
                        .text_color(cx.theme().sidebar_accent_foreground)
                })
            })
            .when(active, |this| {
                this.font_medium()
                    .bg(cx.theme().tokens.sidebar_accent)
                    .text_color(cx.theme().sidebar_accent_foreground)
            })
            .when(reserve_toggle_space, |this| {
                let toggle_debug_selector = toggle_selector
                    .clone()
                    .unwrap_or_else(|| format!("{id}-toggle").into());
                let button = Button::new(format!("{id}-toggle"))
                    .debug_selector(move || toggle_debug_selector.to_string())
                    .xsmall()
                    .ghost()
                    .icon(
                        Icon::new(IconName::ChevronRight)
                            .size_3p5()
                            .when(is_open, |icon| icon.rotate(percentage(90. / 360.))),
                    );
                let toggle_tooltip = if is_open {
                    format!("Collapse {label}")
                } else {
                    format!("Expand {label}")
                };
                let open_state = open_state
                    .clone()
                    .expect("tree parents always own disclosure state");
                this.child(button.tooltip(toggle_tooltip).on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    open_state.update(cx, |open, cx| {
                        *open = !*open;
                        cx.notify();
                    });
                }))
            })
            .child(icon.render(cx))
            .child(
                div()
                    .debug_selector(move || label_debug_selector.to_string())
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .child(label),
            )
            .when_some(suffix, |this, suffix| {
                this.child(suffix(window, cx).into_any_element())
            })
            .when(disabled, |this| {
                this.text_color(cx.theme().muted_foreground)
            })
            .when(!disabled, |this| {
                this.on_click(move |event, window, cx| handler(event, window, cx))
            });
        let row = if let Some(context_menu) = context_menu {
            row.context_menu(move |menu, window, cx| context_menu(menu, window, cx))
                .into_any_element()
        } else {
            row.into_any_element()
        };

        v_flex()
            .w_full()
            .child(row)
            .when(show_children, |this| {
                this.child(
                    v_flex()
                        .border_l_1()
                        .border_color(cx.theme().sidebar_border)
                        .gap_1()
                        .ml_3p5()
                        .pl_2p5()
                        .py_0p5()
                        .children(rendered_children),
                )
            })
            .into_any_element()
    }
}

#[derive(Clone)]
pub(super) struct CondrSidebarSection {
    label: SharedString,
    heading_selector: SharedString,
    action: SidebarSuffixBuilder,
    items: Vec<CondrSidebarTreeItem>,
    collapsed: bool,
}

impl CondrSidebarSection {
    pub(super) fn new(
        label: impl Into<SharedString>,
        heading_selector: impl Into<SharedString>,
        action: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
        items: impl IntoIterator<Item = CondrSidebarTreeItem>,
    ) -> Self {
        Self {
            label: label.into(),
            heading_selector: heading_selector.into(),
            action: Rc::new(action),
            items: items.into_iter().collect(),
            collapsed: false,
        }
    }
}

impl Collapsible for CondrSidebarSection {
    fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    fn is_collapsed(&self) -> bool {
        self.collapsed
    }
}

impl SidebarItem for CondrSidebarSection {
    fn render(
        self,
        _id: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let heading_row_debug_selector =
            SharedString::from(format!("{}-row", self.heading_selector));
        let heading_debug_selector = self.heading_selector;
        let rendered_items = self
            .items
            .into_iter()
            .map(|item| item.render(window, cx))
            .collect::<Vec<_>>();

        v_flex().relative().when(!self.collapsed, |this| {
            this.child(
                h_flex()
                    .debug_selector(move || heading_row_debug_selector.to_string())
                    .h_9()
                    .w_full()
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .pl_1()
                    .text_xs()
                    .text_color(cx.theme().sidebar_foreground.opacity(0.7))
                    .child(
                        div()
                            .debug_selector(move || heading_debug_selector.to_string())
                            .child(self.label),
                    )
                    .child((self.action)(window, cx)),
            )
            .child(v_flex().w_full().gap_1().children(rendered_items))
        })
    }
}

impl Condr {
    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.weak_entity();
        let items = self.connections.iter().map(|connection| {
            let key = connection.key;
            let active_server = key == self.active_connection;
            let connected = connection.can_mutate();
            let server_status =
                server_sidebar_status(connection.status, connection.is_synchronized());
            let workspaces = Session::restore(connection.snapshot.clone())
                .ok()
                .map(|session| {
                    let active_workspace = self.presented_workspace_id(key, &session);
                    let workspace_count = session.workspaces().len();
                    session
                        .workspaces()
                        .iter()
                        .enumerate()
                        .map(|(workspace_index, workspace)| {
                            let workspace_id = workspace.id();
                            let workspace_name = workspace.name().to_owned();
                            let git = connection.workspace_git.get(&workspace_id);
                            let branch = git.and_then(|git| git.branch.clone());
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
                                    let status = agent_sidebar_status(state);
                                    let agent_label = agent.kind.label();
                                    let agent_owner = owner.clone();
                                    Some(
                                        CondrSidebarTreeItem::new(
                                            format!("sidebar-agent-{key}-{}", pane_id.as_u64()),
                                            format!("agent-{key}-{}", pane_id.as_u64()),
                                            format!("agent-label-{key}-{}", pane_id.as_u64()),
                                            agent_label,
                                            CondrSidebarIcon::status(
                                                status,
                                                format!(
                                                    "agent-status-{key}-{}-{}",
                                                    pane_id.as_u64(),
                                                    status.key
                                                ),
                                                format!("{agent_label}: {}", status.label),
                                            ),
                                        )
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
                            let owner = owner.clone();
                            let menu_owner = owner.clone();
                            CondrSidebarTreeItem::new(
                                format!("sidebar-workspace-{key}-{}", workspace_id.as_u64()),
                                format!("workspace-{key}-{}", workspace_id.as_u64()),
                                format!("workspace-label-{key}-{}", workspace_id.as_u64()),
                                workspace_name.clone(),
                                CondrSidebarIcon::new(
                                    SidebarGlyph::Folder,
                                    format!("workspace-icon-{key}-{}", workspace_id.as_u64()),
                                ),
                            )
                            .active(active_server && active_workspace == Some(workspace_id))
                            .tree_parent(format!(
                                "workspace-toggle-{key}-{}",
                                workspace_id.as_u64()
                            ))
                            .default_open(active_server && active_workspace == Some(workspace_id))
                            .children(agents)
                            .disable(!connected)
                            .context_menu(move |menu, _, _| {
                                let rename_owner = menu_owner.clone();
                                let create_owner = menu_owner.clone();
                                let open_owner = menu_owner.clone();
                                let remove_owner = menu_owner.clone();
                                let move_up_owner = menu_owner.clone();
                                let move_down_owner = menu_owner.clone();
                                let close_owner = menu_owner.clone();
                                let rename_name = workspace_name.clone();
                                let create_name = workspace_name.clone();
                                let open_name = workspace_name.clone();
                                let remove_name = workspace_name.clone();
                                let menu = menu.item(
                                    PopupMenuItem::new("Rename Workspace…")
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
                                            PopupMenuItem::new("Create Worktree…")
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
                                            PopupMenuItem::new("Open Existing Worktree…")
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
                                        PopupMenuItem::new("Remove Worktree…")
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
                                let menu = menu
                                    .separator()
                                    .item(
                                        PopupMenuItem::new("Move Up")
                                            .disabled(!connected || workspace_index == 0)
                                            .on_click(move |_, _, cx| {
                                                let _ = move_up_owner.update(cx, |this, _| {
                                                    this.send_layout_to(
                                                        key,
                                                        LayoutCommand::MoveWorkspace {
                                                            workspace_id,
                                                            target_index: workspace_index
                                                                .saturating_sub(1)
                                                                as u32,
                                                        },
                                                    );
                                                });
                                            }),
                                    )
                                    .item(
                                        PopupMenuItem::new("Move Down")
                                            .disabled(
                                                !connected
                                                    || workspace_index + 1 >= workspace_count,
                                            )
                                            .on_click(move |_, _, cx| {
                                                let _ = move_down_owner.update(cx, |this, _| {
                                                    this.send_layout_to(
                                                        key,
                                                        LayoutCommand::MoveWorkspace {
                                                            workspace_id,
                                                            target_index: (workspace_index + 1)
                                                                as u32,
                                                        },
                                                    );
                                                });
                                            }),
                                    );
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
                            .when_some(branch, |item, branch| {
                                item.suffix(move |_, cx| {
                                    div()
                                        .max_w(px(84.0))
                                        .truncate()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(branch.clone())
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
            let new_workspace_label = connection.label.clone();
            CondrSidebarTreeItem::new(
                format!("sidebar-server-{key}"),
                format!("server-{key}"),
                format!("server-label-{key}"),
                connection.label.clone(),
                CondrSidebarIcon::status(
                    server_status,
                    format!("server-status-{key}-{}", server_status.key),
                    format!("{}: {}", connection.label, server_status.label),
                ),
            )
            .active(active_server)
            .tree_parent(format!("server-toggle-{key}"))
            .default_open(active_server)
            .children(workspaces)
            .context_menu(move |menu, _, _| {
                let rename_owner = server_menu_owner.clone();
                let connection_owner = server_menu_owner.clone();
                let delete_owner = server_menu_owner.clone();
                let rename_name = server_name.clone();
                let delete_name = server_name.clone();
                let (connection_label, connection_disabled) = match status {
                    ConnectionStatus::Connected => ("Disconnect", false),
                    ConnectionStatus::Disconnected => ("Connect", false),
                    ConnectionStatus::Connecting => ("Connecting…", true),
                };
                menu.item(
                    PopupMenuItem::new("Rename Server…").on_click(move |_, window, cx| {
                        let name = rename_name.clone();
                        let _ = rename_owner.update(cx, |this, cx| {
                            this.prompt_rename_server_on(key, name, window, cx)
                        });
                    }),
                )
                .item(
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
                )
                .separator()
                .item(
                    PopupMenuItem::new("Delete Server").on_click(move |_, window, cx| {
                        let name = delete_name.clone();
                        let _ = delete_owner.update(cx, |this, cx| {
                            this.confirm_delete_server_on(key, name, window, cx)
                        });
                    }),
                )
            })
            .suffix(move |_, _| {
                let owner = new_workspace_owner.clone();
                let tooltip = format!("New Workspace on {new_workspace_label}…");
                Button::new(("new-workspace", key))
                    .debug_selector(move || format!("new-workspace-server-{key}"))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Plus)
                    .tooltip(tooltip)
                    .disabled(!connected)
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        let _ = owner.update(cx, |this, cx| {
                            this.choose_workspace_directory_on(key, window, cx)
                        });
                    })
            })
            .on_click(move |_, window, cx| {
                let _ = select_owner.update(cx, |this, cx| this.select_server(key, window, cx));
            })
        });

        let add_owner = cx.weak_entity();
        let reconnect_owner = cx.weak_entity();
        let reconnect_visible = self
            .active_connection()
            .is_some_and(|connection| connection.status == ConnectionStatus::Disconnected);
        let servers = CondrSidebarSection::new(
            "Servers",
            "servers-heading",
            move |_, _| {
                let owner = add_owner.clone();
                Button::new("add-server")
                    .debug_selector(|| "add-server".into())
                    .ghost()
                    .xsmall()
                    .icon(IconName::Plus)
                    .tooltip("Add Server")
                    .on_click(move |_, window, cx| {
                        let _ = owner.update(cx, |this, cx| this.prompt_add_server(window, cx));
                    })
                    .into_any_element()
            },
            items,
        );
        let settings_owner = cx.weak_entity();
        Sidebar::new("condr-sidebar")
            .collapsible(SidebarCollapsible::None)
            .w_full()
            .header(
                SidebarHeader::new()
                    .child(Icon::new(IconName::SquareTerminal))
                    .child(div().flex_1().font_semibold().child(condr_core::APP_NAME))
                    .child(
                        Button::new("open-settings")
                            .debug_selector(|| "open-settings".into())
                            .ghost()
                            .xsmall()
                            .icon(IconName::Settings2)
                            .tooltip("Settings")
                            .accessibility_label("Settings")
                            .on_click(move |_, window, cx| {
                                let _ = settings_owner
                                    .update(cx, |this, cx| this.open_settings(window, cx));
                            }),
                    ),
            )
            .child(servers)
            .when(reconnect_visible, |sidebar| {
                sidebar.footer(
                    SidebarFooter::new().child(
                        Button::new("reconnect-server")
                            .ghost()
                            .small()
                            .icon(IconName::LoaderCircle)
                            .tooltip("Reconnect")
                            .on_click(move |_, window, cx| {
                                let _ = reconnect_owner.update(cx, |this, cx| {
                                    this.reconnect_active(window, cx);
                                    cx.notify();
                                });
                            }),
                    ),
                )
            })
    }
}
