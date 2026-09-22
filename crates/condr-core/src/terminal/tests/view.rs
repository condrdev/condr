use super::*;

fn frame_test_view(revision: u64, text: &str) -> TerminalView {
    let mut cells = text
        .chars()
        .map(|character| TerminalCell {
            text: SmolStr::from(character.to_string()),
            ..blank_cell()
        })
        .collect::<Vec<_>>();
    let columns = u16::try_from(cells.len()).unwrap();
    if cells.is_empty() {
        cells.push(blank_cell());
    }
    TerminalView {
        selection: None,
        revision,
        size: TerminalSize::new(1, columns.max(1)),
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cells,
        cursor: None,
    }
}

#[test]
fn decscusr_shape_and_blink_reach_the_snapshot_cursor() {
    let size = TerminalSize::new(1, 4);
    let (event_proxy, _pending_replies, _notices) =
        TerminalEventProxy::new(Arc::new(Mutex::new(size)));
    let mut terminal = Term::new(Config::default(), &size, event_proxy);
    let mut parser: Processor = Processor::new();

    let cursor = |terminal: &Terminal| snapshot_terminal(terminal, size, 1).cursor.unwrap();
    assert_eq!(cursor(&terminal).shape, TerminalCursorShape::Block);
    assert!(!cursor(&terminal).blinking, "default cursor stays steady");

    parser.advance(&mut terminal, b"\x1b[5 q");
    assert_eq!(cursor(&terminal).shape, TerminalCursorShape::Beam);
    assert!(cursor(&terminal).blinking);

    parser.advance(&mut terminal, b"\x1b[4 q");
    assert_eq!(cursor(&terminal).shape, TerminalCursorShape::Underline);
    assert!(!cursor(&terminal).blinking);

    parser.advance(&mut terminal, b"\x1b[?25l");
    assert_eq!(cursor(&terminal).shape, TerminalCursorShape::Hidden);
}

#[test]
fn osc8_hyperlinks_are_carried_per_cell() {
    let size = TerminalSize::new(1, 4);
    let (event_proxy, _pending_replies, _notices) =
        TerminalEventProxy::new(Arc::new(Mutex::new(size)));
    let mut terminal = Term::new(Config::default(), &size, event_proxy);
    let mut parser: Processor = Processor::new();
    parser.advance(
        &mut terminal,
        b"\x1b]8;;https://example.com/a-long-shared-target\x1b\\ab\x1b]8;;\x1b\\c",
    );

    let view = snapshot_terminal(&terminal, size, 1);
    let link = |column| view.cell(0, column).unwrap().hyperlink.clone();
    assert_eq!(
        link(0).as_deref(),
        Some("https://example.com/a-long-shared-target")
    );
    assert_eq!(
        link(1).as_deref(),
        Some("https://example.com/a-long-shared-target")
    );
    assert_eq!(
        link(0).unwrap().as_ptr(),
        link(1).unwrap().as_ptr(),
        "linked cells in one snapshot must share long URI storage"
    );
    assert_eq!(link(2), None);
}

#[test]
fn terminal_hyperlinks_stay_within_the_frame_byte_budget() {
    let size = TerminalSize::new(1, 1);
    let (event_proxy, _pending_replies, _notices) =
        TerminalEventProxy::new(Arc::new(Mutex::new(size)));
    let terminal = Term::new(Config::default(), &size, event_proxy);
    let mut hyperlinks = SnapshotHyperlinks::default();
    hyperlinks.bytes = MAX_TERMINAL_HYPERLINK_BYTES - 3;
    let mut cell = Cell::default();
    cell.set_hyperlink(Some(Hyperlink::new(Some("first"), "abc".into())));

    assert_eq!(
        terminal_cell(&cell, terminal.colors(), &mut hyperlinks, None)
            .hyperlink
            .as_deref(),
        Some("abc")
    );

    cell.set_hyperlink(Some(Hyperlink::new(Some("new-id"), "abc".into())));
    assert_eq!(
        terminal_cell(&cell, terminal.colors(), &mut hyperlinks, None)
            .hyperlink
            .as_deref(),
        Some("abc")
    );

    cell.set_hyperlink(Some(Hyperlink::new(Some("second"), "d".into())));
    assert_eq!(
        terminal_cell(&cell, terminal.colors(), &mut hyperlinks, None).hyperlink,
        None
    );

    cell.set_hyperlink(Some(Hyperlink::new(
        Some("oversized"),
        "x".repeat(MAX_TERMINAL_HYPERLINK_URI_BYTES + 1),
    )));
    assert_eq!(
        terminal_cell(&cell, terminal.colors(), &mut hyperlinks, None).hyperlink,
        None
    );
}

