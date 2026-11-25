//! The Agents section at the bottom of the left sidebar (ADR 0067): every Agent the
//! window's tree lists, grouped by Workspace or by state and most urgent first or in the
//! tree's order (ADR 0074), each row sliding to where its state moves it (ADR 0075), in a
//! height its top-edge handle sets.

use super::*;
use crate::app::change_tree::row_highlight;
use crate::app::slide::SlideColumn;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::resizable::resize_handle;
use rust_i18n::t;

/// The drag payload of the Agents section's handle; the section tracks its moves.
#[derive(Clone)]
pub(in crate::app) struct DraggedAgents;

/// The section's heading, all of it that shows folded.
const HEADING_HEIGHT: Rems = rems(2.25);
/// Critically damped, so the section never bounces past its height: about 90% of the
/// way in 130 ms, settled by 250 ms.
const FOLD_SPRING: SpringConfig = SpringConfig::new(900., 60., 1.);
/// Settled within half a percent of the way, under two pixels on a tall section.
const FOLD_EPSILON: f32 = 0.005;

/// How a window's section groups and orders its Agents, from its heading's menu
/// (ADR 0074); the default is ADR 0067's, by Workspace and by urgency.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct AgentsOrder {
    /// Grouped under their states rather than their Workspaces.
    pub(in crate::app) by_state: bool,
    /// In the tree's order, only `Blocked` and `Error` ahead, rather than by urgency.
    pub(in crate::app) tree_order: bool,
}

/// One Agent the section lists: where it lives and how it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct AgentRow {
    pub(in crate::app) key: ConnectionKey,
    pub(in crate::app) workspace_id: WorkspaceId,
    pub(in crate::app) pane_id: PaneId,
    pub(in crate::app) status: SidebarStatusVisual,
    /// Its place in `URGENCY`; `rank_agents` sets it, and by state it is the row's group.
    pub(in crate::app) rank: usize,
}

/// By state, the rows sort by rank. By Workspace, each Workspace's Agents form a group,
/// the group with the most urgent Agent first and the most urgent first within it; in
/// the tree's order only `Blocked` and `Error` rank ahead of the rest. Ties keep Session
/// order, the rows coming in it. The selected Agent ranks as what it is now, as every
/// other does (ADR 0075).
pub(in crate::app) fn rank_agents(rows: &mut [AgentRow], order: AgentsOrder) {
    for row in rows.iter_mut() {
        row.rank = urgency(row.status);
    }
    if order.by_state {
        rows.sort_by_key(|row| row.rank);
        return;
    }
    let rank = |row: &AgentRow| {
        if order.tree_order {
            row.rank.min(NEEDS_YOU_RANKS)
        } else {
            row.rank
        }
    };
    // Each Workspace in Session order, with its most urgent rank.
    let mut groups: Vec<((ConnectionKey, WorkspaceId), usize)> = Vec::new();
    for row in rows.iter() {
        let workspace = (row.key, row.workspace_id);
        match groups.iter_mut().find(|(group, _)| *group == workspace) {
            Some((_, best)) => *best = (*best).min(rank(row)),
            None => groups.push((workspace, rank(row))),
        }
    }
    rows.sort_by_key(|row| {
        let (ix, best) = groups
            .iter()
            .enumerate()
            .find_map(|(ix, (group, best))| {
                (*group == (row.key, row.workspace_id)).then_some((ix, *best))
            })
            .expect("every row's Workspace is a group");
        (best, ix, rank(row))
    });
}

/// The section's height: a third of the window until its handle is dragged, and never
/// so tall that the tree above loses its room.
pub(in crate::app) fn agents_height(view: &WindowView) -> Pixels {
    let room = view.workspace_size.height;
    view.agents_height
        .unwrap_or(room / 3.)
        .min(room - MIN_AGENTS_HEIGHT * 2.)
        .max(MIN_AGENTS_HEIGHT)
}

