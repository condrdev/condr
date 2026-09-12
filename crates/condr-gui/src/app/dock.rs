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
            .overflow_hidden()
    }

    fn split_frame(
        &self,
        node: gpui_kit::component::dock::NodeId,
        _: Axis,
        _: &mut Window,
        _: &mut App,
    ) -> Stateful<Div> {
        div()
            .id(("condr-dock-split", node.as_u64()))
            .size_full()
            .flex_1()
            .min_h(px(0.))
            .overflow_hidden()
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
            .min_h(px(0.))
            .overflow_hidden()
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
            .or_else(|| {
                Some(
                    self.active_session()?
                        .active_workspace()?
                        .active_tab()
                        .focused_pane()
                        .id(),
                )
            })
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
        let Some(connection) = self.active_connection() else {
            self.active_dock_surface = None;
            return;
        };
        let Ok(session) = Session::restore(connection.snapshot.clone()) else {
            self.active_dock_surface = None;
            return;
        };
        let key = connection.key;
        let held_target = self
            .active_dock_surface
            .filter(|surface| surface.connection_key == key)
            .and_then(|surface| {
                Self::workspace_id_for_surface(&session, surface)
                    .map(|workspace_id| (workspace_id, surface.tab_id))
            });
        let authoritative_target = session
            .active_workspace()
            .map(|workspace| (workspace.id(), workspace.active_tab().id()));
        let Some((_workspace_id, tab_id)) = self
            .should_hold_active_surface()
            .then_some(held_target)
            .flatten()
            .or(authoritative_target)
        else {
            self.target_pane = None;
            self.active_dock_surface = None;
            return;
        };
        let tab = session
            .tab(tab_id)
            .expect("presented Tab belongs to the restored Session");
        let focused = self
            .pending_workspace_selection_for(key)
            .and_then(|pending| pending.pane_id)
            .filter(|pane_id| tab.panes().iter().any(|pane| pane.id() == *pane_id))
            .unwrap_or_else(|| tab.focused_pane().id());
        let authoritative_layout = connection
            .zoomed_panes
            .iter()
            .copied()
            .find(|pane_id| tab.panes().iter().any(|pane| pane.id() == *pane_id))
            .map(PaneLayout::Pane)
            .unwrap_or_else(|| tab.layout().clone());
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
                move |this, dock, event: &DockEvent, window, cx| {
                    if matches!(event, DockEvent::LayoutChanged) {
                        this.on_dock_layout_changed(surface_key, dock, window, cx);
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
                dock.set_locked(true, window, cx);
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
        let layout = tab.layout().clone();
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
