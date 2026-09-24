//! The directory list under a remote Device's Root Directory field (ADR 0002). The field
//! names a directory and, after its last separator, the name being typed in it: the Server
//! lists the directory once, and each keystroke filters that listing here.

use super::*;
use gpui_kit::component::scroll::ScrollableElement as _;
use std::ops::Range;

actions!(
    directory_browser,
    [SelectPrevious, SelectNext, EnterDirectory]
);

/// The key context around the field, so Up, Down and Tab work the list below it.
const CONTEXT: &str = "DirectoryBrowser";

pub(super) fn key_bindings() -> [KeyBinding; 3] {
    let context = Some("DirectoryBrowser > Input");
    [
        KeyBinding::new("up", SelectPrevious, context),
        KeyBinding::new("down", SelectNext, context),
        KeyBinding::new("tab", EnterDirectory, context),
    ]
}

/// What the open Root Directory dialog lists; the next such dialog replaces it.
pub(super) struct DirectoryBrowser {
    key: ConnectionKey,
    /// The latest request and the directory it asked for; an older answer is dropped.
    request_id: u64,
    requested: String,
    input: WeakEntity<InputState>,
    /// The last answer, with the directory it lists as the field spells it.
    listing: Option<(String, Result<BrowsedDirectory, String>)>,
    highlighted: usize,
    scroll: ScrollHandle,
}

/// One row of the list, with the path it stands for as the Server spells it.
#[derive(Clone, Debug, PartialEq)]
enum Row {
    /// The directory the field names: what Enter takes while no name is typed after it.
    Current(String),
    Parent(String),
    /// With the byte ranges of its name that the typed name matched.
    Directory {
        name: String,
        path: String,
        matched: Vec<Range<usize>>,
    },
}

impl Row {
    fn path(&self) -> &str {
        match self {
            Row::Current(path) | Row::Parent(path) | Row::Directory { path, .. } => path,
        }
    }
}

/// The separators of the Server's platform: a POSIX path starts with `/` and uses only
/// it; anything else is taken as Windows, where `\` and `/` both separate.
fn separators(path: &str) -> &'static [char] {
    if path.starts_with('/') {
        &['/']
    } else {
        &['\\', '/']
    }
}

/// `text` as the directory it names and the name being typed in it.
fn split_query(text: &str) -> (&str, &str) {
    match text.rfind(separators(text)) {
        Some(index) => text.split_at(index + 1),
        None => (text, ""),
    }
}

/// `path` ending in a separator, so the field names it as the directory to list.
fn with_separator(path: &str) -> String {
    if path.ends_with(separators(path)) {
        path.to_owned()
    } else {
        format!("{path}{}", separators(path)[0])
    }
}

/// `path` without the separator a directory may end in, unless it is a root such as `/`
/// or `C:\`.
fn without_separator(path: &str) -> &str {
    let trimmed = path.trim_end_matches(separators(path));
    if trimmed.is_empty() || (!path.starts_with('/') && trimmed.ends_with(':')) {
        path
    } else {
        trimmed
    }
}

fn fold(character: char) -> char {
    character.to_lowercase().next().unwrap_or(character)
}

/// The byte ranges of the characters of `name` that spell `filter` in order, ignoring
/// case; `None` when they do not.
fn match_ranges(name: &str, filter: &str) -> Option<Vec<Range<usize>>> {
    let mut wanted = filter.chars().map(fold).peekable();
    let mut ranges = Vec::new();
    for (start, character) in name.char_indices() {
        if wanted.peek() == Some(&fold(character)) {
            wanted.next();
            ranges.push(start..start + character.len_utf8());
        }
    }
    wanted.peek().is_none().then_some(ranges)
}

fn starts_with_folded(name: &str, filter: &str) -> bool {
    let mut name = name.chars().map(fold);
    filter
        .chars()
        .map(fold)
        .all(|wanted| name.next() == Some(wanted))
}

