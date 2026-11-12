//! The Command Palette (ADR 0064): every window action and every place Go to reaches, in
//! one dialog over the window it opened in.

use super::keymap::{self, Needs};
use super::sidebar::{CondrSidebarIcon, SidebarStatusVisual, device_status, pane_sidebar_status};
use super::workspace::tab_label;
use super::*;
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::kbd::Kbd;
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use std::cmp::Reverse;

/// The key context around the open palette, which sits outside the window's own.
pub(super) const CONTEXT: &str = "CommandPalette";

/// How many actions the palette remembers, and how many rows each section of All shows.
const SHORT_LIST: usize = 5;

actions!(command_palette, [NextPage, PreviousPage]);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Page {
    #[default]
    All,
    Actions,
    GoTo,
}

impl Page {
    const ALL: [Self; 3] = [Self::All, Self::Actions, Self::GoTo];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Actions => "Actions",
            Self::GoTo => "Go to",
        }
    }

    fn step(self, step: isize) -> Self {
        let index = Self::ALL.iter().position(|page| *page == self).unwrap_or(0);
        Self::ALL[(index as isize + step).rem_euclid(Self::ALL.len() as isize) as usize]
    }
}

/// Opens the Command Palette on `page`; pressed in the open palette, it closes it.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = condr, no_json)]
pub(super) struct OpenCommandPalette {
    pub(super) page: Page,
}

pub(super) fn key_bindings() -> [KeyBinding; 4] {
    // GPUI makes a Shift pressed and let go alone a keystroke of its own, and binds two
    // of them within its one-second wait for the rest of a sequence.
    let all = || OpenCommandPalette { page: Page::All };
    let input = Some("CommandPalette > Input");
    [
        KeyBinding::new("shift shift", all(), Some(SHORTCUT_CONTEXT)),
        KeyBinding::new("shift shift", all(), Some(CONTEXT)),
        KeyBinding::new("tab", NextPage, input),
        KeyBinding::new("shift-tab", PreviousPage, input),
    ]
}

/// What a row of the palette leads to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    /// A window action, by its place in `keymap::all()`.
    Action(usize),
    Agent(ConnectionKey, PaneId),
    Tab(ConnectionKey, WorkspaceId, TabId),
    Workspace(ConnectionKey, WorkspaceId),
    Device(ConnectionKey),
}

impl Target {
    fn is_action(self) -> bool {
        matches!(self, Self::Action(_))
    }
}

/// How a row looks besides its text.
#[derive(Clone)]
enum Look {
    Action(Option<Keystroke>),
    Agent(AgentKind, SidebarStatusVisual),
    Tab {
        viewer: bool,
    },
    Workspace {
        root: String,
    },
    Device {
        status: ConnectionStatus,
        other_build: bool,
        failed: bool,
    },
}

/// One row before matching: the text it is found and highlighted by, the text it is
/// found by only, and what it shows besides.
#[derive(Clone)]
pub(super) struct Entry {
    pub(super) target: Target,
    pub(super) label: String,
    /// Matched when `label` is not, below every `label` match and without highlights.
    pub(super) also: String,
    detail: String,
    enabled: bool,
    look: Look,
}

/// A row that matched, with the byte ranges of its label to highlight.
pub(super) type Ranked = (Entry, Vec<Range<usize>>);

