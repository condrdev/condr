//! An Agent's card as the Board's Cards and Compact and the Table's row draw it, and the
//! facts they share (ADR 0068).

use super::*;

/// What a card shows of its Agent.
pub(super) struct CardFacts {
    pub(super) kind: AgentKind,
    pub(super) title: String,
    pub(super) workspace: String,
    pub(super) branch: Option<String>,
    pub(super) device: String,
    pub(super) added: u32,
    pub(super) deleted: u32,
    pub(super) ahead: u32,
    pub(super) detail: Option<String>,
}

/// The state's name in its colour, absent for a quiet Agent (ADR 0067).
pub(super) fn state_name(status: SidebarStatusVisual, cx: &App) -> Option<Div> {
    (!matches!(status.key, "idle" | "unknown")).then(|| {
        div()
            .flex_none()
            .text_xs()
            .text_color(status.tone.color(cx))
            .child(status.label)
    })
}

pub(super) fn device_tag(device: String, theme: &Theme) -> Div {
    div()
        .flex_none()
        .px_1()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .child(device)
}

/// `+12 −3 ↑1`, or nothing for a clean Workspace in step with its upstream.
pub(super) fn change_counts(added: u32, deleted: u32, ahead: u32, theme: &Theme) -> Option<Div> {
    (added > 0 || deleted > 0 || ahead > 0).then(|| {
        h_flex()
            .gap_1()
            .text_xs()
            .when(added > 0 || deleted > 0, |this| {
                this.child(div().text_color(theme.success).child(format!("+{added}")))
                    .child(div().text_color(theme.danger).child(format!("−{deleted}")))
            })
            .when(ahead > 0, |this| {
                this.child(div().text_color(theme.success).child(format!("↑{ahead}")))
            })
    })
}

impl Condr {
    /// What a card shows, gathered once for every density.
    pub(super) fn card_facts(&self, card: &TaskCard) -> Option<CardFacts> {
        let connection = self.connection(card.key)?;
        let workspace = connection
            .presented_session()?
            .workspace(card.workspace_id)?;
        let agent = connection.model.agents.get(&card.pane_id)?;
        let kind = agent.kind;
        // Every Agent in a Task's Workspace carries the Task's title (ADR 0068).
        let title = connection
            .model
            .workspace_task(card.workspace_id)
            .map(|task| task.title.clone())
            .or_else(|| connection.model.terminal_titles.get(&card.pane_id).cloned())
            .unwrap_or_else(|| kind.label().to_owned());
        let git = connection.model.workspace_git.get(&card.workspace_id);
        let branch = git.and_then(|git| git.branch.clone());
        let (added, deleted) = git.map_or((0, 0), |git| {
            git.changes
                .entries
                .iter()
                .filter_map(|entry| entry.stat)
                .fold((0, 0), |(added, deleted), stat| {
                    (added + stat.added, deleted + stat.deleted)
                })
        });
        let ahead = git
            .and_then(|git| git.upstream)
            .map_or(0, |upstream| upstream.ahead);
        Some(CardFacts {
            kind,
            title,
            workspace: workspace.name().to_owned(),
            branch,
            device: connection.label.clone(),
            added,
            deleted,
            ahead,
            detail: agent.detail.clone(),
        })
    }

    pub(super) fn card_icon(
        &self,
        card: &TaskCard,
        kind: AgentKind,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (key, pane) = (card.key, card.pane_id.as_u64());
        CondrSidebarIcon::agent(
            kind,
            card.status,
            format!("tasks-status-{key}-{pane}-{}", card.status.key),
            format!("{}: {}", kind.label(), card.status.label),
        )
        .render(cx)
    }