/// With nothing typed: the directory itself, its parent and every subdirectory. With a
/// name typed: the subdirectories that match it, those starting with it first, each group
/// in the listing's order.
fn rows(browsed: &BrowsedDirectory, filter: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    if filter.is_empty() {
        rows.push(Row::Current(browsed.path.to_string_lossy().into_owned()));
        rows.extend(
            browsed
                .parent
                .iter()
                .map(|parent| Row::Parent(parent.to_string_lossy().into_owned())),
        );
    }
    let mut matches: Vec<(bool, Row)> = browsed
        .listing
        .entries
        .iter()
        .filter_map(|entry| {
            let matched = match_ranges(&entry.name, filter)?;
            let path = absolute_path(&browsed.path, RelativePath::new(&entry.name));
            Some((
                !starts_with_folded(&entry.name, filter),
                Row::Directory {
                    name: entry.name.clone(),
                    path: path.to_string_lossy().into_owned(),
                    matched,
                },
            ))
        })
        .collect();
    matches.sort_by_key(|(later, _)| *later);
    rows.extend(matches.into_iter().map(|(_, row)| row));
    rows
}

impl Condr {
    pub(super) fn open_directory_browser(
        &mut self,
        key: ConnectionKey,
        input: &Entity<InputState>,
        cx: &mut Context<Self>,
    ) {
        self.directory_browser = Some(DirectoryBrowser {
            key,
            request_id: 0,
            requested: String::new(),
            input: input.downgrade(),
            listing: None,
            highlighted: 0,
            scroll: ScrollHandle::new(),
        });
        cx.subscribe(input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let text = input.read(cx).value().to_string();
                this.browser_query_changed(&text);
            }
        })
        .detach();
        self.request_browser_directory(String::new());
    }

    /// Lists the directory the field now names unless it is the one already asked for;
    /// an emptied field keeps the last listing rather than jump back home.
    fn browser_query_changed(&mut self, text: &str) {
        let Some(browser) = self.directory_browser.as_mut() else {
            return;
        };
        browser.highlighted = 0;
        browser.scroll.set_offset(Point::default());
        let (directory, _) = split_query(text);
        if !directory.is_empty() && directory != browser.requested {
            self.request_browser_directory(directory.to_owned());
        }
    }

    fn request_browser_directory(&mut self, directory: String) {
        let Some(key) = self.directory_browser.as_ref().map(|browser| browser.key) else {
            return;
        };
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if connection.status != ConnectionStatus::Connected {
            return;
        }
        let request_id = connection.next_layout_request_id;
        connection.next_layout_request_id = request_id.wrapping_add(1).max(1);
        connection.send(ClientMessage::BrowseDirectory {
            request_id,
            path: PathBuf::from(&directory),
        });
        if let Some(browser) = self.directory_browser.as_mut() {
            browser.request_id = request_id;
            browser.requested = directory;
        }
    }

    pub(super) fn apply_browsed_directory(
        &mut self,
        key: ConnectionKey,
        request_id: u64,
        result: Result<BrowsedDirectory, String>,
        cx: &mut Context<Self>,
    ) {
        let handle = self.window_handle;
        let Some(browser) = self
            .directory_browser
            .as_mut()
            .filter(|browser| browser.key == key && browser.request_id == request_id)
        else {
            return;
        };
        // Only the first request goes out empty; its answer names the home directory,
        // which the field then shows, selected so typing replaces it.
        if let (true, Ok(browsed), Some(input)) = (
            browser.requested.is_empty(),
            &result,
            browser.input.upgrade(),
        ) {
            let home = with_separator(&browsed.path.to_string_lossy());
            browser.requested = home.clone();
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| {
                    input.update(cx, |input, cx| {
                        if input.value().is_empty() {
                            input.set_value(home, window, cx);
                            input.select_all(window, cx);
                        }
                    });
                });
            });
        }
        browser.listing = Some((browser.requested.clone(), result));
        browser.highlighted = 0;
        browser.scroll.set_offset(Point::default());
    }

    /// The rows for `text` and the highlighted one, while the listing is of the directory
    /// `text` names.
    fn browser_rows(&self, text: &str) -> Option<(Vec<Row>, usize)> {
        let browser = self.directory_browser.as_ref()?;
        let (directory, filter) = split_query(text);
        let (listed, Ok(browsed)) = browser.listing.as_ref()? else {
            return None;
        };
        (listed == directory).then(|| (rows(browsed, filter), browser.highlighted))
    }

    /// What Enter submits: the highlighted row, or the text as typed while nothing is
    /// listed for it; a directory's trailing separator is dropped either way.
    pub(super) fn browser_choice(&self, text: &str) -> String {
        let chosen = self
            .browser_rows(text)
            .and_then(|(rows, highlighted)| rows.get(highlighted).map(|row| row.path().to_owned()));
        without_separator(chosen.as_deref().unwrap_or(text)).to_owned()
    }

    fn browser_text(&self, cx: &App) -> Option<(Entity<InputState>, String)> {
        let input = self.directory_browser.as_ref()?.input.upgrade()?;
        let text = input.read(cx).value().to_string();
        Some((input, text))
    }

    fn move_browser_highlight(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some((_, text)) = self.browser_text(cx) else {
            return;
        };
        let Some((rows, highlighted)) = self.browser_rows(&text) else {
            return;
        };
        let highlighted = if forward {
            (highlighted + 1).min(rows.len().saturating_sub(1))
        } else {
            highlighted.saturating_sub(1)
        };
        if let Some(browser) = self.directory_browser.as_mut() {
            browser.highlighted = highlighted;
            browser.scroll.scroll_to_item(highlighted);
        }
        cx.notify();
    }

    /// Puts row `index` in the field as the directory to list, as Tab and a click do.
    fn enter_browser_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((input, text)) = self.browser_text(cx) else {
            return;
        };
        let Some(row) = self
            .browser_rows(&text)
            .and_then(|(rows, _)| rows.into_iter().nth(index))
        else {
            return;
        };
        let text = with_separator(row.path());
        input.update(cx, |input, cx| {
            input.set_value(text.clone(), window, cx);
            input.focus(window, cx);
        });
        self.browser_query_changed(&text);
        cx.notify();
    }
}

