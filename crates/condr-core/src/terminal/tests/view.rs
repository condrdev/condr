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
    let mut vt = Vt::new(size).unwrap();
    let cursor = |vt: &mut Vt| vt.view(size, 1, None).cursor.unwrap();
    assert_eq!(cursor(&mut vt).shape, TerminalCursorShape::Block);
    assert!(!cursor(&mut vt).blinking, "default cursor stays steady");

    vt.write(b"\x1b[5 q");
    assert_eq!(cursor(&mut vt).shape, TerminalCursorShape::Beam);
    assert!(cursor(&mut vt).blinking);

    vt.write(b"\x1b[4 q");
    assert_eq!(cursor(&mut vt).shape, TerminalCursorShape::Underline);
    assert!(!cursor(&mut vt).blinking);

    vt.write(b"\x1b[?25l");
    assert_eq!(cursor(&mut vt).shape, TerminalCursorShape::Hidden);
}
#[test]
fn osc8_hyperlinks_are_carried_per_cell() {
    let size = TerminalSize::new(1, 4);
    let mut vt = terminal_showing(
        size,
        b"\x1b]8;;https://example.com/a-long-shared-target\x1b\\ab\x1b]8;;\x1b\\c",
    );
    let view = vt.view(size, 1, None);
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
    let mut hyperlinks = super::vt::Hyperlinks::default();
    hyperlinks.bytes = MAX_TERMINAL_HYPERLINK_BYTES - 3;
    assert_eq!(hyperlinks.intern("abc").as_deref(), Some("abc"));
    assert_eq!(
        hyperlinks.intern("abc").as_deref(),
        Some("abc"),
        "a URI already kept costs nothing"
    );
    assert_eq!(hyperlinks.intern("d"), None);
    assert_eq!(
        hyperlinks.intern(&"x".repeat(MAX_TERMINAL_HYPERLINK_URI_BYTES + 1)),
        None
    );
}
#[test]
fn server_selection_follows_scrolled_output_and_still_copies_it() {
    let size = TerminalSize::new(3, 10);
    let mut vt = terminal_showing(size, b"alpha\r\nbeta\r\ngamma");
    let position = |row, column, side| TerminalPosition { row, column, side };
    let selection = TerminalSelection {
        start: position(1, 0, TerminalSide::Left),
        end: position(1, 3, TerminalSide::Right),
        display_offset: 0,
    };

    vt.select(Some(selection));
    assert_eq!(vt.view(size, 1, None).selection, Some(selection));
    assert_eq!(vt.selection_text(None).as_deref(), Some("beta"));

    vt.write(b"\r\ndelta");
    let moved = vt.view(size, 1, None).selection.unwrap();
    assert_eq!((moved.start.row, moved.end.row), (0, 0));
    assert_eq!(vt.selection_text(None).as_deref(), Some("beta"));

    vt.write(b"\r\nepsilon");
    // Scrolled out of view: still reported (so a copy is offered) but matches no cell.
    let hidden = vt.view(size, 1, None).selection.unwrap();
    assert_eq!((hidden.start.row, hidden.end.row), (size.rows, size.rows));
    assert!(hidden.selected_cell_range(size.columns).is_some());
    assert!((0..size.rows).all(|row| !hidden.contains_cell(row, 0, size.columns)));
    assert_eq!(vt.selection_text(None).as_deref(), Some("beta"));

    vt.select(None);
    assert_eq!(vt.view(size, 1, None).selection, None);
}
/// ADR 0051: a resize, which switching windows does (ADR 0036), keeps the selection, and
/// an erase that touches it keeps it too; a screen swap still clears it (ADR 0008).
#[test]
fn a_selection_survives_a_resize_and_an_erase_but_not_a_screen_swap() {
    let size = TerminalSize::new(3, 10);
    let mut vt = terminal_showing(size, b"alpha\r\nbeta\r\ngamma");
    let position = |row, column, side| TerminalPosition { row, column, side };
    vt.select(Some(TerminalSelection {
        start: position(1, 0, TerminalSide::Left),
        end: position(1, 3, TerminalSide::Right),
        display_offset: 0,
    }));

    let wider = TerminalSize::new(5, 30);
    vt.resize(wider);
    assert_eq!(vt.selection_text(None).as_deref(), Some("beta"));
    vt.resize(TerminalSize::new(3, 6));
    assert_eq!(vt.selection_text(None).as_deref(), Some("beta"));

    vt.write(b"\x1b[2;3H\x1b[K");
    assert!(vt.selection_text(None).is_some(), "an erase keeps it");

    vt.write(b"\x1b[?1049h");
    assert_eq!(
        vt.selection_text(None),
        None,
        "the alternate screen drops it"
    );
}

