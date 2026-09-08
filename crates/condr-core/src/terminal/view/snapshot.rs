use super::*;

pub(in crate::terminal) fn publish_view(
    revision: &AtomicU64,
    updates: &mpsc::Sender<TerminalUpdate>,
) {
    let revision = revision.fetch_add(1, Ordering::AcqRel) + 1;
    let _ = updates.send(TerminalUpdate::View(revision));
}

/// Lowercase throughout, which makes alacritty's search case-insensitive. A URL runs from a
/// known scheme to whitespace or a quote/bracket; [`trimmed_url_len`] drops what prose adds.
const URL_PATTERN: &str = r#"(https?://|file://|ftp://|git://|gemini://|gopher://|zed://|ipfs:|ipns:|magnet:|mailto:|news:|ssh:)[^\s"'<>`]+"#;

thread_local! {
    static URL_SEARCH: RefCell<RegexSearch> =
        RefCell::new(RegexSearch::new(URL_PATTERN).expect("the URL pattern compiles"));
}

/// Plain-text URLs in the viewport, keyed by viewport cell index.
pub(in crate::terminal) type DetectedLinks = HashMap<u32, SmolStr>;

/// Finds plain-text URLs with alacritty's grid search, which follows soft-wrapped lines and
/// steps over wide characters, then trims trailing punctuation and unbalanced brackets.
pub(in crate::terminal) fn detect_links(terminal: &Terminal, size: TerminalSize) -> DetectedLinks {
    let mut links = DetectedLinks::new();
    let (Some(last_row), Some(last_column)) =
        (size.rows.checked_sub(1), size.columns.checked_sub(1))
    else {
        return links;
    };
    let display_offset = i32::try_from(terminal.grid().display_offset()).unwrap_or(i32::MAX);
    let start = Point::new(Line(-display_offset), Column(0));
    let end = Point::new(
        Line(i32::from(last_row) - display_offset),
        Column(usize::from(last_column)),
    );
    let columns = u32::from(size.columns);
    let spacer = Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER;
    URL_SEARCH.with_borrow_mut(|regex| {
        for found in RegexIter::new(start, end, Direction::Right, terminal, regex) {
            // Every cell of the match in order. Spacers carry no character but stay linked so
            // the run has no holes.
            let mut cells = Vec::new();
            let mut point = *found.start();
            loop {
                let cell = &terminal.grid()[point];
                let row = u32::try_from(point.line.0 + display_offset).unwrap_or(u32::MAX);
                let character = (!cell.flags.intersects(spacer)).then_some(cell.c);
                cells.push((row * columns + point.column.0 as u32, character));
                if point == *found.end() {
                    break;
                }
                point = point.add(terminal, Boundary::None, 1);
            }
            let chars = cells
                .iter()
                .filter_map(|(_, character)| *character)
                .collect::<Vec<_>>();
            let keep = trimmed_url_len(&chars);
            if keep == 0 {
                continue;
            }
            let uri = SmolStr::from(chars[..keep].iter().collect::<String>());
            let mut kept = 0;
            for (index, character) in cells {
                if character.is_some() {
                    if kept == keep {
                        break;
                    }
                    kept += 1;
                }
                links.insert(index, uri.clone());
            }
        }
    });
    links
}