/// A heading, if the page shows one, and its rows.
type Section = (Option<&'static str>, Vec<Ranked>);

/// `entries` that match `query`, best first, each with the byte ranges of its label to
/// highlight. A label match beats a match of `also`; ties keep the order given.
pub(super) fn rank(entries: Vec<Entry>, query: &str) -> Vec<Ranked> {
    let pattern = Pattern::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    if pattern.atoms.is_empty() {
        return entries
            .into_iter()
            .map(|entry| (entry, Vec::new()))
            .collect();
    }
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buffer = Vec::new();
    let mut matched = entries
        .into_iter()
        .enumerate()
        .filter_map(|(order, entry)| {
            let mut indices = Vec::new();
            let label = Utf32Str::new(&entry.label, &mut buffer);
            if let Some(score) = pattern.indices(label, &mut matcher, &mut indices) {
                let ranges = byte_ranges(&entry.label, indices);
                return Some(((0, Reverse(score), order), entry, ranges));
            }
            let score = pattern.score(Utf32Str::new(&entry.also, &mut buffer), &mut matcher)?;
            Some(((1, Reverse(score), order), entry, Vec::new()))
        })
        .collect::<Vec<_>>();
    matched.sort_by_key(|(key, ..)| *key);
    matched
        .into_iter()
        .map(|(_, entry, ranges)| (entry, ranges))
        .collect()
}

/// nucleo's character indices as the byte ranges `StyledText` highlights.
fn byte_ranges(text: &str, mut indices: Vec<u32>) -> Vec<Range<usize>> {
    indices.sort_unstable();
    indices.dedup();
    let mut wanted = indices.into_iter().peekable();
    text.char_indices()
        .enumerate()
        .filter_map(|(index, (start, character))| {
            (wanted.next_if_eq(&(index as u32))).map(|_| start..start + character.len_utf8())
        })
        .collect()
}

impl Condr {
    pub(super) fn action_open_command_palette(
        &mut self,
        action: &OpenCommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            return;
        }
        let owner = cx.weak_entity();
        let palette = cx.new(|cx| CommandPalette::new(owner, action.page, window, cx));
        let content = palette.clone();
        window.open_dialog(cx, move |dialog, window, _| {
            let content = content.clone();
            dialog
                .w(rems(40.).to_pixels(window.rem_size()))
                .close_button(false)
                // Summoned by the keyboard, it is there at once for the typing that follows.
                .entrance(gpui_kit::component::dialog::DialogEntrance::None)
                .content(move |body, _, _| body.child(content.clone()))
        });
        let command = palette.read(cx).command.clone();
        window.defer(cx, move |window, cx| {
            command.update(cx, |command, cx| command.focus(window, cx));
        });
    }

    /// Whether an action needing `needs` would do anything in the active window.
    pub(super) fn can_run(&self, needs: Needs) -> bool {
        let can_change = self
            .connection(self.input_connection())
            .is_some_and(ServerConnection::can_mutate);
        match needs {
            Needs::Nothing => true,
            Needs::Pane => can_change && self.focused_pane().is_some(),
            Needs::Tab => self.presented().is_some(),
            Needs::Change => can_change && self.presented().is_some(),
            Needs::Device => can_change,
            Needs::Workspaces => self.connections.iter().any(|connection| {
                connection
                    .presented_session()
                    .is_some_and(|session| !session.workspaces().is_empty())
            }),
            Needs::Changes => {
                self.presented_workspace().is_some() && self.draws_workspace(self.active_view())
            }
        }
    }

    /// The window actions, the last chosen first when `recent_first`. Go to tab N is left
    /// to Go to, and the palette's own keys to the palette.
    fn action_entries(&self, recent_first: bool) -> Vec<Entry> {
        let mut entries = keymap::GROUPS
            .iter()
            .flat_map(|(group, shortcuts)| shortcuts.iter().map(move |shortcut| (*group, shortcut)))
            .enumerate()
            .filter(|(_, (_, shortcut))| {
                let action = shortcut.action.as_any();
                !action.is::<ActivateTab>() && !action.is::<OpenCommandPalette>()
            })
            .map(|(index, (group, shortcut))| Entry {
                target: Target::Action(index),
                label: shortcut.label.to_owned(),
                also: format!("{group} {}", shortcut.label),
                detail: group.to_owned(),
                enabled: self.can_run(shortcut.needs),
                look: Look::Action(self.keymap.keys[index].first().cloned()),
            })
            .collect::<Vec<_>>();
        if recent_first {
            let recency = |entry: &Entry| match entry.target {
                Target::Action(index) => self
                    .recent_actions
                    .iter()
                    .position(|&recent| recent == index),
                _ => None,
            };
            entries.sort_by_key(|entry| recency(entry).map_or((1, 0), |position| (0, position)));
        }
        entries
    }

    /// Every place on every connected Device, in the order an empty query lists them: the
    /// Agents most urgent first, then the Tabs, Workspaces and Devices as the sidebar
    /// lists them.
    pub(super) fn place_entries(&self) -> Vec<Entry> {
        let several_devices = self.connections.len() > 1;
        let (mut agents, mut tabs, mut workspaces, mut devices) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for connection in &self.connections {
            let key = connection.key;
            let device = if several_devices {
                format!(" · {}", connection.label)
            } else {
                String::new()
            };
            devices.push(Entry {
                target: Target::Device(key),
                label: connection.label.clone(),
                also: connection.label.clone(),
                detail: match connection.status {
                    ConnectionStatus::Connected => "Connected",
                    ConnectionStatus::Connecting => "Connecting…",
                    ConnectionStatus::Disconnected => "Not connected",
                }
                .to_owned(),
                enabled: true,
                look: Look::Device {
                    status: connection.status,
                    other_build: connection.build_notice().is_some(),
                    failed: connection.error.is_some(),
                },
            });
            // A Device not connected lists itself only, as the sidebar does (ADR 0020).
            let Some(session) = connection.presented_session() else {
                continue;
            };
            for workspace in session.workspaces() {
                let workspace_id = workspace.id();
                let name = workspace.name();
                let root = workspace.root_directory().to_string_lossy().into_owned();
                workspaces.push(Entry {
                    target: Target::Workspace(key, workspace_id),
                    label: name.to_owned(),
                    also: format!("{name} {root}"),
                    detail: format!("{root}{device}"),
                    enabled: true,
                    look: Look::Workspace { root },
                });
                for (index, tab) in workspace.tabs().iter().enumerate() {
                    // An unnamed terminal Tab shows its focused Pane's title, as the strip does.
                    let title = tab
                        .name()
                        .is_empty()
                        .then(|| tab.focused_pane())
                        .flatten()
                        .and_then(|pane| connection.pane_title(pane.id()));
                    let tab_text = match &title {
                        Some(title) => format!("{}  {title}", index + 1),
                        None => tab_label(index, tab.name()),
                    };
                    tabs.push(Entry {
                        target: Target::Tab(key, workspace_id, tab.id()),
                        label: tab_text.clone(),
                        also: format!("{tab_text} {name}"),
                        detail: format!("{name}{device}"),
                        enabled: true,
                        look: Look::Tab {
                            viewer: !matches!(tab.content(), TabContent::Terminals(_)),
                        },
                    });
                    for pane in tab.panes() {
                        let pane_id = pane.id();
                        let (Some(agent), Some(status)) = (
                            connection.model.agents.get(&pane_id),
                            pane_sidebar_status(&connection.model, pane_id),
                        ) else {
                            continue;
                        };
                        let kind = agent.kind.label();
                        // The Pane's title is what the agent is doing; its kind is the
                        // fallback before a title arrives, as in the sidebar.
                        let title = connection
                            .model
                            .terminal_titles
                            .get(&pane_id)
                            .cloned()
                            .unwrap_or_else(|| kind.to_owned());
                        agents.push((
                            sidebar::urgency(status),
                            Entry {
                                target: Target::Agent(key, pane_id),
                                also: format!("{title} {kind} {name} {tab_text}"),
                                label: title,
                                detail: format!("{name} › {tab_text}{device}"),
                                enabled: true,
                                look: Look::Agent(agent.kind, status),
                            },
                        ));
                    }
                }
            }
        }
        agents.sort_by_key(|(urgency, _)| *urgency);
        agents
            .into_iter()
            .map(|(_, entry)| entry)
            .chain(tabs)
            .chain(workspaces)
            .chain(devices)
            .collect()
    }

    /// Takes the palette's choice in `window`, which it has just left.
    pub(super) fn run_palette_target(
        &mut self,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            Target::Action(index) => {
                self.recent_actions.retain(|recent| *recent != index);
                self.recent_actions.insert(0, index);
                self.recent_actions.truncate(SHORT_LIST);
                // The action runs where its key would: from the window's focus.
                let focus = self.window_view(window).focus_handle.clone();
                if !focus.contains_focused(window, cx) {
                    focus.focus(window, cx);
                }
                let action = keymap::all().nth(index).expect("a shortcut").action;
                window.dispatch_action(action.boxed_clone(), cx);
            }
            Target::Agent(key, pane_id) => {
                self.select_pane(key, pane_id, window, cx);
            }
            Target::Tab(key, workspace_id, tab_id) => {
                self.go_to_place(key, workspace_id, Some(tab_id), window, cx);
            }
            Target::Workspace(key, workspace_id) => {
                self.go_to_place(key, workspace_id, None, window, cx);
            }
            Target::Device(key) => self.go_to_device(key, window, cx),
        }
    }

    /// Shows a Tab, or a Workspace's Tab, in the window that draws it, with the keyboard
    /// in its focused Pane when it has one.
    fn go_to_place(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: Option<TabId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane_id = self.connection(key).and_then(|connection| {
            let session = connection.session()?;
            let tab_id = tab_id.or_else(|| connection.viewed_tab_id(session, workspace_id))?;
            Some(session.tab(tab_id)?.focused_pane()?.id())
        });
        if pane_id.is_some_and(|pane_id| self.select_pane(key, pane_id, window, cx)) {
            return;
        }
        self.show_and_bring_forward(key, workspace_id, tab_id, cx);
        self.rebuild_dock(window, cx);
        cx.notify();
    }

    /// Presents a Device in the window that draws it: its own, or the main window.
    fn go_to_device(&mut self, key: ConnectionKey, window: &mut Window, cx: &mut Context<Self>) {
        let home = self.home_window(key, None, None);
        if home == self.main_window() && self.active_connection != key {
            self.active_connection = key;
            self.refresh_target_pane(key);
        }
        if let Some(view) = self.windows.iter_mut().find(|view| view.handle == home) {
            view.ports = None;
        }
        self.activate_window(home, cx);
        self.rebuild_dock(window, cx);
        cx.notify();
    }
}

