//! Git Graph's commit table (ADR 0079): GPUI Kit's `DataTable` over the commits loaded so
//! far, the graph drawn in each row's subject cell, IntelliJ's columns and highlights.

use super::branches::{RefKind, short_name};
use super::lanes::GraphRow;
use super::*;
use crate::app::format::commit_date;
use gpui_kit::component::table::{Column, TableDelegate, TableState};
use rust_i18n::t;
use std::collections::HashMap;

/// One lane's width in the graph, and its dot's radius.
const LANE: Pixels = px(14.);
const DOT: Pixels = px(3.5);
/// The columns after the subject, which takes the rest of the table's width.
const AUTHOR: Pixels = px(110.);
const DATE: Pixels = px(140.);
const HASH: Pixels = px(84.);
/// The narrowest the subject gets before the table scrolls sideways.
const SUBJECT_MIN: Pixels = px(160.);

/// The label a commit's refs show at the end of its subject, as IntelliJ's compact view
/// merges them: a local branch with the remotes of the same name, `origin & main`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RefLabel {
    pub(super) text: String,
    pub(super) kind: RefKind,
}

/// One row: the commit, its slice of the graph and its labels.
pub(super) struct CommitRow {
    pub(super) commit: CommitSummary,
    pub(super) graph: GraphRow,
    pub(super) labels: Vec<RefLabel>,
    /// Whether a tag names it, which shows as an icon.
    pub(super) tagged: bool,
}

/// The labels for each commit the refs name: the checked-out branch's first as `HEAD`'s.
pub(super) fn ref_labels(
    refs: &[GitRef],
    current: Option<&str>,
) -> HashMap<String, (Vec<RefLabel>, bool)> {
    let mut by_commit: HashMap<String, (Vec<RefLabel>, bool)> = HashMap::new();
    let mut locals: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut remotes: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut detached_head = None;
    for reference in refs {
        match RefKind::of(&reference.name) {
            Some(RefKind::Head) => detached_head = Some(reference.commit.as_str()),
            Some(RefKind::Local) => locals
                .entry(&reference.commit)
                .or_default()
                .push(short_name(&reference.name)),
            Some(RefKind::Remote) => remotes
                .entry(&reference.commit)
                .or_default()
                .push(short_name(&reference.name)),
            Some(RefKind::Tag) => by_commit.entry(reference.commit.clone()).or_default().1 = true,
            None => {}
        }
    }
    let commits: std::collections::BTreeSet<&str> = locals
        .keys()
        .chain(remotes.keys())
        .copied()
        .chain(detached_head)
        .collect();
    for commit in commits {
        let mut labels = Vec::new();
        let mut remote_names = remotes.get(commit).cloned().unwrap_or_default();
        remote_names.sort_unstable();
        let mut local_names = locals.get(commit).cloned().unwrap_or_default();
        local_names.sort_unstable_by_key(|name| (Some(*name) != current, *name));
        for local in &local_names {
            let mut parts: Vec<&str> = Vec::new();
            remote_names.retain(|remote| {
                let same = remote
                    .split_once('/')
                    .is_some_and(|(_, branch)| branch == *local);
                if same {
                    parts.push(remote.split_once('/').map_or(remote, |(name, _)| name));
                }
                !same
            });
            parts.push(local);
            labels.push(RefLabel {
                text: parts.join(" & "),
                kind: if Some(*local) == current {
                    RefKind::Head
                } else {
                    RefKind::Local
                },
            });
        }
        labels.extend(remote_names.into_iter().map(|remote| RefLabel {
            text: remote.to_owned(),
            kind: RefKind::Remote,
        }));
        if current.is_none() && detached_head == Some(commit) {
            labels.insert(
                0,
                RefLabel {
                    text: "HEAD".into(),
                    kind: RefKind::Head,
                },
            );
        }
        by_commit.entry(commit.to_owned()).or_default().0 = labels;
    }
    by_commit
}

/// The lane colour a seed names, as IntelliJ makes them: a hue at fixed saturation and
/// brightness (HSB 0.4, 0.65).
pub(super) fn lane_color(seed: u32) -> Hsla {
    let hue = (seed as f32 * 0.618_034).fract();
    hsla(hue, 0.27, 0.52, 1.)
}

/// A label's colour: `HEAD` yellow, local green, remote violet, as IntelliJ has them.
pub(super) fn label_color(kind: RefKind, cx: &App) -> Hsla {
    let theme = cx.theme();
    match kind {
        RefKind::Head => theme.yellow,
        RefKind::Local => theme.green,
        RefKind::Remote | RefKind::Tag => theme.magenta,
    }
}