    pub(super) fn render_card(&self, card: &TaskCard, cx: &mut Context<Self>) -> AnyElement {
        let Some(facts) = self.card_facts(card) else {
            return div().into_any_element();
        };
        let icon = self.card_icon(card, facts.kind, cx);
        let (card_key, card_pane) = (card.key, card.pane_id);
        let selected = self
            .main_view()
            .tasks_selection
            .is_some_and(|selected| (selected.key, selected.pane_id) == (card_key, card_pane));
        let on_click = cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.click_task_card(card_key, card_pane, None, window, cx)
        });
        let theme = cx.theme();
        let (key, pane) = (card.key, card.pane_id.as_u64());
        let place = match &facts.branch {
            Some(branch) => format!("{} · {branch}", facts.workspace),
            None => facts.workspace.clone(),
        };
        v_flex()
            .id(SharedString::from(format!("tasks-card-{key}-{pane}")))
            .debug_selector(move || format!("tasks-card-{key}-{pane}"))
            .w_full()
            .gap_1()
            .p_2()
            .rounded(theme.radius)
            .border_1()
            .border_color(if selected {
                theme.primary
            } else {
                theme.border
            })
            .bg(theme.sidebar)
            .cursor_pointer()
            .on_click(on_click)
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(icon)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_semibold()
                            .truncate()
                            .child(facts.title),
                    )
                    .children(state_name(card.status, cx)),
            )
            .child(
                h_flex()
                    .gap_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(div().min_w_0().truncate().child(place))
                    .child(device_tag(facts.device, theme)),
            )
            .children(change_counts(
                facts.added,
                facts.deleted,
                facts.ahead,
                theme,
            ))
            .children(facts.detail.map(|detail| {
                div()
                    .text_xs()
                    .text_color(card.status.tone.color(cx))
                    .child(detail)
            }))
            .into_any_element()
    }

    pub(super) fn render_compact_card(
        &self,
        card: &TaskCard,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(facts) = self.card_facts(card) else {
            return div().into_any_element();
        };
        let icon = self.card_icon(card, facts.kind, cx);
        let (card_key, card_pane) = (card.key, card.pane_id);
        let selected = self
            .main_view()
            .tasks_selection
            .is_some_and(|selected| (selected.key, selected.pane_id) == (card_key, card_pane));
        let on_click = cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.click_task_card(card_key, card_pane, None, window, cx)
        });
        let theme = cx.theme();
        let (key, pane) = (card.key, card.pane_id.as_u64());
        v_flex()
            .id(SharedString::from(format!("tasks-card-{key}-{pane}")))
            .debug_selector(move || format!("tasks-card-{key}-{pane}"))
            .w_full()
            .px_1()
            .py_0p5()
            .rounded(theme.radius)
            .border_1()
            .border_color(if selected {
                theme.primary
            } else {
                gpui_kit::transparent_black()
            })
            .cursor_pointer()
            .on_click(on_click)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_sm()
                    .child(icon)
                    .child(div().flex_1().min_w_0().truncate().child(facts.title))
                    .children(state_name(card.status, cx)),
            )
            .children(facts.detail.map(|detail| {
                div()
                    .pl(rems(1.5))
                    .text_xs()
                    .truncate()
                    .text_color(card.status.tone.color(cx))
                    .child(detail)
            }))
            .into_any_element()
    }

    pub(super) fn render_list_row(
        &self,
        card: &TaskCard,
        fit: ListFit,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(facts) = self.card_facts(card) else {
            return div().into_any_element();
        };
        let icon = self.card_icon(card, facts.kind, cx);
        let (card_key, card_pane) = (card.key, card.pane_id);
        let selected = self
            .main_view()
            .tasks_selection
            .is_some_and(|selected| (selected.key, selected.pane_id) == (card_key, card_pane));
        let on_click = cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.click_task_card(card_key, card_pane, None, window, cx)
        });
        let theme = cx.theme();
        let (key, pane) = (card.key, card.pane_id.as_u64());
        let state = match &facts.detail {
            Some(detail) => div()
                .text_color(card.status.tone.color(cx))
                .child(detail.clone()),
            None => div().children(state_name(card.status, cx)),
        };
        list_row(
            fit,
            format!("tasks-row-{key}-{pane}"),
            icon,
            facts.title,
            facts.workspace,
            facts.branch.unwrap_or_default(),
            facts.device,
            h_flex().children(change_counts(
                facts.added,
                facts.deleted,
                facts.ahead,
                theme,
            )),
            state,
        )
        .text_sm()
        .rounded(theme.radius)
        .border_1()
        .border_color(if selected {
            theme.primary
        } else {
            gpui_kit::transparent_black()
        })
        .cursor_pointer()
        .on_click(on_click)
        .into_any_element()
    }
}