/// The open palette: which page it shows and what each row of the last list stands for.
pub(super) struct CommandPalette {
    owner: WeakEntity<Condr>,
    page: Page,
    pub(super) command: Entity<CommandState>,
    /// By section and row, as Kit's `IndexPath` names them.
    targets: Vec<Vec<Target>>,
}

impl CommandPalette {
    fn new(
        owner: WeakEntity<Condr>,
        page: Page,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            owner,
            page,
            command: cx.new(|cx| CommandState::new(window, cx)),
            targets: Vec::new(),
        }
    }

    fn turn_page(&mut self, step: isize, cx: &mut Context<Self>) {
        self.page = self.page.step(step);
        cx.notify();
    }

    fn confirm(&mut self, index: IndexPath, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self
            .targets
            .get(index.section)
            .and_then(|section| section.get(index.row))
            .copied()
        else {
            return;
        };
        window.close_dialog(cx);
        let _ = self
            .owner
            .update(cx, |condr, cx| condr.run_palette_target(target, window, cx));
    }

    /// The page's sections for `query`, each a heading and its rows.
    fn sections(&self, condr: &Condr, query: &str) -> Vec<Section> {
        let searching = !query.trim().is_empty();
        match self.page {
            Page::All => {
                let places = condr.place_entries().into_iter();
                let places: Vec<Entry> = if searching {
                    places.collect()
                } else {
                    places
                        .filter(|entry| matches!(entry.target, Target::Agent(..)))
                        .collect()
                };
                let actions = condr.action_entries(true);
                let actions = if searching {
                    actions
                } else {
                    actions
                        .into_iter()
                        .take(condr.recent_actions.len())
                        .collect()
                };
                [("Go to", places), ("Actions", actions)]
                    .into_iter()
                    .map(|(heading, entries)| {
                        let mut ranked = rank(entries, query);
                        ranked.truncate(SHORT_LIST);
                        (Some(heading), ranked)
                    })
                    .collect()
            }
            Page::GoTo => vec![(None, rank(condr.place_entries(), query))],
            Page::Actions if searching => vec![(None, rank(condr.action_entries(true), query))],
            Page::Actions => {
                let entries = condr.action_entries(true);
                let (recent, rest) =
                    entries.split_at(condr.recent_actions.len().min(entries.len()));
                let rows = |entries: Vec<&Entry>| -> Vec<_> {
                    entries
                        .into_iter()
                        .map(|entry| (entry.clone(), Vec::new()))
                        .collect()
                };
                let mut sections = Vec::new();
                if !recent.is_empty() {
                    sections.push((Some("Recent"), rows(recent.iter().collect())));
                }
                for (group, _) in keymap::GROUPS {
                    let members = rest
                        .iter()
                        .filter(|entry| entry.detail == *group)
                        .collect::<Vec<_>>();
                    if !members.is_empty() {
                        sections.push((Some(*group), rows(members)));
                    }
                }
                sections
            }
        }
    }
}