/// The URL's length without trailing punctuation and closing brackets that have no opener:
/// prose puts those after a URL far more often than a URL ends with them.
fn trimmed_url_len(chars: &[char]) -> usize {
    let mut end = chars.len();
    loop {
        match chars[..end].last() {
            Some('.' | ',' | ';' | ':' | '!' | '?') => end -= 1,
            Some(&close @ (')' | ']' | '}')) => {
                let open = match close {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                let span = &chars[..end];
                let count = |wanted| span.iter().filter(|&&c| c == wanted).count();
                if count(open) < count(close) {
                    end -= 1;
                } else {
                    break;
                }
            }
            _ => break,
        }
    }
    end
}

pub(in crate::terminal) fn snapshot_terminal(
    terminal: &Terminal,
    size: TerminalSize,
    revision: u64,
) -> TerminalView {
    let links = detect_links(terminal, size);
    snapshot_terminal_with_links(terminal, size, revision, &links)
}

pub(in crate::terminal) fn snapshot_terminal_with_links(
    terminal: &Terminal,
    size: TerminalSize,
    revision: u64,
    links: &DetectedLinks,
) -> TerminalView {
    let content = terminal.renderable_content();
    let display_offset = content.display_offset;
    let cursor = terminal_cursor(
        content.cursor.point,
        content.cursor.shape,
        terminal.cursor_style().blinking,
        display_offset,
        size,
    );
    let mut cells = vec![blank_cell(); usize::from(size.rows) * usize::from(size.columns)];
    let mut hyperlinks = SnapshotHyperlinks::default();

    for cell in content.display_iter {
        let row = cell.point.line.0 + display_offset as i32;
        let Ok(row) = u16::try_from(row) else {
            continue;
        };
        let Ok(column) = u16::try_from(cell.point.column.0) else {
            continue;
        };
        if row >= size.rows || column >= size.columns {
            continue;
        }
        let index = usize::from(row) * usize::from(size.columns) + usize::from(column);
        cells[index] = terminal_cell(
            cell.cell,
            content.colors,
            &mut hyperlinks,
            links.get(&(index as u32)),
        );
    }

    TerminalView {
        revision,
        size,
        display_offset: u32::try_from(display_offset).unwrap_or(u32::MAX),
        mouse_tracking: TerminalMouseTracking::from_term_mode(*terminal.mode()),
        cells,
        cursor,
        selection: viewport_selection(terminal, size),
    }
}

/// The Server-tracked selection as inclusive viewport cells, or `None` when there is none.
/// A selection that exists but is entirely scrolled out of view is reported one row past
/// the grid (`row == size.rows`): it matches no cell, so nothing is painted, yet the
/// Client still knows a copy would succeed.
pub(in crate::terminal) fn viewport_selection(
    terminal: &Terminal,
    size: TerminalSize,
) -> Option<TerminalSelection> {
    let range = terminal.selection.as_ref()?.to_range(terminal)?;
    let display_offset = i64::try_from(terminal.grid().display_offset()).ok()?;
    let last_row = i64::from(size.rows.checked_sub(1)?);
    let last_column = size.columns.checked_sub(1)?;
    let start_row = i64::from(range.start.line.0) + display_offset;
    let end_row = i64::from(range.end.line.0) + display_offset;
    if end_row < 0 || start_row > last_row {
        let hidden = |column| TerminalPosition {
            row: size.rows,
            column,
            side: TerminalSide::Left,
        };
        return Some(TerminalSelection {
            start: hidden(0),
            end: TerminalPosition {
                side: TerminalSide::Right,
                ..hidden(last_column)
            },
            display_offset: u32::try_from(display_offset).ok()?,
        });
    }
    let (start_row, start_column) = if start_row < 0 {
        (0, 0)
    } else {
        (
            start_row,
            u16::try_from(range.start.column.0).unwrap_or(u16::MAX),
        )
    };
    let (end_row, end_column) = if end_row > last_row {
        (last_row, last_column)
    } else {
        (
            end_row,
            u16::try_from(range.end.column.0).unwrap_or(u16::MAX),
        )
    };
    Some(TerminalSelection {
        start: TerminalPosition {
            row: u16::try_from(start_row).ok()?,
            column: start_column.min(last_column),
            side: TerminalSide::Left,
        },
        end: TerminalPosition {
            row: u16::try_from(end_row).ok()?,
            column: end_column.min(last_column),
            side: TerminalSide::Right,
        },
        display_offset: u32::try_from(display_offset).ok()?,
    })
}

#[derive(Default)]
pub(in crate::terminal) struct SnapshotHyperlinks {
    values: std::collections::HashSet<SmolStr>,
    pointers: HashMap<(usize, usize), Option<SmolStr>>,
    pub(in crate::terminal) bytes: usize,
}

impl SnapshotHyperlinks {
    fn intern(&mut self, uri: &str) -> Option<SmolStr> {
        let pointer = (uri.as_ptr() as usize, uri.len());
        if let Some(uri) = self.pointers.get(&pointer) {
            return uri.clone();
        }
        let canonical = if uri.len() > MAX_TERMINAL_HYPERLINK_URI_BYTES {
            None
        } else if let Some(uri) = self.values.get(uri) {
            Some(uri.clone())
        } else {
            let next_bytes = self.bytes.checked_add(uri.len())?;
            if next_bytes > MAX_TERMINAL_HYPERLINK_BYTES {
                None
            } else {
                let uri = SmolStr::new(uri);
                self.bytes = next_bytes;
                self.values.insert(uri.clone());
                Some(uri)
            }
        };
        self.pointers.insert(pointer, canonical.clone());
        canonical
    }
}

pub(in crate::terminal) fn terminal_cell(
    cell: &Cell,
    colors: &alacritty_terminal::term::color::Colors,
    hyperlinks: &mut SnapshotHyperlinks,
    detected_link: Option<&SmolStr>,
) -> TerminalCell {
    let text = terminal_cell_text(cell);
    let mut flags = cell.flags.bits();
    // An OSC 8 link is the program's word; a detected one only fills in where it said nothing.
    let hyperlink = match (cell.hyperlink(), detected_link) {
        (Some(link), _) => hyperlinks.intern(link.uri()),
        (None, Some(uri)) => {
            let uri = hyperlinks.intern(uri);
            if uri.is_some() {
                flags |= DETECTED_LINK_FLAG;
            }
            uri
        }
        (None, None) => None,
    };
    TerminalCell {
        text,
        foreground: terminal_color(cell.fg, colors),
        background: terminal_color(cell.bg, colors),
        flags,
        hyperlink,
    }
}

pub(in crate::terminal) fn terminal_cell_text(cell: &Cell) -> SmolStr {
    let mut text = SmolStrBuilder::new();
    text.push(cell.c);
    let mut text_bytes = cell.c.len_utf8();
    if let Some(zerowidth) = cell.zerowidth() {
        for &character in zerowidth {
            if text_bytes + character.len_utf8() > MAX_TERMINAL_CELL_TEXT_BYTES {
                break;
            }
            text.push(character);
            text_bytes += character.len_utf8();
        }
    }
    text.finish()
}

pub(in crate::terminal) fn terminal_cursor(
    point: Point,
    shape: CursorShape,
    blinking: bool,
    display_offset: usize,
    size: TerminalSize,
) -> Option<TerminalCursor> {
    let row = point.line.0 + display_offset as i32;
    u16::try_from(row).ok().and_then(|row| {
        let column = u16::try_from(point.column.0).ok()?;
        (row < size.rows && column < size.columns).then_some(TerminalCursor {
            row,
            column,
            shape: shape.into(),
            blinking,
        })
    })
}

pub(in crate::terminal) fn blank_cell() -> TerminalCell {
    TerminalCell {
        text: SmolStr::new_static(" "),
        foreground: TerminalColor::Named(NamedColor::Foreground as u16),
        background: TerminalColor::Named(NamedColor::Background as u16),
        flags: 0,
        hyperlink: None,
    }
}

fn terminal_color(
    color: Color,
    overrides: &alacritty_terminal::term::color::Colors,
) -> TerminalColor {
    let override_color = match color {
        Color::Named(named) => overrides[named],
        Color::Indexed(index) => overrides[usize::from(index)],
        Color::Spec(_) => None,
    };
    let color = override_color.map_or(color, Color::Spec);
    match color {
        Color::Named(named) => TerminalColor::Named(named as u16),
        Color::Indexed(index) => TerminalColor::Indexed(index),
        Color::Spec(rgb) => TerminalColor::Rgb {
            red: rgb.r,
            green: rgb.g,
            blue: rgb.b,
        },
    }
}

impl From<CursorShape> for TerminalCursorShape {
    fn from(shape: CursorShape) -> Self {
        match shape {
            CursorShape::Block => Self::Block,
            CursorShape::Underline => Self::Underline,
            CursorShape::Beam => Self::Beam,
            CursorShape::HollowBlock => Self::HollowBlock,
            CursorShape::Hidden => Self::Hidden,
        }
    }
}

pub(in crate::terminal) fn viewport_point(
    terminal: &Terminal,
    position: TerminalPosition,
    display_offset: u32,
) -> Point {
    let row = usize::from(position.row).min(terminal.screen_lines().saturating_sub(1));
    let column = usize::from(position.column).min(terminal.columns().saturating_sub(1));
    Point::new(
        Line(row as i32 - i32::try_from(display_offset).unwrap_or(i32::MAX)),
        Column(column),
    )
}

pub(in crate::terminal) fn side(side: TerminalSide) -> Side {
    match side {
        TerminalSide::Left => Side::Left,
        TerminalSide::Right => Side::Right,
    }
}
