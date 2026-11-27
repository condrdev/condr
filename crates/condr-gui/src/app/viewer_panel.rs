//! A viewer Pane as a GPUI Dock panel (ADR 0078): the Diff, the Preview or Git Graph in a
//! Pane's card, its header taking the Pane's click, drag, zoom and menu as a terminal's does.

use super::dock::{DraggedPane, PANE_RADIUS};
use super::terminal_panel::{INNER_RADIUS, pane_controls};
use super::viewer_header::pane_header_bar;
use super::*;
use condr_core::PaneContent;
use rust_i18n::t;

pub(super) struct ViewerPanel {
    connection_key: ConnectionKey,
    pane_id: PaneId,
    owner: WeakEntity<Condr>,
    pub(super) focus_handle: FocusHandle,
}

impl ViewerPanel {
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
        }
    }
}

impl EventEmitter<PanelEvent> for ViewerPanel {}

impl Focusable for ViewerPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for ViewerPanel {
    fn panel_name(&self) -> &'static str {
        "CondrViewer"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Render for ViewerPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (key, pane_id) = (self.connection_key, self.pane_id);
        let focus = self.focus_handle.clone();
        // The viewers draw from Condr's state; a Dock panel renders after Condr has, so
        // Condr is free to update here.
        let card = self
            .owner
            .update(cx, |app, cx| {
                app.render_viewer_pane(key, pane_id, &focus, window, cx)
            })
            .unwrap_or_else(|_| div().into_any_element());
        div().size_full().track_focus(&focus).child(card)
    }
}

impl Condr {
    /// A viewer Pane's card: its header, with the Pane's controls after the viewer's own,
    /// above what it shows.
    fn render_viewer_pane(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
        focus: &FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some((connection, session)) = self
            .connection(key)
            .and_then(|connection| Some((connection, connection.session()?)))
        else {
            return div().into_any_element();
        };
        let (Some(workspace), Some(pane)) =
            (session.workspace_for_pane(pane_id), session.pane(pane_id))
        else {
            return div().into_any_element();
        };
        let workspace_id = workspace.id();
        let content = pane.content().clone();
        let can_mutate = connection.can_mutate();
        let zoomed = connection.model.zoomed_panes.contains(&pane_id);
        let active = self.terminal_input.target == Some((key, pane_id));
        // As a terminal's: a Pane alone in its Tab is not outlined (see `TerminalPanel`).
        let solo = !zoomed
            && self.dock_surfaces.iter().any(|(surface_key, surface)| {
                surface_key.connection_key == key
                    && surface.pane_ids.len() == 1
                    && surface.pane_ids.contains(&pane_id)
            });
        let muted = cx.theme().muted_foreground;
        let titled = |selector, icon: Glyph, title: SharedString, cx: &App| {
            pane_header_bar(selector, cx)
                .child(Icon::new(icon).size_4().flex_shrink_0().text_color(muted))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .text_sm()
                        .font_medium()
                        .child(title),
                )
        };
        let (header, body, title): (Div, AnyElement, SharedString) = match &content {
            PaneContent::Viewer(Viewer::Diff { path }) => {
                let (header, body) = self.render_diff_pane(key, workspace_id, pane_id, path, cx);
                (header, body, path.to_string().into())
            }
            PaneContent::Viewer(Viewer::File { path }) => {
                let view = self.window_view(window);
                let (header, body) =
                    self.render_preview_pane(view, key, workspace_id, pane_id, path, cx);
                (header, body, path.to_string().into())
            }
            PaneContent::Viewer(Viewer::GitGraph) => {
                let title = SharedString::from(ViewerKind::GitGraph.name());
                (
                    titled("git-graph-header", Glyph::GitGraph, title.clone(), cx),
                    self.render_git_graph(key, workspace_id, pane_id, cx),
                    title,
                )
            }
            PaneContent::Unsupported | PaneContent::Terminal { .. } => {
                let title = SharedString::from("?");
                (
                    titled(
                        "unsupported-pane-header",
                        Glyph::CircleAlert,
                        title.clone(),
                        cx,
                    ),
                    self.render_unsupported_pane(key, cx),
                    title,
                )
            }
        };
        let owner = cx.weak_entity();
        let (zoom, menu) = pane_controls(&owner, key, pane_id, zoomed, can_mutate, false);
        let header = header
            .id(SharedString::from(format!(
                "viewer-pane-header-{key}-{}",
                pane_id.as_u64()
            )))
            .debug_selector(move || format!("viewer-pane-header-{}", pane_id.as_u64()))
            .rounded_t(INNER_RADIUS)
            .occlude()
            .on_click({
                let owner = owner.clone();
                let focus = focus.clone();
                move |_, window, cx| {
                    let accepted = owner
                        .update(cx, |app, cx| app.select_pane(key, pane_id, window, cx))
                        .unwrap_or(false);
                    if accepted {
                        focus.focus(window, cx);
                    }
                }
            })
            // Dragged onto another Pane, or onto the Tab strip (ADR 0078).
            .when(can_mutate && !zoomed, |this| {
                this.on_drag(
                    AnyDrag::new(DraggedPane { key, pane_id }),
                    move |_, offset, _, cx| {
                        cx.stop_propagation();
                        let name = title.clone();
                        cx.new(|_| super::left_sidebar::DragPreview {
                            icon: None,
                            name,
                            offset,
                        })
                    },
                )
            })
            .child(h_flex().flex_none().gap_1().child(zoom).child(menu));
        let theme = cx.theme();
        v_flex()
            .size_full()
            .overflow_hidden()
            .rounded(PANE_RADIUS)
            .border_1()
            .border_color(if active && !solo {
                theme.primary
            } else {
                theme.border
            })
            .bg(theme.background)
            // A press anywhere in it makes it the Pane the window's Pane actions act on.
            .capture_any_mouse_down(move |_, _, cx| {
                let _ = owner.update(cx, |app, cx| app.set_target_pane(key, pane_id, cx));
            })
            .child(header)
            .child(body)
            .into_any_element()
    }

    /// A Pane of a kind this build does not know (ADR 0078): what it needs.
    fn render_unsupported_pane(&self, key: ConnectionKey, cx: &mut Context<Self>) -> AnyElement {
        let server = self
            .connection(key)
            .and_then(|connection| connection.server_build.clone())
            .unwrap_or_default();
        v_flex()
            .debug_selector(|| "unsupported-pane".into())
            .flex_1()
            .min_h_0()
            .w_full()
            .items_center()
            .justify_center()
            .gap_1()
            .p_4()
            .child(div().text_sm().child(t!("unsupported_pane.title")))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!(
                        "unsupported_pane.builds",
                        this = condr_core::build_identity(),
                        server = server
                    )),
            )
            .into_any_element()
    }
}