/// ADR 0051: frames wait while a program draws a synchronized update (mode 2026), and a
/// program that dies inside one does not freeze its Pane past the hold timeout.
#[test]
fn a_synchronized_update_holds_frames_until_it_ends_or_times_out() {
    let size = TerminalSize::new(4, 20);
    let terminal = Arc::new(Mutex::new(terminal_showing(size, b"before")));
    let revision = Arc::new(AtomicU64::new(1));
    let source = source_for(&terminal, size, &revision);
    let write = |bytes: &[u8]| {
        terminal.lock().unwrap().write(bytes);
        revision.fetch_add(1, Ordering::AcqRel);
    };
    assert!(source.take_frame().is_some());

    write(b"\x1b[?2026h\r\nhalf");
    assert!(
        source.take_frame().is_none(),
        "held while the program draws"
    );
    assert!(
        source
            .frame_wakeup()
            .is_some_and(|wait| wait <= super::vt::RENDER_HOLD_TIMEOUT),
        "the monitor wakes when the hold would time out"
    );
    write(b" done\x1b[?2026l");
    assert!(source.take_frame().is_some(), "the update ended");

    write(b"\x1b[?2026h\r\nstuck");
    assert!(source.take_frame().is_none());
    terminal.lock().unwrap().held_since =
        Instant::now().checked_sub(super::vt::RENDER_HOLD_TIMEOUT);
    assert!(source.take_frame().is_some(), "the hold timed out");
    assert_eq!(source.frame_wakeup(), None);
}

/// ADR 0051: once output stops, the monitor's idle checks compress the whole scrollback,
/// step by step, and the history reads back the same.
#[test]
fn idle_scrollback_is_compressed_to_the_end_and_reads_back_unchanged() {
    let size = TerminalSize::new(24, 100);
    let terminal = Arc::new(Mutex::new(Vt::new(size).unwrap()));
    let revision = Arc::new(AtomicU64::new(1));
    let source = source_for(&terminal, size, &revision);
    let line = "fn main() { println!(\"a line of ordinary terminal output\"); }\r\n";
    terminal
        .lock()
        .unwrap()
        .write(line.repeat(40_000).as_bytes());
    let before = terminal.lock().unwrap().resident_bytes();
    let tail = terminal.lock().unwrap().recent_text(3);

    source.compress_if_idle();
    assert_eq!(
        terminal.lock().unwrap().resident_bytes(),
        before,
        "the first look only notes activity"
    );
    source.compress_if_idle();
    let after = terminal.lock().unwrap().resident_bytes();
    if cfg!(any(target_os = "macos", target_os = "linux")) {
        assert!(
            after * 2 < before,
            "compressed to the end: {after} of {before} bytes"
        );
    }
    assert_eq!(terminal.lock().unwrap().recent_text(3), tail);
}