impl Render for CommandPalette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.command.read(cx).query(cx);
        let grouped = self.page == Page::Actions && query.trim().is_empty();
        let sections = match self.owner.upgrade() {
            Some(condr) => self.sections(condr.read(cx), &query),
            None => Vec::new(),
        };
        self.targets = sections
            .iter()
            .map(|(_, rows)| rows.iter().map(|(entry, _)| entry.target).collect())
            .collect();
        let groups = sections.into_iter().map(|(heading, rows)| {
            let items = rows.into_iter().map(|(entry, ranges)| {
                let label = entry.label.clone();
                CommandItem::new()
                    .label(label)
                    .disabled(!entry.enabled)
                    .child(move |_, cx| render_row(&entry, &ranges, grouped, cx))
            });
            let group = CommandGroup::new().items(items);
            match heading {
                Some(heading) => group.label(heading),
                None => group,
            }
        });
        let this = cx.entity().downgrade();
        let (query_owner, confirm_owner) = (this.clone(), this.clone());
        let theme = cx.theme();
        let tabs = h_flex()
            .gap_1()
            .pb_2()
            .items_center()
            .children(Page::ALL.map(|page| {
                let owner = this.clone();
                Button::new(SharedString::from(format!(
                    "command-palette-page-{}",
                    page.label()
                )))
                .debug_selector(move || format!("command-palette-page-{}", page.label()))
                .label(page.label())
                .ghost()
                .xsmall()
                .selected(page == self.page)
                .on_click(move |_, window, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.page = page;
                        // Back to the field, so typing goes on.
                        this.command
                            .update(cx, |command, cx| command.focus(window, cx));
                        cx.notify();
                    });
                })
            }))
            .child(div().flex_1())
            .child(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(Kbd::new(Keystroke::parse("tab").expect("a key")))
                    .child("next page"),
            );
        let page = self.page;
        v_flex()
            .debug_selector(|| "command-palette".into())
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &NextPage, _, cx| this.turn_page(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousPage, _, cx| this.turn_page(-1, cx)))
            .on_action(|_: &OpenCommandPalette, window, cx| window.close_dialog(cx))
            .child(tabs)
            .child(
                Command::new(&self.command)
                    .bordered(false)
                    .filterable(false)
                    .max_h(rems(24.))
                    .placeholder(match page {
                        Page::All => "Search actions, Workspaces, Tabs and Agents…",
                        Page::Actions => "Search actions…",
                        Page::GoTo => "Go to a Device, Workspace, Tab or Agent…",
                    })
                    .on_query(move |_, _, cx| {
                        let _ = query_owner.update(cx, |_, cx| cx.notify());
                    })
                    .on_confirm(move |index, window, cx| {
                        let _ =
                            confirm_owner.update(cx, |this, cx| this.confirm(index, window, cx));
                    })
                    .empty(|state, _, cx| {
                        let text = if state.query(cx).trim().is_empty() {
                            "Type to find an action or a place.".to_owned()
                        } else {
                            format!("Nothing matches “{}”.", state.query(cx).trim())
                        };
                        div()
                            .py_6()
                            .w_full()
                            .text_center()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(text)
                    })
                    .map(|command| groups.fold(command, Command::group)),
            )
    }
}