pub(super) fn label_chip(label: &RefLabel, cx: &App) -> Div {
    let color = label_color(label.kind, cx);
    h_flex()
        .min_w_0()
        .gap_1()
        .items_center()
        .text_xs()
        .text_color(color)
        .child(
            Icon::new(Glyph::GitBranch)
                .size_3()
                .flex_none()
                .text_color(color),
        )
        .child(div().min_w_0().truncate().child(label.text.clone()))
}

/// The table's rows and how to reach Condr from them.
pub(in crate::app) struct CommitTable {
    pub(super) owner: WeakEntity<Condr>,
    pub(super) key: ConnectionKey,
    pub(super) tab_id: TabId,
    pub(super) rows: Vec<CommitRow>,
    pub(super) more: bool,
    pub(super) loading: bool,
    /// The repository's `user.email`, whose commits are bold.
    pub(super) user_email: Option<String>,
    /// Tint the commits `HEAD` reaches: only while every ref shows, as IntelliJ does.
    pub(super) tint_head: bool,
    /// The text filter is set, so an empty table says nothing matched.
    pub(super) filtered: bool,
    /// The resources' generation the rows were taken at.
    pub(super) generation: Option<u64>,
    /// The table's width as last drawn, which the subject fills less the other columns:
    /// Kit's columns have fixed widths.
    pub(super) width: Pixels,
}

impl CommitTable {
    fn graph_cell(&self, row: &GraphRow) -> impl IntoElement {
        let row = row.clone();
        let width = LANE * row.width as f32;
        canvas(
            |_, _, _| {},
            move |bounds, (), window, _| {
                let x = |lane: usize| bounds.left() + LANE * lane as f32 + LANE / 2.;
                let (top, middle, bottom) = (bounds.top(), bounds.center().y, bounds.bottom());
                let mut line = |from: Point<Pixels>, to: Point<Pixels>, color: u32| {
                    let mut path = PathBuilder::stroke(px(1.5));
                    path.move_to(from);
                    path.line_to(to);
                    if let Ok(path) = path.build() {
                        window.paint_path(path, lane_color(color));
                    }
                };
                for edge in &row.top {
                    line(
                        point(x(edge.from), top),
                        point(x(edge.to), middle),
                        edge.color,
                    );
                }
                for edge in &row.bottom {
                    line(
                        point(x(edge.from), middle),
                        point(x(edge.to), bottom),
                        edge.color,
                    );
                }
                let center = point(x(row.lane), middle);
                window.paint_quad(
                    fill(
                        Bounds::centered_at(center, size(DOT * 2., DOT * 2.)),
                        lane_color(row.color),
                    )
                    .corner_radii(DOT),
                );
            },
        )
        .flex_none()
        .w(width)
        .h_full()
    }
}