#[test]
fn terminal_cell_text_keeps_short_content_unchanged() {
    let size = TerminalSize::new(1, 4);
    let mut vt = terminal_showing(size, "e\u{301}\u{327}".as_bytes());
    assert_eq!(
        vt.view(size, 1, None).cell(0, 0).unwrap().text,
        "e\u{301}\u{327}"
    );
}
#[test]
fn terminal_cell_text_truncates_long_combining_sequences() {
    let text = super::vt::cell_text(&format!("a{}", "\u{301}".repeat(200)));
    assert_eq!(text.len(), 255);
    assert_eq!(text.chars().count(), 128);
    assert_eq!(text.chars().next(), Some('a'));
    assert!(text.chars().skip(1).all(|character| character == '\u{301}'));
}
#[test]
fn terminal_cell_text_truncation_preserves_utf8_boundaries_and_base() {
    let text = super::vt::cell_text(&format!("\u{754c}{}", "\u{1d165}".repeat(100)));
    assert_eq!(text.len(), 255);
    assert_eq!(text.chars().count(), 64);
    assert_eq!(text.chars().next(), Some('\u{754c}'));
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
        let pane = crate::protocol::PaneTerminalFrame {
            pane_id: crate::PaneId::from_u64(1),
            frame,
        };
        let encoded = crate::protocol::encode_pane_terminal_frame(&pane).unwrap();
        assert!(encoded.len() <= crate::protocol::MAX_CHUNKED_RECORD_SIZE);
        let decoded = crate::protocol::decode_pane_terminal_frame(&encoded)
            .unwrap()
            .unwrap()
            .frame;
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
fn changed_rows_produce_a_sparse_frame_against_the_last_take() {
    let size = TerminalSize::new(4, 12);
    let terminal = Arc::new(Mutex::new(Vt::new(size).unwrap()));
    let revision = Arc::new(AtomicU64::new(0));
    let source = source_for(&terminal, size, &revision);

    let TerminalViewFrame::Full(mut retained) = source.take_frame().unwrap() else {
        panic!("the first frame must establish a full baseline");
    };
    terminal
        .lock()
        .unwrap()
        .write(b"\x1b]8;;https://example.com/a-long-shared-target\x1b\\abc\x1b]8;;\x1b\\");
    revision.store(1, Ordering::Release);

    let frame = source.take_frame().unwrap();
    let TerminalViewFrame::Delta(TerminalViewDelta {
        base_revision: 0,
        revision: 1,
        runs,
        ..
    }) = &frame
    else {
        panic!("a small change must produce a delta");
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
        "linked cells in one frame must share long URI storage"
    );
    retained.apply_frame(frame).unwrap();
    assert_eq!(retained, source.view());
}

fn source_for(
    terminal: &Arc<Mutex<Terminal>>,
    size: TerminalSize,
    revision: &Arc<AtomicU64>,
) -> TerminalViewSource {
    TerminalViewSource {
        terminal: Arc::clone(terminal),
        size: Arc::new(Mutex::new(size)),
        revision: Arc::clone(revision),
        damage_baseline: Arc::new(Mutex::new(None)),
        cursor_settle: Arc::new(Mutex::new(CursorSettle::default())),
    }
}
fn terminal_showing(size: TerminalSize, bytes: &[u8]) -> Terminal {
    let mut vt = Vt::new(size).unwrap();
    vt.write(bytes);
    vt
}
fn selected_at(
    vt: &mut Terminal,
    size: TerminalSize,
    (row, column): (u16, u16),
    unit: TerminalSelectionUnit,
) -> ((u16, u16), (u16, u16)) {
    let position = TerminalPosition {
        row,
        column,
        side: TerminalSide::Left,
    };
    vt.select_at(position, 0, unit);
    let selection = vt.view(size, 1, None).selection.unwrap();
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
        ((0, 0), (1, 14)),
        "a line ends at its last character, as Ghostty selects one"
    );
}

#[test]
fn plain_text_urls_are_detected_trimmed_and_flagged() {
    let size = TerminalSize::new(1, 40);
    let mut terminal = terminal_showing(size, b"see https://example.com/a?b=1. now");
    let view = terminal.view(size, 1, None);
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
        let mut terminal = terminal_showing(size, bytes);
        let view = terminal.view(size, 1, None);
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
    let mut terminal = terminal_showing(size, b"http:/nope");
    assert!(
        terminal
            .view(size, 1, None)
            .cells
            .iter()
            .all(|cell| cell.hyperlink.is_none())
    );
}