#[test]
fn server_selection_follows_scrolled_output_and_still_copies_it() {
    let size = TerminalSize::new(3, 10);
    let (event_proxy, _replies, _notices) = TerminalEventProxy::new(Arc::new(Mutex::new(size)));
    let mut terminal = Term::new(Config::default(), &size, event_proxy);
    let mut parser: Processor = Processor::new();
    parser.advance(&mut terminal, b"alpha\r\nbeta\r\ngamma");
    let position = |row, column, side| TerminalPosition { row, column, side };
    let selection = TerminalSelection {
        start: position(1, 0, TerminalSide::Left),
        end: position(1, 3, TerminalSide::Right),
        display_offset: 0,
    };

    terminal.selection = Some(super::runtime::grid_selection(&terminal, selection));
    assert_eq!(viewport_selection(&terminal, size), Some(selection));
    assert_eq!(terminal.selection_to_string().as_deref(), Some("beta"));

    parser.advance(&mut terminal, b"\r\ndelta");
    let moved = viewport_selection(&terminal, size).unwrap();
    assert_eq!((moved.start.row, moved.end.row), (0, 0));
    assert_eq!(terminal.selection_to_string().as_deref(), Some("beta"));

    parser.advance(&mut terminal, b"\r\nepsilon");
    // Scrolled out of view: still reported (so a copy is offered) but matches no cell.
    let hidden = viewport_selection(&terminal, size).unwrap();
    assert_eq!((hidden.start.row, hidden.end.row), (size.rows, size.rows));
    assert!(hidden.selected_cell_range(size.columns).is_some());
    assert!((0..size.rows).all(|row| !hidden.contains_cell(row, 0, size.columns)));
    assert_eq!(terminal.selection_to_string().as_deref(), Some("beta"));

    terminal.selection = None;
    assert_eq!(viewport_selection(&terminal, size), None);
}

#[test]
fn terminal_cell_text_keeps_short_content_unchanged() {
    let mut cell = Cell {
        c: 'e',
        ..Cell::default()
    };
    cell.push_zerowidth('\u{301}');
    cell.push_zerowidth('\u{327}');

    assert_eq!(terminal_cell_text(&cell), "e\u{301}\u{327}");
}

#[test]
fn terminal_cell_text_truncates_long_combining_sequences() {
    let mut cell = Cell {
        c: 'a',
        ..Cell::default()
    };
    for _ in 0..200 {
        cell.push_zerowidth('\u{301}');
    }

    let text = terminal_cell_text(&cell);
    assert_eq!(text.len(), 255);
    assert_eq!(text.chars().count(), 128);
    assert_eq!(text.chars().next(), Some('a'));
    assert!(text.chars().skip(1).all(|character| character == '\u{301}'));
}

#[test]
fn terminal_cell_text_truncation_preserves_utf8_boundaries_and_base() {
    let mut cell = Cell {
        c: '\u{754c}',
        ..Cell::default()
    };
    for _ in 0..100 {
        cell.push_zerowidth('\u{1d165}');
    }

    let text = terminal_cell_text(&cell);
    assert_eq!(text.len(), 255);
    assert_eq!(text.chars().count(), 64);
    assert_eq!(text.chars().next(), Some('\u{754c}'));
    assert_eq!(std::str::from_utf8(text.as_bytes()).unwrap(), text.as_str());
}

