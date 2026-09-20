use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct DockSurfaceKey {
    pub(super) connection_key: ConnectionKey,
    pub(super) tab_id: TabId,
}

pub(super) struct DockSurface {
    pub(super) area: Entity<DockArea>,
    pub(super) _subscription: Subscription,
    pub(super) pane_ids: HashSet<PaneId>,
    pub(super) projection: Option<PaneLayout>,
    pub(super) programmatic_layout_events: usize,
    pub(super) pending_projection_request: Option<u64>,
    pub(super) pending_projection_applied_sequence: Option<u64>,
    #[cfg(feature = "test-support")]
    pub(super) layout_size: Option<Size<Pixels>>,
}

impl Condr {}

/// A Pane header under drag (ADR 0005 layout, mouse form): the Kit Dock carries it as
/// an opaque `AnyDrag`, resolves the drop zone and paints the placeholder, and reports
/// the landing group and edge; only Condr turns that into a Layout command.
pub(super) struct DraggedPane {
    pub(super) key: ConnectionKey,
    pub(super) pane_id: PaneId,
}

pub(super) struct CondrDockRenderer;

impl DockAreaRenderer for CondrDockRenderer {
    fn frame(&self, _: &mut Window, _: &mut App) -> Stateful<Div> {
        div()
            .id("condr-dock-area")
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_row()
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

    /// The Panes draw their own borders; the Kit's one-pixel divider sits on the seam
    /// and paints over one pixel of them, so it only shows while a drag is under way.
    fn render_split_handle(
        &self,
        handle: &gpui_kit::base::ResizeHandleContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        Some(
            div()
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

    fn tiles_renderer(&self) -> Rc<dyn TilesRenderer> {
        Rc::new(CondrTilesRenderer)
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

pub(super) struct CondrTilesRenderer;

impl TilesRenderer for CondrTilesRenderer {
    fn frame(&self, _: &mut Window, _: &mut App) -> Stateful<Div> {
        div().id("condr-tiles").size_full().overflow_hidden()
    }

    fn render_drag_bar(
        &self,
        _: &gpui_kit::component::dock::TileContext,
        _: &mut Window,
        _: &mut App,
    ) -> AnyElement {
        Empty.into_any_element()
    }
}

impl Condr {
    pub(super) fn focused_pane(&self) -> Option<PaneId> {
        self.target_pane
            .filter(|(key, _)| *key == self.active_connection)
            .map(|(_, pane_id)| pane_id)
            .or_else(|| self.connection_focused_pane(self.active_connection))
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
        else {
            return;
        };
        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let should_focus = owner
                .update(cx, |this, _| {
                    this.active_connection == key
                        && this.target_pane == Some((key, pane_id))
                        && this.active_dock_surface == Some(surface_key)
                })
                .unwrap_or(false);
            if should_focus && !window.has_active_dialog(cx) {
                focus.focus(window, cx);
            }
        });
    }

    pub(super) fn rebuild_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.diff_editors.is_empty() {
            self.prune_diff_editors();
        }
        if !self.file_editors.is_empty() {
            self.prune_file_editors();
        }
        let Some(connection) = self.active_connection() else {
            self.active_dock_surface = None;
            return;
        };
        let Ok(session) = Session::restore(connection.snapshot.clone()) else {
            self.active_dock_surface = None;
            return;
        };
        let key = connection.key;
        let controlling = connection.can_mutate();
        let Some((_workspace_id, tab_id)) = connection.viewed(&session) else {
            self.target_pane = None;
            self.active_dock_surface = None;
            return;
        };
        let tab = session
            .tab(tab_id)
            .expect("presented Tab belongs to the restored Session");
        // A viewer Tab has no Panes and no Dock (ADR 0017); the body renders it directly.
        let Some(tab_layout) = tab.layout().cloned() else {
            self.target_pane = None;
            self.active_dock_surface = None;
            if let Some(diff) = tab.diff() {
                let path = diff.path().to_relative_path_buf();
                self.sync_diff_view(key, _workspace_id, tab_id, path, window, cx);
            } else if let Some(file) = tab.file() {
                let path = file.path().to_relative_path_buf();
                self.sync_file_view(key, _workspace_id, tab_id, path, window, cx);
            }
            return;
        };
        // The Pane the user targeted, while it is in this Tab; else the Tab's own focus.
        let focused = self
            .target_pane
            .filter(|(target_key, _)| *target_key == key)
            .map(|(_, pane_id)| pane_id)
            .filter(|pane_id| tab.panes().iter().any(|pane| pane.id() == *pane_id))
            .unwrap_or_else(|| {
                tab.focused_pane()
                    .expect("a terminal Tab has a focused Pane")
                    .id()
            });
        let authoritative_layout = connection
            .zoomed_panes
            .iter()
            .copied()
            .find(|pane_id| tab.panes().iter().any(|pane| pane.id() == *pane_id))
            .map(PaneLayout::Pane)
            .unwrap_or(tab_layout);
        // Where "Insert Path into Terminal" goes once a viewer Tab takes the active slot.
        self.last_terminal_tabs.insert((key, _workspace_id), tab_id);
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
        let active_size = self
            .active_dock_surface
            .and_then(|active| self.dock_surfaces.get(&active))
            .map(|surface| surface.area.read(cx).bounds().size)
            .filter(|size| size.width > px(0.) && size.height > px(0.));
        let measured_size = {
            let size = size(
                self.workspace_size.width.max(px(0.)),
                self.workspace_size.height.max(px(0.)),
            );
            (size.width > px(0.) && size.height > px(0.)).then_some(size)
        };
        let available_size = if self.active_dock_surface == Some(surface_key) {
            target_size.or(measured_size).or(active_size)
        } else {
            active_size.or(measured_size).or(target_size)
        }
        .unwrap_or_else(|| {
            size(
                (window.viewport_size().width - self.sidebar_layout_width()).max(px(1.)),
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
            let subscription = cx.subscribe_in(
                &area,
                window,
                move |this, dock, event: &DockEvent, window, cx| match event {
                    DockEvent::LayoutChanged => {
                        this.on_dock_layout_changed(surface_key, dock, window, cx);
                    }
                    DockEvent::DragDrop { item, target } => {
                        this.on_dock_drag_drop(surface_key, dock, item, target, cx);
                    }
                },
            );
            DockSurface {
                area,
                _subscription: subscription,
                pane_ids: HashSet::new(),
                projection: None,
                programmatic_layout_events: 0,
                pending_projection_request: None,
                pending_projection_applied_sequence: None,
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

        self.target_pane = Some((key, focused));
        self.active_dock_surface = Some(surface_key);
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
                // Locked, the Kit installs no drop handling, so a viewer sees no drop zones;
                // Condr renders no tab bar, so unlocked adds no Kit-driven rearranging.
                dock.set_locked(!controlling, window, cx);
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
        self.focus_pane_panel(surface_key, focused, window, cx);
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
        let Some(dragged) = item.value().downcast_ref::<DraggedPane>() else {
            return;
        };
        let DockDropTarget::Group { node, placement } = target else {
            return;
        };
        let key = surface_key.connection_key;
        if dragged.key != key {
            return;
        }
        let Some(target_pane) = self.dock_group_pane(key, dock, *node, cx) else {
            return;
        };
        let pane_id = dragged.pane_id;
        let same_tab = self.dock_surfaces.get(&surface_key).is_some_and(|surface| {
            surface.pane_ids.contains(&pane_id) && surface.pane_ids.contains(&target_pane)
        });
        if target_pane == pane_id || !same_tab {
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
                side: match placement {
                    gpui_kit::base::Placement::Left => PaneDirection::Left,
                    gpui_kit::base::Placement::Right => PaneDirection::Right,
                    gpui_kit::base::Placement::Top => PaneDirection::Up,
                    gpui_kit::base::Placement::Bottom => PaneDirection::Down,
                },
            },
        };
        self.send_layout_to(key, command);
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
        self.panels
            .iter()
            .find(|((panel_key, _), entity)| {
                *panel_key == key && PanelId::from(entity.entity_id()) == panel
            })
            .map(|((_, pane_id), _)| *pane_id)
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
        if self.active_dock_surface != Some(surface_key) {
            return;
        }
        let Some(connection) = self.connection(surface_key.connection_key) else {
            return;
        };
        let Ok(mut session) = Session::restore(connection.snapshot.clone()) else {
            return;
        };
        let Some(tab) = session.tab(surface_key.tab_id) else {
            return;
        };
        let Some(layout) = tab.layout().cloned() else {
            return;
        };
        let zoomed = connection
            .zoomed_panes
            .iter()
            .any(|pane_id| tab.panes().iter().any(|pane| pane.id() == *pane_id));
        if zoomed {
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
                        if this.active_dock_surface == Some(surface_key) {
                            this.rebuild_dock(window, cx);
                            cx.notify();
                        }
                    });
                });
            }
        }
    }
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
