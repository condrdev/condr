use condr_core::*;
use smol_str::SmolStr;

fn blank_cell() -> TerminalCell {
    TerminalCell {
        text: " ".into(),
        foreground: TerminalColor::Named(256),
        background: TerminalColor::Named(257),
        flags: 0,
        hyperlink: None,
    }
}

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
fn sparse_terminal_frame_round_trips_from_the_committed_baseline() {
    let mut previous = frame_test_view(7, "abcdef");
    let mut current = frame_test_view(8, "abXdef");
    current.cursor = Some(TerminalCursor {
        row: 0,
        column: 3,
        shape: TerminalCursorShape::Beam,
        blinking: false,
    });

    let frame = TerminalView::frame_from(Some(&previous), &current).unwrap();
    assert!(matches!(
        &frame,
        TerminalViewFrame::Delta(TerminalViewDelta { runs, .. })
            if runs.len() == 1 && runs[0].start == 2 && runs[0].cells.len() == 1
    ));
    previous.apply_frame(frame).unwrap();
    assert_eq!(previous, current);
}

#[test]
fn terminal_frame_wire_interns_hyperlinks_and_round_trips() {
    let uri = "https://example.com/a-long-target-that-must-not-repeat";
    let mut view = frame_test_view(7, "abcdef");
    for cell in &mut view.cells[1..4] {
        cell.hyperlink = Some(uri.into());
    }
    let delta = TerminalViewDelta {
        selection: None,
        base_revision: 7,
        revision: 8,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: vec![TerminalCellRun {
            start: 1,
            cells: view.cells[1..4].to_vec(),
        }],
    };

    for frame in [
        TerminalViewFrame::Full(view),
        TerminalViewFrame::Delta(delta),
    ] {
        let encoded = bincode::serialize(&frame).unwrap();
        assert_eq!(
            encoded
                .windows(uri.len())
                .filter(|bytes| *bytes == uri.as_bytes())
                .count(),
            1
        );
        assert_eq!(
            bincode::deserialize::<TerminalViewFrame>(&encoded).unwrap(),
            frame
        );
    }
}

#[test]
fn link_free_deltas_take_the_budget_fast_path_without_changing_results() {
    let mut budgeted = frame_test_view(1, "abc");
    let mut plain = budgeted.clone();
    let mut hyperlinks = TerminalHyperlinkBudget::new(&mut budgeted);
    let delta = TerminalViewDelta {
        selection: None,
        base_revision: 1,
        revision: 2,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: vec![TerminalCellRun {
            start: 1,
            cells: vec![frame_test_view(2, "x").cells.remove(0)],
        }],
    };

    let returned = hyperlinks
        .apply_delta(&mut budgeted, delta.clone())
        .unwrap();
    plain
        .apply_frame(TerminalViewFrame::Delta(delta.clone()))
        .unwrap();
    assert_eq!(returned, delta);
    assert_eq!(budgeted, plain);

    // A stale delta is still rejected before anything is applied.
    assert!(hyperlinks.apply_delta(&mut budgeted, delta).is_err());
    assert_eq!(budgeted, plain);
}

#[test]
fn retained_hyperlinks_share_storage_across_sparse_deltas() {
    let uri = "https://example.com/a-long-target-shared-across-deltas";
    let mut view = frame_test_view(1, "ab");
    view.cells[0].hyperlink = Some(SmolStr::new(uri));
    let mut hyperlinks = TerminalHyperlinkBudget::new(&mut view);
    let incoming = SmolStr::new(uri);
    assert_ne!(
        view.cells[0].hyperlink.as_ref().unwrap().as_ptr(),
        incoming.as_ptr()
    );

    let mut cell = frame_test_view(2, "b").cells.remove(0);
    cell.hyperlink = Some(incoming);
    hyperlinks
        .apply_delta(
            &mut view,
            TerminalViewDelta {
                selection: None,
                base_revision: 1,
                revision: 2,
                display_offset: 0,
                mouse_tracking: TerminalMouseTracking::None,
                cursor: None,
                runs: vec![TerminalCellRun {
                    start: 1,
                    cells: vec![cell],
                }],
            },
        )
        .unwrap();

    assert_eq!(
        view.cells[0].hyperlink.as_ref().unwrap().as_ptr(),
        view.cells[1].hyperlink.as_ref().unwrap().as_ptr(),
        "retained cells must share one long URI allocation across deltas"
    );
}

#[test]
fn dense_or_resized_terminal_frames_use_a_full_view() {
    let previous = frame_test_view(1, "abcdef");
    let dense = frame_test_view(2, "XYZWef");
    assert!(matches!(
        TerminalView::frame_from(Some(&previous), &dense),
        Some(TerminalViewFrame::Full(_))
    ));

    let resized = frame_test_view(3, "abcdefg");
    assert!(matches!(
        TerminalView::frame_from(Some(&previous), &resized),
        Some(TerminalViewFrame::Full(_))
    ));
}

#[test]
fn visually_identical_terminal_revision_does_not_create_a_frame() {
    let previous = frame_test_view(1, "same");
    let current = frame_test_view(2, "same");
    assert_eq!(TerminalView::frame_from(Some(&previous), &current), None);
}

#[test]
fn terminal_delta_rejects_revision_gaps_and_invalid_runs() {
    let mut view = frame_test_view(4, "abcd");
    let gap = TerminalViewFrame::Delta(TerminalViewDelta {
        selection: None,
        base_revision: 5,
        revision: 6,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: Vec::new(),
    });
    assert!(matches!(
        view.apply_frame(gap),
        Err(TerminalFrameError::RevisionMismatch {
            expected: 5,
            actual: 4
        })
    ));

    let invalid = TerminalViewFrame::Delta(TerminalViewDelta {
        selection: None,
        base_revision: 4,
        revision: 5,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: vec![TerminalCellRun {
            start: 4,
            cells: vec![blank_cell()],
        }],
    });
    assert_eq!(
        view.apply_frame(invalid),
        Err(TerminalFrameError::InvalidCellRun)
    );
}

#[test]
fn terminal_selection_matches_simple_cell_boundary_semantics() {
    let position = |row, column, side| TerminalPosition { row, column, side };
    let selection = |start, end| TerminalSelection {
        start,
        end,
        display_offset: 0,
    };

    let empty = selection(
        position(0, 1, TerminalSide::Left),
        position(0, 1, TerminalSide::Left),
    );
    assert!(!empty.contains_cell(0, 1, 10));

    let one_cell = selection(
        position(0, 1, TerminalSide::Left),
        position(0, 1, TerminalSide::Right),
    );
    assert!(one_cell.contains_cell(0, 1, 10));

    let middle_cell = selection(
        position(0, 1, TerminalSide::Right),
        position(0, 3, TerminalSide::Left),
    );
    assert!(!middle_cell.contains_cell(0, 1, 10));
    assert!(middle_cell.contains_cell(0, 2, 10));
    assert!(!middle_cell.contains_cell(0, 3, 10));

    let reversed = selection(
        position(1, 1, TerminalSide::Right),
        position(0, 8, TerminalSide::Left),
    );
    assert!(reversed.contains_cell(0, 8, 10));
    assert!(reversed.contains_cell(1, 1, 10));
}