#[test]
fn terminal_frame_wire_drops_hyperlinks_over_budget_without_losing_text() {
    let budgeted_links = MAX_TERMINAL_HYPERLINK_BYTES / MAX_TERMINAL_HYPERLINK_URI_BYTES;
    let mut view = frame_test_view(7, &"x".repeat(budgeted_links + 2));
    view.cells[0].hyperlink = Some("x".repeat(MAX_TERMINAL_HYPERLINK_URI_BYTES + 1).into());
    for (index, cell) in view.cells[1..=budgeted_links].iter_mut().enumerate() {
        let prefix = format!("https://example.com/{index}/");
        cell.hyperlink = Some(
            format!(
                "{prefix}{}",
                "x".repeat(MAX_TERMINAL_HYPERLINK_URI_BYTES - prefix.len())
            )
            .into(),
        );
    }
    let prefix = "https://example.com/overflow/";
    view.cells.last_mut().unwrap().hyperlink = Some(
        format!(
            "{prefix}{}",
            "x".repeat(MAX_TERMINAL_HYPERLINK_URI_BYTES - prefix.len())
        )
        .into(),
    );
    let delta = TerminalViewDelta {
        selection: None,
        base_revision: 6,
        revision: 7,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: vec![TerminalCellRun {
            start: 0,
            cells: view.cells.clone(),
        }],
    };

    for frame in [
        TerminalViewFrame::Full(view),
        TerminalViewFrame::Delta(delta),
    ] {
        let mut projected = frame.clone();
        assert!(projected.normalize_hyperlinks_for_wire());
        let encoded = bincode::serialize(&frame).unwrap();
        assert!(encoded.len() <= crate::protocol::MAX_CHUNKED_RECORD_SIZE);
        let decoded = bincode::deserialize::<TerminalViewFrame>(&encoded).unwrap();
        assert_eq!(decoded, projected);
        let cells = match &decoded {
            TerminalViewFrame::Full(view) => &view.cells,
            TerminalViewFrame::Delta(delta) => &delta.runs[0].cells,
        };

        assert!(cells.iter().all(|cell| cell.text == "x"));
        assert!(cells[0].hyperlink.is_none());
        assert!(
            cells[1..=budgeted_links]
                .iter()
                .all(|cell| cell.hyperlink.is_some())
        );
        assert!(cells.last().unwrap().hyperlink.is_none());
    }
}

#[test]
fn alacritty_damage_produces_a_sparse_frame_against_the_last_take() {
    let size = TerminalSize::new(4, 12);
    let shared_size = Arc::new(Mutex::new(size));
    let (event_proxy, _pending_replies, _notices) =
        TerminalEventProxy::new(Arc::clone(&shared_size));
    let terminal = Arc::new(Mutex::new(Term::new(Config::default(), &size, event_proxy)));
    let revision = Arc::new(AtomicU64::new(0));
    let source = TerminalViewSource {
        terminal: Arc::clone(&terminal),
        size: shared_size,
        revision: Arc::clone(&revision),
        damage_baseline: Arc::new(Mutex::new(None)),
        cursor_settle: Arc::new(Mutex::new(Default::default())),
    };

    let TerminalViewFrame::Full(mut retained) = source.take_frame().unwrap() else {
        panic!("the first damage frame must establish a full baseline");
    };
    let mut parser: Processor = Processor::new();
    parser.advance(
        &mut *terminal.lock().expect("terminal state lock poisoned"),
        b"\x1b]8;;https://example.com/a-long-shared-target\x1b\\abc\x1b]8;;\x1b\\",
    );
    revision.store(1, Ordering::Release);

    let frame = source.take_frame().unwrap();
    let TerminalViewFrame::Delta(TerminalViewDelta {
        base_revision: 0,
        revision: 1,
        runs,
        ..
    }) = &frame
    else {
        panic!("partial damage must produce a delta");
    };
    assert!(runs.iter().map(|run| run.cells.len()).sum::<usize>() < retained.cells.len());
    let links = runs
        .iter()
        .flat_map(|run| &run.cells)
        .filter_map(|cell| cell.hyperlink.as_ref())
        .collect::<Vec<_>>();
    assert!(links.len() >= 2);
    assert_eq!(
        links[0].as_ptr(),
        links[1].as_ptr(),
        "linked cells in one damage frame must share long URI storage"
    );
    retained.apply_frame(frame).unwrap();
    assert_eq!(retained, source.view());
}

fn terminal_showing(size: TerminalSize, bytes: &[u8]) -> Terminal {
    let (event_proxy, _pending_replies, _notices) =
        TerminalEventProxy::new(Arc::new(Mutex::new(size)));
    let mut terminal = Term::new(terminal_config(), &size, event_proxy);
    {
        let mut parser: Processor = Processor::new();
        parser.advance(&mut terminal, bytes);
    }
    terminal
}

fn selected_at(
    terminal: &mut Terminal,
    size: TerminalSize,
    (row, column): (u16, u16),
    unit: TerminalSelectionUnit,
) -> ((u16, u16), (u16, u16)) {
    let position = TerminalPosition {
        row,
        column,
        side: TerminalSide::Left,
    };
    let ty = match unit {
        TerminalSelectionUnit::Word => SelectionType::Semantic,
        TerminalSelectionUnit::Line => SelectionType::Lines,
    };
    let point = viewport_point(terminal, position, 0);
    terminal.selection = Some(Selection::new(ty, point, side(position.side)));
    let selection = viewport_selection(terminal, size).unwrap();
    (
        (selection.start.row, selection.start.column),
        (selection.end.row, selection.end.column),
    )
}

