//! A terminal Pane as a GPUI Dock panel.

use super::*;

pub(super) struct TerminalPanel {
    connection_key: ConnectionKey,
    pane_id: PaneId,
    owner: WeakEntity<Condr>,
    pub(super) focus_handle: FocusHandle,
    pub(super) render_cache: Rc<RefCell<TerminalRenderCache>>,
    pub(super) scroll_remainder: Rc<RefCell<Point<f32>>>,
    focus_subscriptions: Vec<Subscription>,
    ime_terminal_revision: Option<u64>,
    cursor_blink_hidden: bool,
    /// Runs only while the focused cursor asks to blink; dropping it stops the blink.
    _cursor_blink: Option<Task<()>>,
}

/// Half a blink period, matching the common host-terminal cadence.
const CURSOR_BLINK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

impl TerminalPanel {
    pub(super) fn new(
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
            cursor_blink_hidden: false,
            _cursor_blink: None,
        }
    }

    /// Shows the cursor and restarts the phase, so input never lands on a hidden cursor.
    fn restart_cursor_blink(&mut self, cx: &mut Context<Self>) {
        self.cursor_blink_hidden = false;
        self._cursor_blink = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CURSOR_BLINK_INTERVAL).await;
                let toggled = this.update(cx, |this, cx| {
                    this.cursor_blink_hidden = !this.cursor_blink_hidden;
                    cx.notify();
                });
                if toggled.is_err() {
                    break;
                }
            }
        }));
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
        let (
            terminal,
            active,
            solo,
            controlling,
            marked_text,
            selection,
            hovered_link,
            runtime_epoch,
            pane_title,
            agent_kind,
            attention,
        ) = owner
            .as_ref()
            .map(|owner| {
                let app = owner.read(cx);
                let active = app.target_pane == Some((self.connection_key, self.pane_id));
                let solo = app.dock_surfaces.iter().any(|(surface_key, surface)| {
                    surface_key.connection_key == self.connection_key
                        && surface.pane_ids.len() == 1
                        && surface.pane_ids.contains(&self.pane_id)
                });
                let connection = app.connection(self.connection_key);
                let terminal = app.terminal(self.connection_key, self.pane_id).cloned();
                let pane_title = connection
                    .and_then(|connection| connection.terminal_titles.get(&self.pane_id).cloned())
                    .map(SharedString::from)
                    .or_else(|| {
                        connection
                            .and_then(|connection| connection.agents.get(&self.pane_id))
                            .map(|agent| SharedString::from(agent.kind.label()))
                    })
                    .unwrap_or_else(|| SharedString::from("Terminal"));
                (
                    terminal,
                    active,
                    solo,
                    connection.is_some_and(ServerConnection::can_mutate),
                    app.marked_text_for(self.connection_key, self.pane_id),
                    app.selection_for(self.connection_key, self.pane_id),
                    app.hovered_link_for(self.connection_key, self.pane_id),
                    connection.and_then(|connection| connection.runtime_epoch),
                    pane_title,
                    connection
                        .and_then(|connection| connection.agents.get(&self.pane_id))
                        .map(|agent| agent.kind),
                    connection.is_some_and(|connection| {
                        connection.controlling && connection.attention.contains(&self.pane_id)
                    }),
                )
            })
            .unwrap_or((
                None,
                false,
                false,
                false,
                None,
                None,
                None,
                None,
                SharedString::from("Terminal"),
                None,
                false,
            ));
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

        // Like herdr forwarding DECSCUSR to its host terminal, blink only when the
        // program asked for it, and only in the focused Pane.
        let blinking = terminal
            .as_ref()
            .and_then(|terminal| terminal.view.cursor)
            .is_some_and(|cursor| {
                cursor.blinking && cursor.shape != condr_core::TerminalCursorShape::Hidden
            })
            && self.focus_handle.is_focused(window);
        if !blinking {
            self._cursor_blink = None;
            self.cursor_blink_hidden = false;
        } else if self._cursor_blink.is_none() {
            self.restart_cursor_blink(cx);
        }

        let key = self.connection_key;
        let pane_id = self.pane_id;
        let link_tooltip = hovered_link
            .as_ref()
            .map(|link| SharedString::from(link.uri.as_str()));
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
            // The gap between border and grid belongs to the terminal, so it is
            // painted in the terminal's background rather than the pane chrome.
            .p(px(5.))
            .bg(cx
                .try_global::<TerminalPalette>()
                .cloned()
                .unwrap_or_default()
                .background)
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
                        hovered_link,
                        runtime_epoch,
                        cursor_blink_hidden: self.cursor_blink_hidden,
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
            })
            .when_some(link_tooltip, |this, uri| {
                this.tooltip(move |window, cx| Tooltip::new(uri.clone()).build(window, cx))
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
        let title_tooltip = pane_title.clone();

        v_flex()
            .size_full()
            .overflow_hidden()
            .when(!solo, |this| {
                this.border_3().border_color(if active {
                    rgb(ACTIVE_PANE_BORDER_RGB).into()
                } else {
                    cx.theme().border
                })
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
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_1()
                            .when(attention, |header| {
                                header.child(
                                    div()
                                        .id(format!(
                                            "terminal-pane-bell-{key}-{}",
                                            pane_id.as_u64()
                                        ))
                                        .size_4()
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .tooltip(|window, cx| {
                                            Tooltip::new("Terminal bell").build(window, cx)
                                        })
                                        .child(
                                            super::sidebar::SidebarGlyph::CircleAlert
                                                .icon()
                                                .xsmall()
                                                .text_color(cx.theme().warning),
                                        ),
                                )
                            })
                            // The agent's mark sits before its Pane title, so a row of
                            // Panes reads by agent before it reads by text.
                            .when_some(agent_kind, |this, kind| {
                                this.child(
                                    super::sidebar::agent_mark(kind, cx.theme().muted_foreground)
                                        .xsmall(),
                                )
                            })
                            .child(
                                div()
                                    .id(format!("terminal-pane-title-{key}-{}", pane_id.as_u64()))
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(title_tooltip.clone()).build(window, cx)
                                    })
                                    .child(pane_title),
                            ),
                    )
                    .child(pane_menu),
            )
            .child(body)
    }
}
impl Condr {
    /// Input makes a blinking cursor visible again and restarts its phase.
    pub(super) fn restart_cursor_blink(&self, key: ConnectionKey, pane_id: PaneId, cx: &mut App) {
        if let Some(panel) = self.panels.get(&(key, pane_id)) {
            panel.update(cx, |panel, cx| {
                if panel._cursor_blink.is_some() {
                    panel.restart_cursor_blink(cx);
                    cx.notify();
                }
            });
        }
    }
}
