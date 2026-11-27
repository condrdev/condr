//! Git Graph's filters above its commits (ADR 0079), as IntelliJ's log has them: the text,
//! the branch, `HEAD` until another is picked, the author and the date, then Refresh.

use super::branches::{RefKind, RefMenu, ref_menus};
use super::*;
use crate::app::dialogs::{field_error, open_form_dialog};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone as _};
use condr_core::LogFilter;
use gpui_kit::component::calendar::Date;
use gpui_kit::component::date_picker::{DatePicker, DatePickerState};
use std::borrow::Cow;
use std::rc::Rc;

/// What a filter's menu does with the value picked.
type Pick<T> = Rc<dyn Fn(T, &mut App)>;
/// What a filter's clear button does.
type Clear = Rc<dyn Fn(&mut App)>;

/// The span of author dates the Date filter keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum DateFilter {
    #[default]
    All,
    LastDay,
    LastWeek,
    /// Whole local days, from the first through the last; an open end runs on.
    Days(Option<NaiveDate>, Option<NaiveDate>),
}

impl DateFilter {
    /// Its ends in seconds since the Unix epoch, as of `now`, both kept.
    fn bounds(self, now: DateTime<Local>) -> (Option<i64>, Option<i64>) {
        const DAY: i64 = 24 * 60 * 60;
        let local = |time: NaiveDateTime| {
            Local
                .from_local_datetime(&time)
                .earliest()
                .map(|time| time.timestamp())
        };
        match self {
            Self::All => (None, None),
            Self::LastDay => (Some(now.timestamp() - DAY), None),
            Self::LastWeek => (Some(now.timestamp() - 7 * DAY), None),
            Self::Days(first, last) => (
                first.and_then(|day| local(day.and_time(NaiveTime::MIN))),
                last.and_then(|day| local(day.and_hms_opt(23, 59, 59)?)),
            ),
        }
    }

    fn label(self) -> Option<String> {
        let day = |day: NaiveDate| day.format("%Y/%m/%d").to_string();
        Some(match self {
            Self::All => return None,
            Self::LastDay => t!("git_graph.date.last_day").into_owned(),
            Self::LastWeek => t!("git_graph.date.last_week").into_owned(),
            Self::Days(Some(first), Some(last)) if first == last => day(first),
            Self::Days(Some(first), Some(last)) => {
                t!("git_graph.date.days", first = day(first), last = day(last)).into_owned()
            }
            Self::Days(Some(first), None) => {
                t!("git_graph.date.since", day = day(first)).into_owned()
            }
            Self::Days(None, Some(last)) => {
                t!("git_graph.date.until", day = day(last)).into_owned()
            }
            Self::Days(None, None) => return None,
        })
    }
}

/// The author the User filter keeps: how the filter names them, and their email.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AuthorFilter {
    pub(super) label: String,
    pub(super) email: String,
}

/// What a page of `view`'s log keeps.
pub(super) fn log_filter(view: &GitGraphView) -> LogFilter {
    let (since, until) = view.date.bounds(Local::now());
    LogFilter {
        text: (!view.text_filter.is_empty()).then(|| view.text_filter.clone()),
        author: view.author.as_ref().map(|author| author.email.clone()),
        since,
        until,
    }
}

