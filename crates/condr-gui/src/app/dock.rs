use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct DockSurfaceKey {
    pub(super) connection_key: ConnectionKey,
    pub(super) tab_id: TabId,
}

pub(super) struct DockSurface {
    pub(super) area: Entity<DockArea>,
    /// The window `_subscription` delivers to; a surface that moves to another window
    /// subscribes again there (ADR 0043).
    pub(super) window: AnyWindowHandle,
    pub(super) _subscription: Subscription,
    pub(super) pane_ids: HashSet<PaneId>,
    pub(super) projection: Option<PaneLayout>,
    pub(super) programmatic_layout_events: usize,
    pub(super) pending_projection_request: Option<u64>,
    pub(super) pending_projection_applied_sequence: Option<u64>,
    /// The Pane this surface last gave the keyboard to. `terminal_input.target` already
    /// names the new Pane by the time the Dock syncs, so it cannot tell that focus moved.
    pub(super) keyboard_pane: Option<PaneId>,
    #[cfg(feature = "test-support")]
    pub(super) layout_size: Option<Size<Pixels>>,
}

/// A Pane header under drag (ADR 0005 layout, mouse form): the Kit Dock carries it as
/// an opaque `AnyDrag`, resolves the drop zone and paints the placeholder, and reports
/// the landing group and edge; only Condr turns that into a Layout command.
pub(super) struct DraggedPane {
    pub(super) key: ConnectionKey,
    pub(super) pane_id: PaneId,
}

/// Half the 8px gap around and between the Panes' cards, in the colour of the bars around
/// them. The Kit's split handles overlay the seam rather than take room, so the area and
/// each Pane's slot both keep half of it.
const HALF_GAP: Pixels = px(4.);
/// A Pane card's corner radius.
pub(super) const PANE_RADIUS: Pixels = px(8.);

pub(super) struct CondrDockRenderer;

impl DockAreaRenderer for CondrDockRenderer {
    fn frame(&self, _: &mut Window, cx: &mut App) -> Stateful<Div> {
        div()
            .id("condr-dock-area")
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_row()
            .p(HALF_GAP)
            .bg(cx.theme().sidebar)
    }

    fn center_frame(&self, _: &mut Window, _: &mut App) -> Stateful<Div> {
        div()
            .id("condr-dock-center")
            .flex()
            .flex_1()
            .flex_col()
            .min_w(px(0.))
            .overflow_hidden()
    }

    fn split_frame(
        &self,
        node: gpui_kit::component::dock::NodeId,
        _: Axis,
        _: &mut Window,
        _: &mut App,
    ) -> Stateful<Div> {
        // `min_w` and `min_h` at zero: a flex item's minimum defaults to its content, and
        // a Pane whose terminal still has last frame's columns would widen the split past
        // its slot, which the Kit's stack then records as its container size.
        div()
            .id(("condr-dock-split", node.as_u64()))
            .size_full()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_hidden()
    }

    /// The Kit straddles the handle on the seam between two slots, the middle of the gap
    /// between their cards, and gives what is drawn here a one-pixel slot there: a line
    /// that shows only while a drag is under way.
    fn render_split_handle(
        &self,
        handle: &gpui_kit::base::ResizeHandleContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        Some(
            div()
                .debug_selector(|| "pane-split-handle".into())
                .flex_none()
                .map(|this| match handle.axis() {
                    Axis::Horizontal => this.h_full().w(px(1.)),
                    Axis::Vertical => this.w_full().h(px(1.)),
                })
                .when(handle.is_active(), |this| this.bg(cx.theme().primary))
                .into_any_element(),
        )
    }

    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(CondrTabGroupRenderer)
    }
}

pub(super) struct CondrTabGroupRenderer;

impl TabGroupRenderer for CondrTabGroupRenderer {
    fn frame(
        &self,
        _: &gpui_kit::component::dock::TabGroupContext,
        _: &mut Window,
        _: &mut App,
    ) -> Stateful<Div> {
        div()
            .id("condr-tab-group")
            .p(HALF_GAP)
            .size_full()
            .flex()
            .flex_col()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_hidden()
    }