#[test]
fn detected_urls_continue_across_soft_wraps_only_and_yield_to_osc8() {
    let size = TerminalSize::new(3, 8);
    let mut terminal = terminal_showing(size, b"https://example.com/path");
    let view = terminal.view(size, 1, None);
    assert!(
        view.cells
            .iter()
            .all(|cell| cell.hyperlink.as_deref() == Some("https://example.com/path"))
    );

    let mut terminal = terminal_showing(size, b"https://\r\nexample.com");
    let view = terminal.view(size, 1, None);
    assert!(view.cells.iter().all(|cell| cell.hyperlink.is_none()));

    let mut terminal = terminal_showing(
        size,
        b"\x1b]8;;https://other\x1b\\https://\x1b]8;;\x1b\\x.y/z",
    );
    let view = terminal.view(size, 1, None);
    let cell = view.cell(0, 0).unwrap();
    assert_eq!(cell.hyperlink.as_deref(), Some("https://other"));
    assert_eq!(cell.flags & DETECTED_LINK_FLAG, 0);
}

#[test]
fn link_changes_reach_delta_frames_without_text_damage() {
    let size = TerminalSize::new(4, 12);
    let terminal = Arc::new(Mutex::new(terminal_showing(size, b"https://examp")));
    let revision = Arc::new(AtomicU64::new(1));
    let source = source_for(&terminal, size, &revision);
    let Some(TerminalViewFrame::Full(view)) = source.take_frame() else {
        panic!("the first frame is full");
    };
    assert_eq!(
        view.cell(0, 0).unwrap().hyperlink.as_deref(),
        Some("https://examp")
    );

    // Blank the wrapped tail: only row 1's text changes, yet row 0's URL shrank.
    terminal.lock().unwrap().write(b"\x1b[2;1H ");
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
    // 21 rows of distinct text, then the cursor parked where the status block will draw,
    // so the cell it leaves on the first tick is inside the redrawn rows.
    let screen = (0..21)
        .map(|row| format!("\x1b[{};1H{row:02}{}", row + 1, "x".repeat(78)))
        .collect::<String>()
        + "\x1b[22;1H";
    let terminal = Arc::new(Mutex::new(terminal_showing(size, screen.as_bytes())));
    let revision = Arc::new(AtomicU64::new(1));
    let source = source_for(&terminal, size, &revision);
    let Some(TerminalViewFrame::Full(mut retained)) = source.take_frame() else {
        panic!("the first frame is full");
    };

    let spinner = ['|', '/', '-', '\\'];
    let (mut frames, mut delta_cells) = (0u64, 0usize);
    let started = std::time::Instant::now();
    for chunk in 1..=CHUNKS {
        let tick = format!(
            "\x1b[22;1H\x1b[2K{} working on step {chunk}\x1b[23;1H\x1b[2Kelapsed {}s\x1b[24;1H\x1b[2K> ",
            spinner[usize::try_from(chunk % 4).unwrap()],
            chunk / 10
        );
        terminal.lock().unwrap().write(tick.as_bytes());
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
    let columns = usize::from(size.columns);
    assert!(
        delta_cells <= usize::try_from(frames).unwrap() * REDRAWN_ROWS * columns,
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

#[test]
fn programs_negotiate_the_kitty_keyboard_protocol() {
    let mut vt = Vt::new(TerminalSize::new(2, 8)).unwrap();
    let escape = |vt: &mut Vt| {
        vt.encode_key(
            &TerminalKey::Escape,
            TerminalModifiers::default(),
            TerminalKeyEventKind::Press,
            false,
        )
        .unwrap()
    };
    vt.write(b"\x1b[>1u");
    assert_eq!(escape(&mut vt), b"\x1b[27u");
    vt.write(b"\x1b[<u");
    assert_eq!(escape(&mut vt), b"\x1b");
}