/// The heading's menu: Group, then Order, which is greyed while grouping by state, where
/// every order is Session order (ADR 0074).
fn agents_order_menu(menu: PopupMenu, order: AgentsOrder, owner: WeakEntity<Condr>) -> PopupMenu {
    let item = |label, checked, disabled, next: AgentsOrder| {
        let owner = owner.clone();
        PopupMenuItem::new(label)
            .checked(checked)
            .disabled(disabled)
            .on_click(move |_, window, cx| {
                let _ = owner.update(cx, |this, cx| this.set_agents_order(next, window, cx));
            })
    };
    let by_state = |by_state| AgentsOrder { by_state, ..order };
    let tree_order = |tree_order| AgentsOrder {
        tree_order,
        ..order
    };
    menu.label(t!("sidebar.agents_order.group"))
        .item(item(
            t!("sidebar.agents_order.workspace"),
            !order.by_state,
            false,
            by_state(false),
        ))
        .item(item(
            t!("sidebar.agents_order.state"),
            order.by_state,
            false,
            by_state(true),
        ))
        .separator()
        .label(t!("sidebar.agents_order.order"))
        .item(item(
            t!("sidebar.agents_order.urgency"),
            !order.tree_order,
            order.by_state,
            tree_order(false),
        ))
        .item(item(
            t!("sidebar.agents_order.sidebar"),
            order.tree_order,
            order.by_state,
            tree_order(true),
        ))
}

impl Condr {
    /// Every Agent the window's section lists, in its order. A Device that is not
    /// connected lists none, as in "Needs you".
    pub(in crate::app) fn agent_rows(&self, view: &WindowView) -> Vec<AgentRow> {
        let scope = view.scope;
        let mut rows = Vec::new();
        for connection in &self.connections {
            let key = connection.key;
            let Some(session) = connection.presented_session() else {
                continue;
            };
            for workspace in session.workspaces() {
                if !scope.lists_workspace(key, workspace.id()) {
                    continue;
                }
                for pane in workspace.tabs().iter().flat_map(|tab| tab.panes()) {
                    if let Some(status) = pane_sidebar_status(&connection.model, pane.id()) {
                        rows.push(AgentRow {
                            key,
                            workspace_id: workspace.id(),
                            pane_id: pane.id(),
                            status,
                            rank: 0,
                        });
                    }
                }
            }
        }
        rank_agents(&mut rows, view.agents_order);
        rows
    }

    /// Selects the next or previous Agent in the window's section, wrapping at both
    /// ends. When the keyboard's target is none of them, next starts at the first and
    /// previous at the last.
    pub(in crate::app) fn cycle_agent(
        &mut self,
        step: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.in_tasks_mode(self.window_view(window)) {
            return self.step_task_card(step, true, window, cx);
        }
        let rows = self.agent_rows(self.window_view(window));
        if rows.is_empty() {
            return;
        }
        let current = self
            .terminal_input
            .target
            .and_then(|target| rows.iter().position(|row| (row.key, row.pane_id) == target));
        let target_ix = match current {
            Some(ix) => (ix as isize + step).rem_euclid(rows.len() as isize) as usize,
            None if step > 0 => 0,
            None => rows.len() - 1,
        };
        let row = rows[target_ix];
        self.select_pane(row.key, row.pane_id, window, cx);
    }

    fn set_agents_order(&mut self, order: AgentsOrder, window: &Window, cx: &mut Context<Self>) {
        self.window_view_mut(window).agents_order = order;
        self.schedule_state_save(cx);
        cx.notify();
    }

    fn toggle_agents_section(&mut self, window: &Window, cx: &mut Context<Self>) {
        let view = self.window_view_mut(window);
        view.agents_collapsed = !view.agents_collapsed;
        self.schedule_state_save(cx);
        cx.notify();
    }