    fn content_frame(
        &self,
        _: &gpui_kit::component::dock::TabGroupContext,
        _: &mut Window,
        _: &mut App,
    ) -> Stateful<Div> {
        div()
            .id("condr-tab-content")
            .size_full()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            // The drop placeholder below is positioned inside this frame.
            .relative()
            .overflow_hidden()
    }

    /// The placeholder while a Pane header hovers this group: base picks the half it
    /// covers (or the whole group for a swap), springs carry it from one zone to the
    /// next. Base draws nothing itself and the Kit skin is crate-private, so this is
    /// the Kit's look, redrawn.
    fn render_drop_indicator(
        &self,
        indicator: gpui_kit::component::dock::DropIndicator,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let to = indicator.to();
        let id = "condr-drop-placeholder";
        let motion = cx.theme().motion_tokens().spring_move.with_epsilon(0.5);
        let left = gpui_kit::base::spring((id, "left"), to.origin().x, motion, window, cx);
        let top = gpui_kit::base::spring((id, "top"), to.origin().y, motion, window, cx);
        let width = gpui_kit::base::spring((id, "width"), to.size().width, motion, window, cx);
        let height = gpui_kit::base::spring((id, "height"), to.size().height, motion, window, cx);
        Some(
            div()
                .debug_selector(|| id.into())
                .absolute()
                .left(left)
                .top(top)
                .w(width)
                .h(height)
                .bg(cx.theme().tokens.drop_target)
                .into_any_element(),
        )
    }

    fn render_tab_bar(
        &self,
        _: &gpui_kit::component::dock::TabGroupContext,
        _: &mut Window,
        _: &mut App,
    ) -> AnyElement {
        Empty.into_any_element()
    }
}

impl Condr {
    /// The Pane the active window's actions apply to, on `input_connection`.
    pub(super) fn focused_pane(&self) -> Option<PaneId> {
        let key = self.input_connection();
        self.terminal_input
            .target
            .filter(|(target_key, _)| *target_key == key)
            .map(|(_, pane_id)| pane_id)
            .or_else(|| self.view_focused_pane(self.active_view()))
    }

    /// The connection the active window presents: where its actions go.
    pub(super) fn input_connection(&self) -> ConnectionKey {
        self.presented_key(self.active_view())
    }

    /// The Session's focus in the terminal Tab `view` draws.
    pub(super) fn view_focused_pane(&self, view: &WindowView) -> Option<PaneId> {
        if self.shown_elsewhere(view).is_some() {
            return None;
        }
        let (_, session, _, tab_id) = self.presented_in(view)?;
        Some(session.tab(tab_id)?.focused_pane()?.id())
    }