/// A row: its mark, its label with the matched characters picked out, what it belongs
/// to, and an action's key or an Agent's state at the end.
fn render_row(entry: &Entry, ranges: &[Range<usize>], grouped: bool, cx: &mut App) -> AnyElement {
    let theme = cx.theme();
    let (muted, highlight) = (theme.muted_foreground, theme.blue);
    let label = StyledText::new(entry.label.clone()).with_highlights(ranges.iter().map(|range| {
        (
            range.clone(),
            HighlightStyle {
                color: Some(highlight),
                font_weight: Some(FontWeight::SEMIBOLD),
                ..HighlightStyle::default()
            },
        )
    }));
    let selector = match entry.target {
        Target::Action(index) => format!(
            "command-palette-action-{}",
            keymap::all()
                .nth(index)
                .map_or("", |shortcut| shortcut.name)
        ),
        Target::Agent(key, pane_id) => format!("command-palette-agent-{key}-{}", pane_id.as_u64()),
        Target::Tab(key, _, tab_id) => format!("command-palette-tab-{key}-{}", tab_id.as_u64()),
        Target::Workspace(key, workspace_id) => {
            format!("command-palette-workspace-{key}-{}", workspace_id.as_u64())
        }
        Target::Device(key) => format!("command-palette-device-{key}"),
    };
    let mark: Option<AnyElement> = match &entry.look {
        Look::Action(_) => None,
        Look::Agent(kind, status) => Some(
            CondrSidebarIcon::agent(*kind, *status, format!("{selector}-mark"), status.label)
                .render(cx),
        ),
        Look::Tab { viewer } => Some(
            Icon::new(if *viewer {
                Glyph::FileText
            } else {
                Glyph::SquareTerminal
            })
            .size_4()
            .text_color(muted)
            .into_any_element(),
        ),
        Look::Workspace { root } => Some(
            CondrSidebarIcon::avatar(&entry.label, root, format!("{selector}-mark")).render(cx),
        ),
        Look::Device {
            status,
            other_build,
            failed,
        } => {
            let Target::Device(key) = entry.target else {
                unreachable!("a Device row leads to its Device")
            };
            Some(device_status(key, *status, *other_build, *failed, cx))
        }
    };
    let theme = cx.theme();
    let detail = (!(grouped && entry.target.is_action())).then(|| entry.detail.clone());
    let end: Option<AnyElement> = match &entry.look {
        Look::Action(key) => key.clone().map(|key| Kbd::new(key).into_any_element()),
        Look::Agent(_, status) => Some(
            div()
                .text_xs()
                .text_color(status.tone.color(cx))
                .child(status.label)
                .into_any_element(),
        ),
        _ => None,
    };
    h_flex()
        .debug_selector(move || selector.clone())
        .w_full()
        .min_w_0()
        .gap_2()
        .items_center()
        .children(mark.map(|mark| {
            div()
                .flex_none()
                .w(rems(1.25))
                .flex()
                .justify_center()
                .child(mark)
        }))
        .child(div().flex_none().max_w(rems(20.)).truncate().child(label))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(theme.muted_foreground)
                .children(detail),
        )
        .children(end.map(|end| div().flex_none().child(end)))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{Entry, Look, Target, rank};
    use condr_core::WorkspaceId;

    fn entry(label: &str, also: &str) -> Entry {
        Entry {
            target: Target::Workspace(1, WorkspaceId::from_u64(1)),
            label: label.to_owned(),
            also: also.to_owned(),
            detail: String::new(),
            enabled: true,
            look: Look::Tab { viewer: false },
        }
    }

    fn labels(ranked: &[super::Ranked]) -> Vec<&str> {
        ranked
            .iter()
            .map(|(entry, _)| entry.label.as_str())
            .collect()
    }

    #[test]
    fn an_empty_query_keeps_every_row_in_order() {
        let ranked = rank(vec![entry("b", "b"), entry("a", "a")], "  ");
        assert_eq!(labels(&ranked), ["b", "a"]);
        assert!(ranked.iter().all(|(_, ranges)| ranges.is_empty()));
    }

    #[test]
    fn a_label_match_ranks_above_a_match_of_the_rest() {
        let entries = vec![
            entry("Rename tab", "Tabs zoom Rename tab"),
            entry("Toggle zoom", "Panes Toggle zoom"),
            entry("Close pane", "Panes Close pane"),
        ];
        let ranked = rank(entries.clone(), "zoom");
        assert_eq!(labels(&ranked), ["Toggle zoom", "Rename tab"]);
        assert!(!ranked[0].1.is_empty(), "the label match is highlighted");
        assert!(ranked[1].1.is_empty(), "the rest is matched only");
        // Words match anywhere and all must: the group finds the label it lacks.
        assert_eq!(labels(&rank(entries, "pane zoom")), ["Toggle zoom"]);
    }

    #[test]
    fn equal_matches_keep_the_order_given() {
        let ranked = rank(
            vec![entry("Split right", ""), entry("Split down", "")],
            "split",
        );
        assert_eq!(labels(&ranked), ["Split right", "Split down"]);
    }

    #[test]
    fn highlights_are_byte_ranges_of_the_matched_characters() {
        let ranked = rank(vec![entry("日本 Tab", "")], "TAB");
        let tab = "日本 ".len();
        assert_eq!(
            ranked[0].1,
            [tab..tab + 1, tab + 1..tab + 2, tab + 2..tab + 3]
        );
    }
}