#[test]
fn word_and_line_selections_follow_soft_wraps_and_keep_urls_whole() {
    let size = TerminalSize::new(2, 20);
    let mut terminal = terminal_showing(size, b"run https://example.com/a?q=1, next");

    // `:` is not a separator, so the URL is one word; the trailing comma is not part of it.
    assert_eq!(
        selected_at(&mut terminal, size, (0, 12), TerminalSelectionUnit::Word),
        ((0, 4), (1, 8))
    );
    assert_eq!(
        selected_at(&mut terminal, size, (0, 1), TerminalSelectionUnit::Word),
        ((0, 0), (0, 2))
    );
    assert_eq!(
        selected_at(&mut terminal, size, (0, 5), TerminalSelectionUnit::Line),
        ((0, 0), (1, 19))
    );
}

#[test]
fn plain_text_urls_are_detected_trimmed_and_flagged() {
    let size = TerminalSize::new(1, 40);
    let terminal = terminal_showing(size, b"see https://example.com/a?b=1. now");
    let view = snapshot_terminal(&terminal, size, 1);
    let link = |column| view.cell(0, column).unwrap().hyperlink.clone();
    assert_eq!(link(4).as_deref(), Some("https://example.com/a?b=1"));
    assert_eq!(link(28).as_deref(), Some("https://example.com/a?b=1"));
    assert_eq!(link(29), None, "trailing punctuation belongs to the prose");
    assert_eq!(link(3), None);
    assert_ne!(view.cell(0, 4).unwrap().flags & DETECTED_LINK_FLAG, 0);

    let cases: [(&[u8], &str); 4] = [
        (b"(https://x.y/z)", "https://x.y/z"),
        (
            b"https://en.wikipedia.org/wiki/Foo_(bar)",
            "https://en.wikipedia.org/wiki/Foo_(bar)",
        ),
        (b"SSH://host/path", "SSH://host/path"),
        (b"mailto:user@example.com", "mailto:user@example.com"),
    ];
    for (bytes, expected) in cases {
        let terminal = terminal_showing(size, bytes);
        let view = snapshot_terminal(&terminal, size, 1);
        let linked = view
            .cells
            .iter()
            .filter_map(|cell| cell.hyperlink.as_deref())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            linked,
            [expected].into(),
            "{}",
            String::from_utf8_lossy(bytes)
        );
    }
    let terminal = terminal_showing(size, b"http:/nope");
    assert!(
        snapshot_terminal(&terminal, size, 1)
            .cells
            .iter()
            .all(|cell| cell.hyperlink.is_none())
    );
}

#[test]
fn detected_urls_continue_across_soft_wraps_only_and_yield_to_osc8() {
    let size = TerminalSize::new(3, 8);
    let terminal = terminal_showing(size, b"https://example.com/path");
    let view = snapshot_terminal(&terminal, size, 1);
    assert!(
        view.cells
            .iter()
            .all(|cell| cell.hyperlink.as_deref() == Some("https://example.com/path"))
    );

    let terminal = terminal_showing(size, b"https://\r\nexample.com");
    let view = snapshot_terminal(&terminal, size, 1);
    assert!(view.cells.iter().all(|cell| cell.hyperlink.is_none()));

    let terminal = terminal_showing(
        size,
        b"\x1b]8;;https://other\x1b\\https://\x1b]8;;\x1b\\x.y/z",
    );
    let view = snapshot_terminal(&terminal, size, 1);
    let cell = view.cell(0, 0).unwrap();
    assert_eq!(cell.hyperlink.as_deref(), Some("https://other"));
    assert_eq!(cell.flags & DETECTED_LINK_FLAG, 0);
}

#[test]
fn link_changes_reach_delta_frames_without_text_damage() {
    let size = TerminalSize::new(4, 12);
    let terminal = Arc::new(Mutex::new(terminal_showing(size, b"https://examp")));
    let revision = Arc::new(AtomicU64::new(1));
    let source = TerminalViewSource {
        terminal: Arc::clone(&terminal),
        size: Arc::new(Mutex::new(size)),
        revision: Arc::clone(&revision),
        damage_baseline: Arc::new(Mutex::new(None)),
        cursor_settle: Arc::new(Mutex::new(CursorSettle::default())),
    };
    let Some(TerminalViewFrame::Full(view)) = source.take_frame() else {
        panic!("the first frame is full");
    };
    assert_eq!(
        view.cell(0, 0).unwrap().hyperlink.as_deref(),
        Some("https://examp")
    );

    // Blank the wrapped tail: alacritty damages row 1 only, yet row 0's URL shrank.
    {
        let mut parser: Processor = Processor::new();
        parser.advance(&mut *terminal.lock().unwrap(), b"\x1b[2;1H ");
    }
    revision.fetch_add(1, Ordering::AcqRel);
    let Some(TerminalViewFrame::Delta(delta)) = source.take_frame() else {
        panic!("a small change is a delta");
    };
    let row0 = delta
        .runs
        .iter()
        .find(|run| run.start == 0)
        .expect("row 0 is resent for its link change");
    assert_eq!(row0.cells[0].hyperlink.as_deref(), Some("https://exam"));
}