    pub(super) fn focus_pane_panel(
        &self,
        surface_key: DockSurfaceKey,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = surface_key.connection_key;
        let Some(focus) = self
            .panels
            .get(&(key, pane_id))
            .map(|panel| panel.read(cx).focus_handle.clone())
            .or_else(|| {
                self.viewer_panels
                    .get(&(key, pane_id))
                    .map(|panel| panel.read(cx).focus_handle.clone())
            })
        else {
            return;
        };
        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let should_focus = owner
                .update(cx, |this, _| {
                    this.owns_window(window)
                        && this.window_view(window).dock_surface == Some(surface_key)
                        && (this.last_active != window.window_handle()
                            || this.terminal_input.target == Some((key, pane_id)))
                })
                .unwrap_or(false);
            if should_focus && !window.has_active_dialog(cx) {
                focus.focus(window, cx);
            }
        });
    }

    /// Prepares the Dock surface `window` draws and, while it is the window last
    /// activated, points the keyboard at its Pane.
    pub(super) fn rebuild_window_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Every view change ends here (ADR 0021), so this is where the state file learns
        // of it; `show_view` has no context to schedule from.
        if std::mem::take(&mut self.state_dirty) {
            self.schedule_state_save(cx);
        }
        let handle = window.window_handle();
        let input_window = self.last_active == handle;
        // Tasks mode shows no Tab: only the selected Agent's Pane (ADR 0068).
        if self.in_tasks_mode(self.window_view(window)) {
            self.rebuild_tasks_dock(window, cx);
            return;
        }
        // What this window draws may be the Pane the Tasks window's area shows, which
        // yields to it, or may have stopped being it (ADR 0069).
        if let Some(tasks) = self.window_for(WindowScope::Tasks) {
            self.in_window(tasks, cx, |this, window, cx| {
                let shown =
                    |this: &Self| this.tasks_view().tasks_dock.as_ref().map(|dock| dock.pane);
                let before = shown(this);
                this.rebuild_tasks_dock(window, cx);
                if shown(this) != before {
                    cx.notify();
                }
            });
        }
        let view = self.window_view(window);
        // The main window's page saying Tasks is in its own window draws no Tab either.
        let shown = if self.shown_elsewhere(view).is_some() || view.page == ContentPage::Tasks {
            None
        } else {
            let key = self.presented_key(view);
            let scope = view.scope;
            self.connection(key).and_then(|connection| {
                let session = connection.session()?;
                let (workspace_id, tab_id) = match scope {
                    WindowScope::Main | WindowScope::Device(_) => connection.viewed(session)?,
                    WindowScope::Workspace(_, workspace_id) => (
                        workspace_id,
                        connection.viewed_tab_id(session, workspace_id)?,
                    ),
                    WindowScope::Tab(_, tab_id) => (workspace_of_tab(session, tab_id)?, tab_id),
                    WindowScope::Tasks => return None,
                };
                Some((key, workspace_id, tab_id))
            })
        };
        let Some((key, _workspace_id, tab_id)) = shown else {
            if input_window {
                self.terminal_input.target = None;
            }
            self.window_view_mut(window).dock_surface = None;
            return;
        };
        let connection = self
            .connection(key)
            .expect("the presented connection exists");
        let session = connection.session().expect("the presented Session exists");
        let can_mutate = connection.can_mutate();
        let tab = session
            .tab(tab_id)
            .expect("presented Tab belongs to the restored Session");
        let tab_layout = tab.layout().cloned().expect("a Tab has a layout");
        // A viewer Pane is a Dock panel like a terminal's (ADR 0078); what it shows is
        // prepared here, as a terminal's arrives over the wire.
        let viewers: Vec<(PaneId, Viewer)> = tab
            .panes()
            .iter()
            .filter_map(|pane| Some((pane.id(), pane.viewer()?.clone())))
            .collect();
        // The Pane the user targeted, while it is in this Tab; else the Tab's own focus.
        let focused = self
            .terminal_input
            .target
            .filter(|_| input_window)
            .filter(|(target_key, _)| *target_key == key)
            .map(|(_, pane_id)| pane_id)
            .filter(|pane_id| tab.panes().iter().any(|pane| pane.id() == *pane_id))
            .unwrap_or_else(|| {
                tab.focused_pane()
                    .expect("a terminal Tab has a focused Pane")
                    .id()
            });
        let authoritative_layout = connection
            .zoomed_pane(tab)
            .map(PaneLayout::Pane)
            .unwrap_or(tab_layout);
        for (pane_id, viewer) in viewers {
            match viewer {
                Viewer::Diff { path } => self.sync_diff_view(key, _workspace_id, pane_id, path, cx),
                Viewer::File { path } => {
                    self.sync_file_view(key, _workspace_id, pane_id, path, window, cx)
                }
                Viewer::GitGraph => {
                    self.sync_git_graph_view(key, _workspace_id, pane_id, window, cx)
                }
            }
        }
        // Where "Insert Path into Terminal" looks for a terminal (ADR 0078).
        self.right_sidebar
            .last_presented_tabs
            .insert((key, _workspace_id), tab_id);
        let surface_key = DockSurfaceKey {
            connection_key: key,
            tab_id,
        };
        let layout = self
            .dock_surfaces
            .get(&surface_key)
            .filter(|surface| surface.pending_projection_request.is_some())
            .and_then(|surface| surface.projection.clone())
            .unwrap_or(authoritative_layout);
        let target_size = self
            .dock_surfaces
            .get(&surface_key)
            .map(|surface| surface.area.read(cx).bounds().size)
            .filter(|size| size.width > px(0.) && size.height > px(0.));
        let shown_surface = self.window_view(window).dock_surface;
        let active_size = shown_surface
            .and_then(|active| self.dock_surfaces.get(&active))
            .map(|surface| surface.area.read(cx).bounds().size)
            .filter(|size| size.width > px(0.) && size.height > px(0.));
        let measured_size = {
            let workspace_size = self.window_view(window).workspace_size;
            let size = size(
                workspace_size.width.max(px(0.)),
                workspace_size.height.max(px(0.)),
            );
            (size.width > px(0.) && size.height > px(0.)).then_some(size)
        };
        let available_size = if shown_surface == Some(surface_key) {
            target_size.or(measured_size).or(active_size)
        } else {
            active_size.or(measured_size).or(target_size)
        }
        .unwrap_or_else(|| {
            size(
                (window.viewport_size().width - self.window_view(window).sidebar_layout_width())
                    .max(px(1.)),
                (window.viewport_size().height - WORKSPACE_TITLE_BAR_HEIGHT).max(px(1.)),
            )
        });

        self.dock_surfaces.entry(surface_key).or_insert_with(|| {
            let area = cx.new(|cx| {
                DockArea::new(
                    format!("condr-workspace-{key}-{}", tab_id.as_u64()),
                    None,
                    window,
                    cx,
                )
                .with_renderer(Rc::new(CondrDockRenderer))
            });
            let subscription = subscribe_to_dock(surface_key, &area, window, cx);
            DockSurface {
                area,
                window: handle,
                _subscription: subscription,
                pane_ids: HashSet::new(),
                projection: None,
                programmatic_layout_events: 0,
                pending_projection_request: None,
                pending_projection_applied_sequence: None,
                keyboard_pane: None,
                #[cfg(feature = "test-support")]
                layout_size: None,
            }
        });
        {
            let surface = self
                .dock_surfaces
                .get_mut(&surface_key)
                .expect("Dock surface was installed");
            surface.pane_ids.clear();
            collect_layout_pane_ids(&layout, &mut surface.pane_ids);
        }

        if self.dock_surfaces[&surface_key].window != handle {
            let area = self.dock_surfaces[&surface_key].area.clone();
            let subscription = subscribe_to_dock(surface_key, &area, window, cx);
            let surface = self
                .dock_surfaces
                .get_mut(&surface_key)
                .expect("Dock surface was installed");
            surface.window = handle;
            surface._subscription = subscription;
        }

        // Focus follows the presented Pane, not every rebuild: one for a Git refresh must
        // leave an open context menu or a sidebar field holding the keyboard.
        let retargeted = (input_window
            && self.dock_surfaces[&surface_key].keyboard_pane != Some(focused))
            || shown_surface != Some(surface_key);
        if input_window {
            self.terminal_input.target = Some((key, focused));
        }
        self.window_view_mut(window).dock_surface = Some(surface_key);
        let needs_rebuild = self.dock_surfaces.get(&surface_key).is_none_or(|surface| {
            surface.projection.as_ref() != Some(&layout)
                || target_size.is_some_and(|target_size| {
                    (target_size.width - available_size.width).abs() > px(1.)
                        || (target_size.height - available_size.height).abs() > px(1.)
                })
        });
        if needs_rebuild {
            let dock_layout = self.build_dock_layout(key, &layout, available_size, cx);
            let area = {
                let surface = self
                    .dock_surfaces
                    .get_mut(&surface_key)
                    .expect("Dock surface was installed");
                surface.programmatic_layout_events =
                    surface.programmatic_layout_events.saturating_add(1);
                surface.area.clone()
            };
            area.update(cx, |dock, cx| {
                // Locked, the Kit installs no drop handling, so a Session still synchronizing
                // shows no drop zones; Condr renders no tab bar, so unlocked adds no
                // Kit-driven rearranging.
                dock.set_locked(!can_mutate, window, cx);
                dock.set_center(dock_layout, window, cx);
            });
            let surface = self
                .dock_surfaces
                .get_mut(&surface_key)
                .expect("Dock surface was installed");
            surface.projection = Some(layout);
            #[cfg(feature = "test-support")]
            {
                surface.layout_size = Some(available_size);
                self.dock_rebuild_count += 1;
            }
        }
        if retargeted || needs_rebuild {
            self.dock_surfaces
                .get_mut(&surface_key)
                .expect("Dock surface was installed")
                .keyboard_pane = Some(focused);
            self.focus_pane_panel(surface_key, focused, window, cx);
        }
    }

    pub(super) fn build_dock_layout(
        &mut self,
        key: ConnectionKey,
        layout: &PaneLayout,
        available_size: Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> DockLayout {
        match layout {
            PaneLayout::Pane(pane_id) => {
                let terminal = self
                    .connection(key)
                    .and_then(ServerConnection::session)
                    .and_then(|session| session.pane(*pane_id))
                    .is_none_or(condr_core::Pane::is_terminal);
                if !terminal {
                    let panel = self
                        .viewer_panels
                        .entry((key, *pane_id))
                        .or_insert_with(|| {
                            let owner = cx.weak_entity();
                            cx.new(|cx| {
                                super::viewer_panel::ViewerPanel::new(key, *pane_id, owner, cx)
                            })
                        })
                        .clone();
                    return DockLayout::tabs().panel(panel);
                }
                let panel = self
                    .panels
                    .entry((key, *pane_id))
                    .or_insert_with(|| {
                        let owner = cx.weak_entity();
                        cx.new(|cx| TerminalPanel::new(key, *pane_id, owner, cx))
                    })
                    .clone();
                DockLayout::tabs().panel(panel)
            }
            PaneLayout::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let extent = match direction {
                    SplitDirection::Horizontal => available_size.width,
                    SplitDirection::Vertical => available_size.height,
                };
                let extent = if extent > px(0.) { extent } else { px(1000.) };
                let first_extent = extent * *ratio;
                let second_extent = extent - first_extent;
                let (first_size, second_size) = match direction {
                    SplitDirection::Horizontal => (
                        size(first_extent, available_size.height),
                        size(second_extent, available_size.height),
                    ),
                    SplitDirection::Vertical => (
                        size(available_size.width, first_extent),
                        size(available_size.width, second_extent),
                    ),
                };
                let first = self.build_dock_layout(key, first, first_size, cx);
                let second = self.build_dock_layout(key, second, second_size, cx);
                let split = match direction {
                    SplitDirection::Horizontal => DockLayout::h_split(),
                    SplitDirection::Vertical => DockLayout::v_split(),
                };
                split
                    .child(first, Some(first_extent))
                    .child(second, Some(second_extent))
            }
        }
    }

    /// A Pane header dropped on another Pane: an edge splits that Pane and puts the
    /// dragged one on that side, the centre swaps the two. The Server owns the result;
    /// the Dock is rebuilt when it confirms, as after any Layout command.
    fn on_dock_drag_drop(
        &mut self,
        surface_key: DockSurfaceKey,
        dock: &Entity<DockArea>,
        item: &AnyDrag,
        target: &DockDropTarget,
        cx: &mut Context<Self>,
    ) {
        let node = target.node();
        let placement = target.placement();
        let key = surface_key.connection_key;
        let Some(target_pane) = self.dock_group_pane(key, dock, node, cx) else {
            return;
        };
        let side = |placement| match placement {
            gpui_kit::base::Placement::Left => PaneDirection::Left,
            gpui_kit::base::Placement::Right => PaneDirection::Right,
            gpui_kit::base::Placement::Top => PaneDirection::Up,
            gpui_kit::base::Placement::Bottom => PaneDirection::Down,
        };
        // Another Tab, dragged from the strip, merges beside the Pane; the shown Tab
        // dropped on its own Panes joins the Tab closing it would show, as IntelliJ's
        // editor tab dropped on its own edge does. The centre puts it to the right
        // (ADR 0078).
        if let Some(tab) = item.value().downcast_ref::<super::workspace::DraggedTab>() {
            if tab.key != key {
                return;
            }
            let side = placement.map_or(PaneDirection::Right, side);
            let command = if tab.tab_id != surface_key.tab_id {
                Some(LayoutCommand::MoveTabInto {
                    tab_id: tab.tab_id,
                    target_pane_id: target_pane,
                    side,
                })
            } else {
                self.tab_closing_shows(key, tab.workspace_id, tab.tab_id)
                    .map(|other_tab_id| LayoutCommand::MoveTabBeside {
                        tab_id: tab.tab_id,
                        other_tab_id,
                        side,
                    })
            };
            if let Some(command) = command {
                self.send_layout_to(key, command);
            }
            return;
        }
        let Some(dragged) = item.value().downcast_ref::<DraggedPane>() else {
            return;
        };
        if dragged.key != key {
            return;
        }
        let pane_id = dragged.pane_id;
        let same_tab = self.dock_surfaces.get(&surface_key).is_some_and(|surface| {
            surface.pane_ids.contains(&pane_id) && surface.pane_ids.contains(&target_pane)
        });
        // Swapping takes two Panes of one Tab; a Pane from another Tab only moves in.
        if target_pane == pane_id || (!same_tab && placement.is_none()) {
            return;
        }
        let command = match placement {
            None => LayoutCommand::SwapPanes {
                pane_id,
                other: target_pane,
            },
            Some(placement) => LayoutCommand::MovePane {
                pane_id,
                target_pane_id: target_pane,
                side: side(placement),
            },
        };
        self.send_layout_to(key, command);
    }

    /// The Tab shown once `tab_id` closes: the one after it, else the one before, as
    /// `ServerConnection` moves a View off a Tab that went.
    fn tab_closing_shows(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
    ) -> Option<TabId> {
        let tabs = self
            .connection(key)?
            .session()?
            .workspace(workspace_id)?
            .tabs();
        let ix = tabs.iter().position(|tab| tab.id() == tab_id)?;
        tabs.get(ix + 1)
            .or(ix.checked_sub(1).and_then(|before| tabs.get(before)))
            .map(Tab::id)
    }

    /// The Pane a Dock tab group shows: every group here holds exactly one Panel.
    fn dock_group_pane(
        &self,
        key: ConnectionKey,
        dock: &Entity<DockArea>,
        node: gpui_kit::component::dock::NodeId,
        cx: &App,
    ) -> Option<PaneId> {
        let dock = dock.read(cx);
        let node = dock.layout(DockPlacement::Center)?.find_node(node)?;
        let PaneRef::Tabs { panels, .. } = node.kind() else {
            return None;
        };
        let panel = *panels.first()?;
        let ids = self
            .panels
            .iter()
            .map(|(id, entity)| (*id, entity.entity_id()))
            .chain(
                self.viewer_panels
                    .iter()
                    .map(|(id, entity)| (*id, entity.entity_id())),
            );
        ids.into_iter()
            .find(|((panel_key, _), entity)| *panel_key == key && PanelId::from(*entity) == panel)
            .map(|((_, pane_id), _)| pane_id)
    }

    pub(super) fn on_dock_layout_changed(
        &mut self,
        surface_key: DockSurfaceKey,
        dock: &Entity<DockArea>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(surface) = self.dock_surfaces.get_mut(&surface_key) else {
            return;
        };
        if surface.programmatic_layout_events > 0 {
            surface.programmatic_layout_events -= 1;
            return;
        }
        if !self
            .windows
            .iter()
            .any(|view| view.dock_surface == Some(surface_key))
        {
            return;
        }
        let Some(connection) = self.connection(surface_key.connection_key) else {
            return;
        };
        let Some(mut session) = connection.session().cloned() else {
            return;
        };
        let Some(tab) = session.tab(surface_key.tab_id) else {
            return;
        };
        let Some(layout) = tab.layout().cloned() else {
            return;
        };
        if connection.zoomed_pane(tab).is_some() {
            return;
        }
        let mut ratios = Vec::new();
        collect_dock_ratios(&dock.read(cx).dump(cx).center, &mut ratios);
        let mut expected = Vec::new();
        collect_layout_ratios(&layout, &mut expected);
        if ratios.len() == expected.len()
            && ratios
                .iter()
                .zip(expected)
                .any(|(actual, expected)| (actual - expected).abs() > 0.001)
        {
            let projection = session
                .set_tab_split_ratios(surface_key.tab_id, &ratios)
                .then(|| {
                    session
                        .tab(surface_key.tab_id)
                        .expect("resized Tab remains in the Session")
                        .layout()
                        .expect("a resized Tab is a terminal Tab")
                        .clone()
                });
            let sent = self.send_layout_to(
                surface_key.connection_key,
                LayoutCommand::SetSplitRatios {
                    tab_id: surface_key.tab_id,
                    ratios,
                },
            );
            if let Some(request_id) = sent
                && let Some(projection) = projection
                && let Some(surface) = self.dock_surfaces.get_mut(&surface_key)
            {
                surface.projection = Some(projection);
                surface.pending_projection_request = Some(request_id);
                surface.pending_projection_applied_sequence = None;
            } else if sent.is_none() {
                if let Some(surface) = self.dock_surfaces.get_mut(&surface_key) {
                    surface.projection = None;
                }
                let owner = cx.weak_entity();
                window.defer(cx, move |window, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.rebuild_dock(window, cx);
                        cx.notify();
                    });
                });
            }
        }
    }
}