impl TableDelegate for CommitTable {
    fn columns_count(&self, _: &App) -> usize {
        4
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        match col_ix {
            // The graph touches the row's edges, so the subject has no padding of its own.
            // Less the vertical scrollbar's room.
            0 => Column::new("subject", t!("git_graph.column.subject"))
                .width((self.width - AUTHOR - DATE - HASH - px(16.)).max(SUBJECT_MIN))
                .paddings(Edges {
                    left: px(4.),
                    right: px(8.),
                    top: px(0.),
                    bottom: px(0.),
                }),
            1 => Column::new("author", t!("git_graph.column.author")).width(AUTHOR),
            2 => Column::new("date", t!("git_graph.column.date")).width(DATE),
            _ => Column::new("hash", t!("git_graph.column.hash")).width(HASH),
        }
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let row = &self.rows[row_ix];
        let own = self
            .user_email
            .as_deref()
            .is_some_and(|email| email == row.commit.author_email);
        let theme = cx.theme();
        div()
            .id(("git-graph-row", row_ix))
            // No rule between rows, so the graph's lanes run on unbroken.
            .border_b_0()
            .when(self.tint_head && row.commit.on_head, |this| {
                this.bg(theme.blue.opacity(0.08))
            })
            .when(row.commit.parents.len() > 1, |this| {
                this.text_color(theme.muted_foreground)
            })
            .when(own, |this| this.font_weight(FontWeight::BOLD))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let row = &self.rows[row_ix];
        let commit = &row.commit;
        match col_ix {
            // The subject keeps some room; the labels give theirs up first.
            0 => h_flex()
                .size_full()
                .gap_2()
                .items_center()
                .child(self.graph_cell(&row.graph))
                .child(
                    div()
                        .min_w(rems(6.))
                        .flex_1()
                        .truncate()
                        .child(commit.subject.clone()),
                )
                .when(!row.labels.is_empty() || row.tagged, |this| {
                    this.child(
                        h_flex()
                            .min_w_0()
                            .flex_shrink(1.)
                            .gap_2()
                            .items_center()
                            .children(row.labels.iter().take(2).map(|label| label_chip(label, cx)))
                            .when(row.labels.len() > 2, |this| {
                                this.child(
                                    div()
                                        .flex_none()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("+{}", row.labels.len() - 2)),
                                )
                            })
                            .when(row.tagged, |this| {
                                this.child(
                                    Icon::new(Glyph::Tag)
                                        .size_3()
                                        .flex_none()
                                        .text_color(label_color(RefKind::Tag, cx)),
                                )
                            }),
                    )
                })
                .into_any_element(),
            1 => div()
                .truncate()
                .child(if commit.committer_differs {
                    format!("{}*", commit.author)
                } else {
                    commit.author.clone()
                })
                .into_any_element(),
            2 => div()
                .truncate()
                .child(commit_date(commit.time, chrono::Local::now()))
                .into_any_element(),
            _ => div()
                .truncate()
                .font_family(cx.theme().mono_font_family.clone())
                .child(commit.id.chars().take(8).collect::<String>())
                .into_any_element(),
        }
    }

    fn cell_label(&self, row_ix: usize, col_ix: usize, _: &App) -> Option<SharedString> {
        let commit = &self.rows.get(row_ix)?.commit;
        Some(match col_ix {
            0 => commit.subject.clone().into(),
            1 => commit.author.clone().into(),
            2 => commit_date(commit.time, chrono::Local::now()).into(),
            _ => short_id(&commit.id).to_owned().into(),
        })
    }

    fn context_menu(
        &mut self,
        row_ix: usize,
        menu: PopupMenu,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        let Some(row) = self.rows.get(row_ix) else {
            return menu;
        };
        let id = row.commit.id.clone();
        let (owner, key, tab_id) = (self.owner.clone(), self.key, self.tab_id);
        let step = move |parent: bool| {
            let owner = owner.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let _ = owner.update(cx, |this, cx| {
                    this.step_git_graph_commit(key, tab_id, row_ix, parent, window, cx);
                });
            }
        };
        menu.item(
            PopupMenuItem::new(t!("git_history.copy_commit_id"))
                .icon(menu_icon(Glyph::Copy))
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(id.clone()));
                }),
        )
        .item(PopupMenuItem::new(t!("git_graph.go_to_parent")).on_click(step(true)))
        .item(PopupMenuItem::new(t!("git_graph.go_to_child")).on_click(step(false)))
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        empty_state(
            if self.filtered {
                t!("git_graph.no_matches")
            } else {
                t!("git_history.no_commits")
            },
            cx,
        )
    }

    fn loading(&self, _: &App) -> bool {
        self.loading && self.rows.is_empty()
    }

    fn has_more(&self, _: &App) -> bool {
        self.more
    }

    fn load_more(&mut self, _: &mut Window, cx: &mut Context<TableState<Self>>) {
        let (owner, key, tab_id) = (self.owner.clone(), self.key, self.tab_id);
        // Never inside this table's own update: asking reads the table back.
        cx.defer(move |cx| {
            let _ = owner.update(cx, |this, cx| {
                this.request_git_graph_page(key, tab_id, true);
                cx.notify();
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{RefKind, RefLabel, ref_labels};
    use condr_core::GitRef;

    #[test]
    fn a_branch_and_its_remotes_share_one_label_and_the_current_one_is_heads() {
        let refs: Vec<GitRef> = [
            ("HEAD", "a"),
            ("refs/heads/main", "a"),
            ("refs/remotes/origin/main", "a"),
            ("refs/remotes/origin/dev", "a"),
            ("refs/heads/topic", "b"),
            ("refs/tags/v1", "b"),
        ]
        .iter()
        .map(|(name, commit)| GitRef {
            name: (*name).into(),
            commit: (*commit).into(),
        })
        .collect();
        let labels = ref_labels(&refs, Some("main"));
        assert_eq!(
            labels["a"].0,
            [
                RefLabel {
                    text: "origin & main".into(),
                    kind: RefKind::Head
                },
                RefLabel {
                    text: "origin/dev".into(),
                    kind: RefKind::Remote
                },
            ]
        );
        assert!(!labels["a"].1);
        assert_eq!(
            labels["b"],
            (
                vec![RefLabel {
                    text: "topic".into(),
                    kind: RefKind::Local
                }],
                true
            )
        );
        // A detached HEAD is a label of its own.
        let detached = ref_labels(&refs, None);
        assert_eq!(detached["a"].0[0].text, "HEAD");
    }
}