    pub(super) fn render_agents_section(
        &self,
        view: &WindowView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = self.agent_rows(view);
        let order = view.agents_order;
        let collapsed = view.agents_collapsed;
        let owner = cx.weak_entity();
        let label = t!("sidebar.agents");
        let summary = if collapsed {
            agent_status_summary(rows.iter().map(|row| row.status))
        } else {
            Vec::new()
        };
        // Folding slides the section and turns its chevron on one spring, which an
        // interrupting click reverses from where it is (none under reduced motion).
        let open = SpringAnimation::new(FOLD_SPRING)
            .with_epsilon(FOLD_EPSILON)
            .to(!collapsed);
        let toggle_owner = owner.clone();
        let chevron = Button::new("agents-fold")
            .debug_selector(|| "agents-fold".into())
            .xsmall()
            .ghost()
            .tooltip(if collapsed {
                t!("sidebar.expand", name = label)
            } else {
                t!("sidebar.collapse", name = label)
            })
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                let _ = toggle_owner.update(cx, |this, cx| this.toggle_agents_section(window, cx));
            })
            .with_spring("agents-chevron", open.clone(), |button, phase| {
                button.icon(
                    Icon::new(IconName::ChevronRight)
                        .size_3p5()
                        .rotate(percentage(phase.interpolate_clamped(0., 0.25))),
                )
            });
        let heading_owner = owner.clone();
        // Styled as a Device heading is, which a click folds the same way.
        let heading = h_flex()
            .id("agents-heading")
            .debug_selector(|| "agents-heading".into())
            .h(HEADING_HEIGHT)
            .w_full()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_1()
            .rounded(cx.theme().radius)
            .text_xs()
            .font_semibold()
            .text_color(cx.theme().sidebar_foreground.opacity(0.7))
            .map(|this| row_highlight(this, false, cx))
            .child(chevron)
            .child(div().min_w_0().truncate().child(label))
            .when(!summary.is_empty(), |this| {
                this.child(status_counts(&summary, cx).font_normal())
            })
            // Open, the menu that groups and orders the rows; its popover takes the press,
            // so the heading does not fold.
            .when(!collapsed, |this| {
                let order_owner = owner.clone();
                let label = t!("sidebar.agents_order.button");
                this.child(div().flex_1()).child(
                    Button::new("agents-order")
                        .debug_selector(|| "agents-order".into())
                        .xsmall()
                        .ghost()
                        .icon(Icon::new(gpui_kit::assets::IconName::ListFilter).size_3p5())
                        .tooltip(label.clone())
                        .accessibility_label(label)
                        .dropdown_menu(move |menu, _, _| {
                            agents_order_menu(menu, order, order_owner.clone())
                        }),
                )
            })
            .on_click(move |_, window, cx| {
                let _ = heading_owner.update(cx, |this, cx| this.toggle_agents_section(window, cx));
            });
        let section = v_flex()
            .id("agents-section")
            .debug_selector(|| "agents-section".into())
            .relative();

        let several_devices = self.connections.len() > 1;
        let mut items = Vec::new();
        let mut group = None;
        for row in &rows {
            let AgentRow {
                key,
                workspace_id,
                pane_id,
                status,
                rank,
            } = *row;
            let Some(connection) = self.connection(key) else {
                continue;
            };
            let Some(workspace) = connection
                .presented_session()
                .and_then(|session| session.workspace(workspace_id))
            else {
                continue;
            };
            let Some(agent) = connection.model.agents.get(&pane_id) else {
                continue;
            };
            let workspace_name = if several_devices {
                format!("{} · {}", workspace.name(), connection.label)
            } else {
                workspace.name().to_owned()
            };
            // A muted heading over each group's Agents; it does nothing when clicked.
            let this_group = if order.by_state {
                (None, rank)
            } else {
                (Some((key, workspace_id)), 0)
            };
            if group != Some(this_group) {
                group = Some(this_group);
                let (selector, name) = if order.by_state {
                    let state = urgency_status(rank);
                    (
                        format!("agents-state-{}", state.key),
                        state.label.to_owned(),
                    )
                } else {
                    let selector = format!("agents-group-{key}-{}", workspace_id.as_u64());
                    (selector, workspace_name.clone())
                };
                items.push((
                    ElementId::from(SharedString::from(selector.clone())),
                    div()
                        .debug_selector(move || selector.clone())
                        .px_1()
                        .pt_1()
                        .truncate()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(name)
                        .into_any_element(),
                ));
            }
            let kind = agent.kind.label();
            let title = connection
                .model
                .terminal_titles
                .get(&pane_id)
                .cloned()
                .unwrap_or_else(|| kind.to_owned());
            let pane = pane_id.as_u64();
            let row_owner = owner.clone();
            items.push((
                ElementId::from(SharedString::from(format!("agents-{key}-{pane}"))),
                CondrSidebarTreeItem::new(
                    format!("agents-{key}-{pane}"),
                    format!("agents-row-{key}-{pane}"),
                    format!("agents-label-{key}-{pane}"),
                    title,
                )
                .icon(CondrSidebarIcon::agent(
                    agent.kind,
                    status,
                    format!("agents-status-{key}-{pane}-{}", status.key),
                    format!("{kind}: {}", status.label),
                ))
                // What it waits for or why it halted, while it says so (ADR 0024).
                .when_some(agent.detail.clone(), |item, text| item.detail(text))
                // By state its heading names the state, so the row names its Workspace.
                .when(order.by_state, |item| {
                    item.label_end(workspace_name, cx.theme().muted_foreground)
                })
                // Named only while it wants something or works; a quiet row stays quiet.
                .when(
                    !order.by_state && !matches!(status.key, "idle" | "unknown"),
                    |item| item.label_end(status.label, status.tone.color(cx)),
                )
                .active(
                    key == self.presented_key(view)
                        && self.terminal_input.target == Some((key, pane_id)),
                )
                .disable(!connection.can_mutate())
                .on_click(move |_, window, cx| {
                    let _ =
                        row_owner.update(cx, |this, cx| this.select_pane(key, pane_id, window, cx));
                })
                .render(window, cx),
            ));
        }
        let list = if items.is_empty() {
            div()
                .debug_selector(|| "agents-empty".into())
                .px_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(t!("sidebar.no_agents"))
                .into_any_element()
        } else {
            SlideColumn::new("agents-rows")
                .gap_1()
                .rows(items)
                .into_any_element()
        };
        // Folded, the section is its heading.
        let folded_height = HEADING_HEIGHT.to_pixels(window.rem_size());
        // The colour Kit's resize handle draws its line in.
        let border = cx.theme().border;
        let open_height = agents_height(view);
        section
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<DraggedAgents>, window, cx| {
                    // The section grows upward from its bottom edge, which stays put.
                    let height = event.bounds.bottom() - event.event.position.y;
                    let view = this.window_view_mut(window);
                    view.agents_height = Some(height.round());
                    view.agents_height = Some(agents_height(view));
                    this.schedule_state_save(cx);
                    cx.notify();
                }),
            )
            .child(heading)
            .with_spring("agents-open", open, move |section, phase| {
                let phase = phase.clamp(0.0..=1.0);
                section
                    .h(phase.interpolate(folded_height, open_height))
                    // Only while it slides, so the open handle reaches over the border.
                    .when(phase.0 < 1., |section| section.overflow_hidden())
                    // Folded, the rule over the heading is a plain line, the one the open
                    // handle draws; over the heading's first pixel, so it adds no height.
                    .when(phase.0 == 0., |section| {
                        section.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .w_full()
                                .h(px(1.))
                                .bg(border),
                        )
                    })
                    // The list stays until the section has closed over it.
                    .when(phase.0 > 0., |section| {
                        section
                            .child(
                                div()
                                    .id("agents-list")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .pb_2()
                                    .child(list),
                            )
                            // Kit's handle straddles the section's top edge and draws its
                            // rule, with a band wider than the line to grab; it swallows the
                            // press so the row under it is not clicked.
                            .child(
                                resize_handle("agents-resize", Axis::Vertical)
                                    .debug_selector(|| "agents-resize".into())
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_drag(DraggedAgents, |_, _, _, cx| cx.new(|_| EmptyView)),
                            )
                    })
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentRow, AgentsOrder, rank_agents};
    use crate::app::sidebar::{BELL_SIDEBAR_STATUS, agent_sidebar_status, urgency_status};
    use condr_core::{AgentDisplayState, PaneId, WorkspaceId};

    fn row(pane: u64, state: AgentDisplayState) -> AgentRow {
        in_workspace(1, pane, state)
    }

    fn in_workspace(workspace: u64, pane: u64, state: AgentDisplayState) -> AgentRow {
        AgentRow {
            key: 1,
            workspace_id: WorkspaceId::from_u64(workspace),
            pane_id: PaneId::from_u64(pane),
            status: agent_sidebar_status(state),
            rank: 0,
        }
    }

    #[test]
    fn workspaces_group_their_agents_the_most_urgent_group_first() {
        let mut rows = vec![
            in_workspace(1, 1, AgentDisplayState::Idle),
            in_workspace(1, 2, AgentDisplayState::Working),
            in_workspace(2, 3, AgentDisplayState::Idle),
            in_workspace(2, 4, AgentDisplayState::Blocked),
            in_workspace(3, 5, AgentDisplayState::Working),
        ];
        rank_agents(&mut rows, AgentsOrder::default());
        // Workspace 2 holds the Blocked one; 1 and 3 tie on Working and keep Session order.
        assert_eq!(panes(&rows), [4, 3, 2, 1, 5]);
    }

    fn panes(rows: &[AgentRow]) -> Vec<u64> {
        rows.iter().map(|row| row.pane_id.as_u64()).collect()
    }

    #[test]
    fn agents_sort_by_urgency_and_keep_session_order_within_a_state() {
        let mut rows = vec![
            row(1, AgentDisplayState::Idle),
            row(2, AgentDisplayState::Working),
            row(3, AgentDisplayState::Done),
            row(4, AgentDisplayState::Unknown),
            row(5, AgentDisplayState::Blocked),
            row(6, AgentDisplayState::Done),
            row(7, AgentDisplayState::Error),
            AgentRow {
                status: BELL_SIDEBAR_STATUS,
                ..row(8, AgentDisplayState::Idle)
            },
        ];
        rank_agents(&mut rows, AgentsOrder::default());
        // Blocked and Error ahead of a bell (ADR 0074).
        assert_eq!(panes(&rows), [5, 7, 8, 3, 6, 2, 1, 4]);
    }

    #[test]
    fn the_trees_order_moves_only_what_needs_a_person() {
        let session = || {
            vec![
                in_workspace(1, 1, AgentDisplayState::Idle),
                in_workspace(1, 2, AgentDisplayState::Working),
                in_workspace(2, 3, AgentDisplayState::Done),
                in_workspace(2, 4, AgentDisplayState::Idle),
                in_workspace(3, 5, AgentDisplayState::Working),
                in_workspace(3, 6, AgentDisplayState::Error),
                in_workspace(3, 7, AgentDisplayState::Blocked),
            ]
        };
        let tree_order = AgentsOrder {
            by_state: false,
            tree_order: true,
        };
        let mut rows = session();
        rank_agents(&mut rows, tree_order);
        // Workspace 3 leads with its Blocked and Error; the rest keep Session order.
        assert_eq!(panes(&rows), [7, 6, 5, 1, 2, 3, 4]);
        let mut rows = session();
        rank_agents(&mut rows, AgentsOrder::default());
        assert_eq!(panes(&rows), [7, 6, 5, 3, 4, 2, 1]);
    }

    #[test]
    fn by_state_the_rows_fall_under_their_states() {
        let session = || {
            vec![
                in_workspace(1, 1, AgentDisplayState::Idle),
                in_workspace(1, 2, AgentDisplayState::Done),
                in_workspace(2, 3, AgentDisplayState::Unknown),
                in_workspace(2, 4, AgentDisplayState::Working),
                in_workspace(1, 5, AgentDisplayState::Blocked),
                in_workspace(2, 6, AgentDisplayState::Idle),
            ]
        };
        for tree_order in [false, true] {
            let mut rows = session();
            let order = AgentsOrder {
                by_state: true,
                tree_order,
            };
            rank_agents(&mut rows, order);
            // Within a state every order is Session order.
            assert_eq!(panes(&rows), [5, 2, 4, 1, 3, 6]);
            let headings = rows
                .iter()
                .map(|row| urgency_status(row.rank).key)
                .collect::<Vec<_>>();
            assert_eq!(
                headings,
                ["blocked", "done", "working", "idle", "idle", "idle"],
                "an Unknown Agent sits with the Idle ones"
            );
        }
    }
}