/// The Dock's events reach `Condr` through the window drawing it.
fn subscribe_to_dock(
    surface_key: DockSurfaceKey,
    area: &Entity<DockArea>,
    window: &mut Window,
    cx: &mut Context<Condr>,
) -> Subscription {
    cx.subscribe_in(
        area,
        window,
        move |this, dock, event: &DockEvent, window, cx| match event {
            DockEvent::LayoutChanged => {
                this.on_dock_layout_changed(surface_key, dock, window, cx);
            }
            DockEvent::DragDrop { item, target } => {
                this.on_dock_drag_drop(surface_key, dock, item, target, cx);
            }
        },
    )
}

fn collect_layout_ratios(layout: &PaneLayout, ratios: &mut Vec<f32>) {
    if let PaneLayout::Split {
        ratio,
        first,
        second,
        ..
    } = layout
    {
        ratios.push(*ratio);
        collect_layout_ratios(first, ratios);
        collect_layout_ratios(second, ratios);
    }
}

fn collect_layout_pane_ids(layout: &PaneLayout, pane_ids: &mut HashSet<PaneId>) {
    match layout {
        PaneLayout::Pane(pane_id) => {
            pane_ids.insert(*pane_id);
        }
        PaneLayout::Split { first, second, .. } => {
            collect_layout_pane_ids(first, pane_ids);
            collect_layout_pane_ids(second, pane_ids);
        }
    }
}

fn collect_dock_ratios(state: &PanelState, ratios: &mut Vec<f32>) {
    if let PanelInfo::Stack { sizes, .. } = &state.info
        && sizes.len() == 2
    {
        let total = sizes[0] + sizes[1];
        if total > px(0.) {
            ratios.push((sizes[0] / total).clamp(0.1, 0.9));
        }
    }
    for child in &state.children {
        collect_dock_ratios(child, ratios);
    }
}