#[test]
fn terminal_size_rejects_unbounded_grid_allocations() {
    assert!(TerminalSize::new(256, 256).validate().is_ok());
    assert!(TerminalSize::new(257, 256).validate().is_err());
}

/// The burst benchmark AGENTS.md asks for, VT side: an agent CLI redrawing a three-line status
/// block hundreds of times per second, with the monitor taking one frame per display interval.
/// Every frame must be a sparse delta, the retained view must equal the VT afterwards, and
/// the last frame must carry the last revision. Run with `--nocapture` for the timing.
#[test]
fn a_status_line_burst_coalesces_into_sparse_frames_and_keeps_the_last_revision() {
    const CHUNKS: u64 = 5_000;
    /// PTY reads per 60 Hz frame; the monitor coalesces them into one `take_frame`.
    const CHUNKS_PER_FRAME: u64 = 40;
    const REDRAWN_ROWS: usize = 3;

    let size = TerminalSize::new(24, 80);
    let screen = (0..21)
        .map(|row| format!("\x1b[{};1H{row:02}{}", row + 1, "x".repeat(78)))
        .collect::<String>();
    let terminal = Arc::new(Mutex::new(terminal_showing(size, screen.as_bytes())));
    let revision = Arc::new(AtomicU64::new(1));
    let source = TerminalViewSource {
        terminal: Arc::clone(&terminal),
        size: Arc::new(Mutex::new(size)),
        revision: Arc::clone(&revision),
        damage_baseline: Arc::new(Mutex::new(None)),
        cursor_settle: Arc::new(Mutex::new(CursorSettle::default())),
    };
    let Some(TerminalViewFrame::Full(mut retained)) = source.take_frame() else {
        panic!("the first frame is full");
    };

    let spinner = ['|', '/', '-', '\\'];
    let mut parser: Processor = Processor::new();
    let (mut frames, mut delta_cells) = (0u64, 0usize);
    let started = std::time::Instant::now();
    for chunk in 1..=CHUNKS {
        let tick = format!(
            "\x1b[22;1H\x1b[2K{} working on step {chunk}\x1b[23;1H\x1b[2Kelapsed {}s\x1b[24;1H\x1b[2K> ",
            spinner[usize::try_from(chunk % 4).unwrap()],
            chunk / 10
        );
        parser.advance(&mut *terminal.lock().unwrap(), tick.as_bytes());
        revision.fetch_add(1, Ordering::AcqRel);
        if chunk % CHUNKS_PER_FRAME != 0 && chunk != CHUNKS {
            continue;
        }
        let frame = source.take_frame().expect("new output produces a frame");
        match &frame {
            TerminalViewFrame::Delta(delta) => {
                delta_cells += delta.runs.iter().map(|run| run.cells.len()).sum::<usize>();
            }
            TerminalViewFrame::Full(_) => {
                panic!("a status-line redraw must not invalidate the whole screen")
            }
        }
        frames += 1;
        retained.apply_frame(frame).unwrap();
    }
    let elapsed = started.elapsed();

    assert_eq!(frames, CHUNKS / CHUNKS_PER_FRAME);
    assert_eq!(
        retained.revision,
        revision.load(Ordering::Acquire),
        "the final frame carries the last revision"
    );
    // Plus one row: the first tick also damages the cell the cursor left on row 21.
    let columns = usize::from(size.columns);
    assert!(
        delta_cells <= usize::try_from(frames).unwrap() * REDRAWN_ROWS * columns + columns,
        "each frame carries at most the redrawn rows, got {delta_cells} cells over {frames} frames"
    );
    // The settled cursor may still be held back; everything else must match the VT.
    let mut current = source.view();
    retained.cursor = None;
    current.cursor = None;
    assert_eq!(
        retained, current,
        "the retained view matches the VT after the burst"
    );
    eprintln!(
        "burst: {CHUNKS} chunks -> {frames} frames, {delta_cells} delta cells, {elapsed:.2?}"
    );
}
