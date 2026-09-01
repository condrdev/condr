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

#[derive(Clone, Copy, PartialEq)]
pub(super) struct TerminalGeometry {
    pub(super) bounds: Bounds<Pixels>,
    pub(super) cell_size: Size<Pixels>,
}

#[derive(Clone, Copy)]
pub(super) struct LocalTerminalSelection {
    pub(super) connection_key: ConnectionKey,
    pub(super) pane_id: PaneId,
    pub(super) range: TerminalSelection,
    pub(super) dragging: bool,
}

#[derive(Clone, Copy)]
pub(super) struct ReportedTerminalMouse {
    pub(super) connection_key: ConnectionKey,
    pub(super) pane_id: PaneId,
    pub(super) button: TerminalMouseButton,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct ReportedTerminalMouseMotion {
    pub(super) connection_key: ConnectionKey,
    pub(super) pane_id: PaneId,
    pub(super) mouse_tracking: TerminalMouseTracking,
    pub(super) event: TerminalMouseEvent,
}

pub(super) struct TerminalPanel {
    connection_key: ConnectionKey,
    pane_id: PaneId,
    owner: WeakEntity<Condr>,
    pub(super) focus_handle: FocusHandle,
    pub(super) render_cache: Rc<RefCell<TerminalRenderCache>>,
    pub(super) scroll_remainder: Rc<RefCell<Point<f32>>>,
    focus_subscriptions: Vec<Subscription>,
    ime_terminal_revision: Option<u64>,
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
            .overflow_hidden()
    }

    fn split_frame(
        &self,
        node: gpui_component::dock::NodeId,
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
        _: &gpui_component::dock::TabGroupContext,
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
        _: &gpui_component::dock::TabGroupContext,
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
        _: &gpui_component::dock::TabGroupContext,
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
        _: &gpui_component::dock::TileContext,
        _: &mut Window,
        _: &mut App,
    ) -> AnyElement {
        Empty.into_any_element()
    }
}

impl TerminalPanel {
    fn new(
        connection_key: ConnectionKey,
        pane_id: PaneId,
        owner: WeakEntity<Condr>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            connection_key,
            pane_id,
            owner,
            focus_handle: cx.focus_handle(),
            render_cache: Rc::new(RefCell::new(TerminalRenderCache::default())),
            scroll_remainder: Rc::new(RefCell::new(point(0., 0.))),
            focus_subscriptions: Vec::new(),
            ime_terminal_revision: None,
        }
    }
}

impl EventEmitter<PanelEvent> for TerminalPanel {}

impl Focusable for TerminalPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for TerminalPanel {
    fn panel_name(&self) -> &'static str {
        "CondrTerminal"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Render for TerminalPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focus_subscriptions.is_empty() {
            let focus_owner = self.owner.clone();
            let blur_owner = self.owner.clone();
            // GPUI leases this Panel while it runs a focus callback, and sync_terminal_focus
            // reads every Panel, so the owner update has to wait for the lease to be released.
            self.focus_subscriptions = vec![
                cx.on_focus(&self.focus_handle, window, move |_, window, cx| {
                    let owner = focus_owner.clone();
                    window.defer(cx, move |window, cx| {
                        let _ = owner.update(cx, |app, cx| app.sync_terminal_focus(window, cx));
                    });
                }),
                cx.on_blur(&self.focus_handle, window, move |_, window, cx| {
                    let owner = blur_owner.clone();
                    window.defer(cx, move |window, cx| {
                        let _ = owner.update(cx, |app, cx| app.sync_terminal_focus(window, cx));
                    });
                }),
            ];
        }
        let owner = self.owner.upgrade();
        let (terminal, active, controlling, marked_text, selection, runtime_epoch, pane_title) =
            owner
                .as_ref()
                .map(|owner| {
                    let app = owner.read(cx);
                    let active = app.target_pane == Some((self.connection_key, self.pane_id));
                    let connection = app.connection(self.connection_key);
                    (
                        app.terminal(self.connection_key, self.pane_id).cloned(),
                        active,
                        connection.is_some_and(ServerConnection::can_mutate),
                        app.marked_text_for(self.connection_key, self.pane_id),
                        app.selection_for(self.connection_key, self.pane_id),
                        connection.and_then(|connection| connection.runtime_epoch),
                        connection
                            .and_then(|connection| connection.agents.get(&self.pane_id))
                            .map(|agent| agent.kind.label())
                            .unwrap_or("Terminal"),
                    )
                })
                .unwrap_or((None, false, false, None, None, None, "Terminal"));
        let ime_terminal_revision = marked_text
            .as_ref()
            .and_then(|_| terminal.as_ref().map(|terminal| terminal.view.revision));
        if self.ime_terminal_revision.is_some()
            && ime_terminal_revision.is_some()
            && self.ime_terminal_revision != ime_terminal_revision
        {
            window.invalidate_character_coordinates();
        }
        self.ime_terminal_revision = ime_terminal_revision;