/// A filter as IntelliJ draws one: its name, and its value once set, opening its menu,
/// then a button that clears it.
fn filter_button(
    id: &'static str,
    name: Cow<'static, str>,
    value: Option<String>,
    clear: Option<(Cow<'static, str>, Clear)>,
    menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> Div {
    let label = match value {
        Some(value) => t!("git_graph.filter_value", name = name, value = value).into_owned(),
        None => name.into_owned(),
    };
    h_flex()
        .flex_none()
        .items_center()
        .child(
            Button::new(id)
                .debug_selector(move || id.into())
                .ghost()
                .xsmall()
                .label(label)
                .dropdown_caret(true)
                .dropdown_menu(menu),
        )
        .children(clear.map(|(tooltip, clear)| {
            let clear_id = SharedString::from(format!("{id}-clear"));
            Button::new(clear_id.clone())
                .debug_selector(move || clear_id.to_string())
                .ghost()
                .xsmall()
                .icon(Icon::new(IconName::Close))
                .tooltip(tooltip.clone())
                .accessibility_label(tooltip)
                .on_click(move |_, _, cx| clear(cx))
        }))
}

/// One submenu of the Branch filter and its folders, each ref checked when it is `shown`.
fn ref_menu(
    menu: PopupMenu,
    refs: &RefMenu,
    shown: Option<Rc<str>>,
    pick: Pick<String>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let menu = refs.folders.iter().fold(menu, |menu, (label, folder)| {
        let (folder, shown, pick) = (folder.clone(), shown.clone(), pick.clone());
        menu.submenu(label.clone(), window, cx, move |menu, window, cx| {
            ref_menu(menu, &folder, shown.clone(), pick.clone(), window, cx)
        })
    });
    refs.refs.iter().fold(menu, |menu, (label, name)| {
        let (pick, name) = (pick.clone(), name.clone());
        menu.item(
            PopupMenuItem::new(label.clone())
                .checked(shown.as_deref() == Some(name.as_str()))
                .on_click(move |_, _, cx| pick(name.clone(), cx)),
        )
    })
}

impl Condr {
    /// Above the commits: the text filter, the Branch, User and Date filters, and Refresh at
    /// the far end.
    pub(super) fn render_git_graph_filters(
        &self,
        view: &GitGraphView,
        key: ConnectionKey,
        pane_id: PaneId,
        refs: &[GitRef],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let workspace_id = view.workspace_id;
        let owner = cx.weak_entity();
        let loading = self.connection(key).is_some_and(|connection| {
            connection
                .resources
                .pending_graph_logs
                .contains_key(&pane_id)
        });
        column_header(cx)
            .child(
                div().w(rems(14.)).flex_shrink(1.).min_w(rems(8.)).child(
                    Input::new(&view.text)
                        .xsmall()
                        .prefix(Icon::new(IconName::Search).size_3().text_color(muted)),
                ),
            )
            .child(
                h_flex()
                    .min_w_0()
                    .flex_1()
                    .gap_1()
                    .items_center()
                    .overflow_hidden()
                    .child(self.branch_filter(view, key, pane_id, refs, cx))
                    .child(self.user_filter(view, key, pane_id, cx))
                    .child(self.date_filter(view, key, pane_id, cx)),
            )
            .child(
                Button::new("git-graph-refresh")
                    .debug_selector(|| "git-graph-refresh".into())
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::RefreshCw))
                    .tooltip(t!("right_sidebar.refresh"))
                    .accessibility_label(t!("right_sidebar.refresh"))
                    .loading(loading)
                    .on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            this.reload_git_graph(key, workspace_id);
                            cx.notify();
                        });
                    }),
            )
            .into_any_element()
    }

    /// Branch: `HEAD` until another is picked; All, `HEAD`, then a submenu each for the
    /// local branches, every remote and the tags.
    fn branch_filter(
        &self,
        view: &GitGraphView,
        key: ConnectionKey,
        pane_id: PaneId,
        refs: &[GitRef],
        cx: &mut Context<Self>,
    ) -> Div {
        let owner = cx.weak_entity();
        let pick: Pick<Option<String>> = Rc::new(move |branch, cx| {
            let _ = owner.update(cx, |this, cx| {
                this.set_git_graph_branch(key, pane_id, branch, cx);
            });
        });
        let shown: Option<Rc<str>> = view.branch.as_deref().map(Rc::from);
        let menus = ref_menus(refs);
        let clear = view.branch.is_some().then(|| {
            let pick = pick.clone();
            (
                t!("git_graph.clear_branch"),
                Rc::new(move |cx: &mut App| pick(None, cx)) as Clear,
            )
        });
        filter_button(
            "git-graph-branch-filter",
            t!("git_graph.branch_any"),
            view.branch
                .as_deref()
                .map(|name| short_name(name).to_owned()),
            clear,
            move |menu, window, cx| {
                let item = |label: Cow<'static, str>, branch: Option<&str>| {
                    let pick = pick.clone();
                    let branch = branch.map(str::to_owned);
                    PopupMenuItem::new(label)
                        .checked(shown.as_deref() == branch.as_deref())
                        .on_click(move |_, _, cx| pick(branch.clone(), cx))
                };
                let menu = menu
                    .item(item(t!("git_graph.all"), None))
                    .item(item("HEAD".into(), Some("HEAD")))
                    .separator();
                menus.iter().fold(menu, |menu, (kind, remote, refs)| {
                    let label = match kind {
                        RefKind::Local => t!("git_graph.branches.local").into_owned(),
                        RefKind::Tag => t!("git_graph.branches.tags").into_owned(),
                        _ => remote.clone(),
                    };
                    let (refs, shown) = (refs.clone(), shown.clone());
                    let pick = pick.clone();
                    let pick: Pick<String> = Rc::new(move |name, cx| pick(Some(name), cx));
                    menu.submenu(label, window, cx, move |menu, window, cx| {
                        ref_menu(menu, &refs, shown.clone(), pick.clone(), window, cx)
                    })
                })
            },
        )
    }

    /// User: everyone until one is picked; All, Me by the repository's `user.email`, then
    /// everyone who authored a commit loaded so far.
    fn user_filter(
        &self,
        view: &GitGraphView,
        key: ConnectionKey,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> Div {
        let owner = cx.weak_entity();
        let pick: Pick<Option<AuthorFilter>> = Rc::new(move |author, cx| {
            let _ = owner.update(cx, |this, cx| {
                this.set_git_graph_author(key, pane_id, author, cx);
            });
        });
        let table = view.table.read(cx).delegate();
        let me = table.user_email.clone();
        let mut authors: Vec<(String, String)> = table.authors.values().cloned().collect();
        authors.sort_by_cached_key(|(name, _)| name.to_lowercase());
        let shown = view
            .author
            .as_ref()
            .map(|author| author.email.to_lowercase());
        let clear = view.author.is_some().then(|| {
            let pick = pick.clone();
            (
                t!("git_graph.user.clear"),
                Rc::new(move |cx: &mut App| pick(None, cx)) as Clear,
            )
        });
        filter_button(
            "git-graph-user-filter",
            t!("git_graph.user.name"),
            view.author.as_ref().map(|author| author.label.clone()),
            clear,
            move |menu, _, _| {
                let item = |label: String, author: Option<AuthorFilter>| {
                    let pick = pick.clone();
                    let checked = shown.as_deref()
                        == author.as_ref().map(|a| a.email.to_lowercase()).as_deref();
                    PopupMenuItem::new(label)
                        .checked(checked)
                        .on_click(move |_, _, cx| pick(author.clone(), cx))
                };
                let mut menu = menu.item(item(t!("git_graph.all").into_owned(), None));
                if let Some(email) = &me {
                    let label = t!("git_graph.user.me").into_owned();
                    menu = menu.item(item(
                        label.clone(),
                        Some(AuthorFilter {
                            label,
                            email: email.clone(),
                        }),
                    ));
                }
                if !authors.is_empty() {
                    menu = menu.separator();
                }
                authors.iter().fold(menu, |menu, (name, email)| {
                    menu.item(item(
                        name.clone(),
                        Some(AuthorFilter {
                            label: name.clone(),
                            email: email.clone(),
                        }),
                    ))
                })
            },
        )
    }

    /// Date: every date until one is picked; All, the last 24 hours, the last 7 days, or
    /// days picked in a calendar.
    fn date_filter(
        &self,
        view: &GitGraphView,
        key: ConnectionKey,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> Div {
        let owner = cx.weak_entity();
        let pick: Pick<DateFilter> = Rc::new({
            let owner = owner.clone();
            move |date, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.set_git_graph_date(key, pane_id, date, cx);
                });
            }
        });
        let shown = view.date;
        let clear = (shown != DateFilter::All).then(|| {
            let pick = pick.clone();
            (
                t!("git_graph.date.clear"),
                Rc::new(move |cx: &mut App| pick(DateFilter::All, cx)) as Clear,
            )
        });
        filter_button(
            "git-graph-date-filter",
            t!("git_graph.date.name"),
            shown.label(),
            clear,
            move |menu, _, _| {
                let item = |label: Cow<'static, str>, date: DateFilter| {
                    let pick = pick.clone();
                    PopupMenuItem::new(label)
                        .checked(shown == date)
                        .on_click(move |_, _, cx| pick(date, cx))
                };
                let owner = owner.clone();
                menu.item(item(t!("git_graph.all"), DateFilter::All))
                    .item(item(t!("git_graph.date.last_day"), DateFilter::LastDay))
                    .item(item(t!("git_graph.date.last_week"), DateFilter::LastWeek))
                    .separator()
                    .item(
                        PopupMenuItem::new(t!("git_graph.date.select"))
                            .checked(matches!(shown, DateFilter::Days(..)))
                            .on_click(move |_, window, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    this.open_git_graph_days(key, pane_id, window, cx);
                                });
                            }),
                    )
            },
        )
    }

    /// The days the Date filter keeps, picked in a calendar: the first through the last, or
    /// from one day on.
    fn open_git_graph_days(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.git_graph.views.get(&(key, pane_id)) else {
            return;
        };
        let shown = view.date;
        let picker = cx.new(|cx| {
            let mut picker = DatePickerState::range(window, cx).date_format("%Y/%m/%d");
            if let DateFilter::Days(first, last) = shown {
                picker.set_date(Date::Range(first, last), window, cx);
            }
            picker
        });
        let error = cx.new(|_| None::<String>);
        let owner = cx.weak_entity();
        open_form_dialog(
            t!("git_graph.date.title").into(),
            t!("git_graph.date.apply").into(),
            error.clone(),
            {
                let (picker, error) = (picker.clone(), error.clone());
                move |_, cx| {
                    v_flex()
                        .gap_2()
                        .child(
                            div().debug_selector(|| "git-graph-days".into()).child(
                                DatePicker::new(&picker)
                                    .placeholder(t!("git_graph.date.placeholder"))
                                    .number_of_months(2),
                            ),
                        )
                        .children(field_error(error.read(cx).clone(), cx))
                        .into_any_element()
                }
            },
            move |_, cx| {
                let Date::Range(first, last) = picker.read(cx).date() else {
                    return Err(t!("git_graph.date.pick").into_owned());
                };
                if first.is_none() && last.is_none() {
                    return Err(t!("git_graph.date.pick").into_owned());
                }
                let _ = owner.update(cx, |this, cx| {
                    this.set_git_graph_date(key, pane_id, DateFilter::Days(first, last), cx);
                });
                Ok(())
            },
            window,
            cx,
        );
    }

    fn set_git_graph_author(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        author: Option<AuthorFilter>,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.git_graph.views.get_mut(&(key, pane_id)) else {
            return;
        };
        if view.author == author {
            return;
        }
        view.author = author;
        self.request_git_graph_page(key, pane_id, false);
        cx.notify();
    }

    fn set_git_graph_date(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        date: DateFilter,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.git_graph.views.get_mut(&(key, pane_id)) else {
            return;
        };
        if view.date == date {
            return;
        }
        view.date = date;
        self.request_git_graph_page(key, pane_id, false);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::DateFilter;
    use chrono::{Local, NaiveDate, TimeZone as _};

    #[test]
    fn the_date_filter_keeps_whole_local_days_and_spans_up_to_now() {
        let now = Local.with_ymd_and_hms(2026, 11, 25, 14, 25, 0).unwrap();
        assert_eq!(DateFilter::All.bounds(now), (None, None));
        assert_eq!(
            DateFilter::LastWeek.bounds(now),
            (Some(now.timestamp() - 7 * 24 * 60 * 60), None)
        );
        let day = |d| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        let start = Local.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap();
        let end = Local.with_ymd_and_hms(2026, 11, 20, 23, 59, 59).unwrap();
        assert_eq!(
            DateFilter::Days(Some(day(1)), Some(day(20))).bounds(now),
            (Some(start.timestamp()), Some(end.timestamp()))
        );
        assert_eq!(
            DateFilter::Days(Some(day(1)), None).bounds(now),
            (Some(start.timestamp()), None)
        );
    }
}