/// The field, with Up and Down moving the highlight and Tab entering its directory.
pub(super) fn browser_field(
    owner: &WeakEntity<Condr>,
    input: &Entity<InputState>,
    field: impl IntoElement,
) -> AnyElement {
    let (previous, next, enter) = (owner.clone(), owner.clone(), owner.clone());
    let input = input.clone();
    div()
        .key_context(CONTEXT)
        .on_action(move |_: &SelectPrevious, _, cx| {
            let _ = previous.update(cx, |this, cx| this.move_browser_highlight(false, cx));
        })
        .on_action(move |_: &SelectNext, _, cx| {
            let _ = next.update(cx, |this, cx| this.move_browser_highlight(true, cx));
        })
        .on_action(move |_: &EnterDirectory, window, cx| {
            let text = input.read(cx).value().to_string();
            let _ = enter.update(cx, |this, cx| {
                if let Some((_, highlighted)) = this.browser_rows(&text) {
                    this.enter_browser_row(highlighted, window, cx);
                }
            });
        })
        .child(field)
        .into_any_element()
}

/// The list under the field. A listing of another directory stays until the new one
/// lands, without a highlight, so stepping in does not flash an empty list.
pub(super) fn render_directory_browser(
    owner: &WeakEntity<Condr>,
    input: &Entity<InputState>,
    cx: &App,
) -> Option<AnyElement> {
    let condr = owner.upgrade()?;
    let condr = condr.read(cx);
    let browser = condr.directory_browser.as_ref()?;
    let theme = cx.theme();
    let note = |text: String| {
        div()
            .h_7()
            .px_2()
            .flex()
            .items_center()
            .text_sm()
            .text_color(theme.muted_foreground)
            .truncate()
            .child(text)
            .into_any_element()
    };
    // A path still being typed does not exist yet: that is not an error, so it is muted.
    let (listed, browsed) = match &browser.listing {
        None => return Some(note("Loading…".into())),
        Some((_, Err(reason))) => return Some(note(reason.clone())),
        Some((listed, Ok(browsed))) => (listed, browsed),
    };
    let text = input.read(cx).value();
    let (directory, filter) = split_query(&text);
    let rows = rows(browsed, filter);
    let highlighted = (listed == directory).then_some(browser.highlighted);
    let empty = rows.iter().all(|row| !matches!(row, Row::Directory { .. }));
    let row_elements = rows.into_iter().enumerate().map(|(index, row)| {
        let (id, label): (String, AnyElement) = match &row {
            Row::Current(_) => (
                "directory-browser-current".into(),
                "This directory".into_any_element(),
            ),
            Row::Parent(_) => ("directory-browser-parent".into(), "..".into_any_element()),
            Row::Directory { name, matched, .. } => (
                format!("directory-browser-{name}"),
                StyledText::new(name.clone())
                    .with_highlights(matched.iter().map(|range| {
                        (
                            range.clone(),
                            HighlightStyle {
                                color: Some(theme.blue),
                                ..HighlightStyle::default()
                            },
                        )
                    }))
                    .into_any_element(),
            ),
        };
        let owner = owner.clone();
        h_flex()
            .id(SharedString::from(id.clone()))
            .debug_selector(move || id.clone())
            .h_7()
            .w_full()
            .flex_none()
            .px_2()
            .gap_1()
            .items_center()
            .cursor_pointer()
            .rounded(theme.radius)
            .text_sm()
            .when(highlighted == Some(index), |this| {
                this.bg(theme.list_active)
            })
            .hover(|this| this.bg(theme.list_hover))
            .on_click(move |_, window, cx| {
                let _ = owner.update(cx, |this, cx| this.enter_browser_row(index, window, cx));
            })
            .child(
                img(file_icons::folder_icon(theme.is_dark()))
                    .size_4()
                    .flex_shrink_0(),
            )
            .child(div().min_w_0().flex_1().truncate().child(label))
            .into_any_element()
    });
    Some(
        div()
            .relative()
            .h_64()
            .w_full()
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius)
            .child(
                v_flex()
                    .id("directory-browser")
                    .debug_selector(|| "directory-browser".into())
                    .size_full()
                    .p_1()
                    .overflow_y_scroll()
                    .track_scroll(&browser.scroll)
                    .children(row_elements)
                    .when(empty, |list| {
                        list.child(note(if filter.is_empty() {
                            "No subdirectories".into()
                        } else {
                            "No matching directories".into()
                        }))
                    })
                    .when(browsed.listing.truncated, |list| {
                        list.child(truncated_note(cx))
                    }),
            )
            .vertical_scrollbar(&browser.scroll)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::{Row, match_ranges, rows, split_query, with_separator, without_separator};
    use condr_core::{BrowsedDirectory, DirectoryEntry, DirectoryListing, FileKind};
    use std::path::PathBuf;

    fn browsed(path: &str, parent: Option<&str>, names: &[&str]) -> BrowsedDirectory {
        BrowsedDirectory {
            path: PathBuf::from(path),
            parent: parent.map(PathBuf::from),
            listing: DirectoryListing {
                entries: names
                    .iter()
                    .map(|name| DirectoryEntry {
                        name: (*name).into(),
                        kind: FileKind::Directory,
                        ignored: false,
                    })
                    .collect(),
                truncated: false,
            },
        }
    }

    #[test]
    fn a_query_splits_after_its_platforms_last_separator() {
        assert_eq!(split_query("/home/me/pro"), ("/home/me/", "pro"));
        assert_eq!(split_query("/home/me/"), ("/home/me/", ""));
        assert_eq!(split_query("/a\\b"), ("/", "a\\b"));
        assert_eq!(split_query(r"C:\Users/me\pro"), (r"C:\Users/me\", "pro"));
        assert_eq!(split_query("~"), ("~", ""));
        assert_eq!(with_separator("/home/me"), "/home/me/");
        assert_eq!(with_separator(r"C:\Users"), r"C:\Users\");
        assert_eq!(with_separator("/"), "/");
        assert_eq!(without_separator("/home/me/"), "/home/me");
        assert_eq!(without_separator("/"), "/");
        assert_eq!(without_separator(r"C:\"), r"C:\");
        assert_eq!(without_separator(r"C:\Users\"), r"C:\Users");
    }

    #[test]
    fn names_match_in_order_ignoring_case_and_prefixes_come_first() {
        assert_eq!(
            match_ranges("Workspace", "wsp"),
            Some(vec![0..1, 4..5, 5..6])
        );
        assert_eq!(match_ranges("workspace", "pw"), None);
        // Ranges are bytes, so a highlight never splits a character.
        let ranges = match_ranges("项目", "目").unwrap();
        assert_eq!((ranges.len(), ranges[0].clone()), (1, 3..6));

        let listing = browsed("/home/me", Some("/home"), &["aproject", "docs", "Projects"]);
        let names = |rows: Vec<Row>| -> Vec<String> {
            rows.into_iter()
                .map(|row| match row {
                    Row::Directory { name, .. } => name,
                    other => other.path().to_owned(),
                })
                .collect()
        };
        assert_eq!(
            names(rows(&listing, "")),
            ["/home/me", "/home", "aproject", "docs", "Projects"]
        );
        assert_eq!(names(rows(&listing, "pro")), ["Projects", "aproject"]);
        assert_eq!(names(rows(&listing, "x")), Vec::<String>::new());
        let Row::Directory { path, .. } = &rows(&listing, "doc")[0] else {
            panic!("a directory row");
        };
        assert_eq!(path, "/home/me/docs");
    }
}