        let key = self.connection_key;
        let pane_id = self.pane_id;
        let focus = self.focus_handle.clone();
        let click_owner = self.owner.clone();
        let body = div()
            .id(format!("terminal-pane-{key}-{}", pane_id.as_u64()))
            .debug_selector(move || format!("terminal-pane-{}", pane_id.as_u64()))
            .key_context("CondrTerminal")
            .track_focus(&self.focus_handle)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                let accepted = click_owner
                    .update(cx, |app, cx| app.select_pane(key, pane_id, window, cx))
                    .unwrap_or(false);
                if accepted {
                    focus.focus(window, cx);
                }
            })
            .w_full()
            .flex_1()
            .min_h(px(0.))
            .overflow_hidden()
            .font_family(cx.theme().mono_font_family.clone())
            .text_size(cx.theme().mono_font_size)
            .line_height(relative(1.35))
            .child(if let (Some(owner), Some(terminal)) = (owner, terminal) {
                TerminalElement::new(
                    owner,
                    TerminalElementProps {
                        focus_handle: self.focus_handle.clone(),
                        connection_key: key,
                        pane_id,
                        terminal: terminal.view,
                        marked_text,
                        selection,
                        runtime_epoch,
                        render_cache: self.render_cache.clone(),
                        scroll_remainder: self.scroll_remainder.clone(),
                    },
                )
                .into_any_element()
            } else {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Terminal unavailable")
                    .into_any_element()
            });

        let menu_owner = self.owner.clone();
        let pane_menu = Button::new(format!("terminal-pane-menu-{key}-{}", pane_id.as_u64()))
            .icon(IconName::EllipsisVertical)
            .xsmall()
            .ghost()
            .tab_stop(false)
            .tooltip("Pane actions")
            .accessibility_label("Pane actions")
            .debug_selector(move || format!("terminal-pane-menu-{}", pane_id.as_u64()))
            .dropdown_menu(move |menu, window, cx| {
                let _ = menu_owner.update(cx, |app, cx| {
                    app.set_target_pane(key, pane_id, cx);
                });
                menu.menu_with_enable("Split Right", Box::new(SplitRight), controlling)
                    .menu_with_enable("Split Down", Box::new(SplitDown), controlling)
                    .separator()
                    .submenu("Swap", window, cx, move |menu, _, _| {
                        menu.menu_with_enable("Left", Box::new(SwapLeft), controlling)
                            .menu_with_enable("Right", Box::new(SwapRight), controlling)
                            .menu_with_enable("Up", Box::new(SwapUp), controlling)
                            .menu_with_enable("Down", Box::new(SwapDown), controlling)
                    })
                    .separator()
                    .menu_with_enable("Toggle Zoom", Box::new(ToggleZoom), controlling)
                    .menu_with_enable("Close Pane", Box::new(ClosePane), controlling)
            })
            .anchor(Anchor::TopRight);

        v_flex()
            .size_full()
            .overflow_hidden()
            .border_3()
            .border_color(if active {
                rgb(ACTIVE_PANE_BORDER_RGB).into()
            } else {
                cx.theme().border
            })
            .child(
                h_flex()
                    .h(px(28.))
                    .w_full()
                    .flex_shrink_0()
                    .justify_between()
                    .gap_2()
                    .px_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .occlude()
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(pane_title),
                    )
                    .child(pane_menu),
            )
            .child(body)
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
                (self.workspace_size.height - WORKSPACE_TAB_BAR_HEIGHT).max(px(0.)),
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
                (window.viewport_size().width - INITIAL_SIDEBAR_WIDTH).max(px(1.)),
                (window.viewport_size().height - WORKSPACE_TAB_BAR_HEIGHT).max(px(1.)),
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
