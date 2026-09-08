use super::*;

#[test]
fn terminal_frame_chunks_survive_metadata_interleaving_until_complete() {
    let pane_id = pane_id();
    let expected = PaneTerminalFrame {
        pane_id,
        frame: TerminalViewFrame::Full(terminal_view(7, "chunked")),
    };
    let payload = encode_pane_terminal_frame(&expected).unwrap();
    let midpoint = payload.len() / 2;
    let chunks = [&payload[..midpoint], &payload[midpoint..]];
    for interleaved in [
        ServerMessage::TerminalClipboard {
            pane_id,
            text: "copied between chunks".into(),
        },
        ServerMessage::Event {
            server_id: ServerId(1),
            session_id: SessionId(1),
            sequence: 1,
            event: SessionEvent::TerminalTitleChanged {
                pane_id,
                title: Some("title between chunks".into()),
            },
        },
    ] {
        let mut assembly = None;
        for (chunk_index, payload) in chunks.iter().enumerate() {
            let assembled = assemble_terminal_frame_chunk(
                &mut assembly,
                TerminalFrameChunk {
                    server_id: ServerId(1),
                    session_id: SessionId(1),
                    pane_id,
                    revision: 7,
                    chunk_index: chunk_index as u32,
                    chunk_count: 2,
                    payload: payload.to_vec(),
                },
            )
            .unwrap();
            if chunk_index == 0 {
                assert!(assembled.is_none());
                assert!(enforce_terminal_chunk_reliable_fence(&mut assembly, &interleaved).is_ok());
                assert!(assembly.is_some());
            } else {
                assert_eq!(assembled, Some(expected.clone()));
            }
        }
        assert!(assembly.is_none());
    }
}

#[test]
fn terminal_chunk_identity_mismatch_aborts_the_in_progress_record() {
    let pane_id = pane_id();
    let mut assembly = None;
    assemble_terminal_frame_chunk(
        &mut assembly,
        TerminalFrameChunk {
            server_id: ServerId(1),
            session_id: SessionId(2),
            pane_id,
            revision: 7,
            chunk_index: 0,
            chunk_count: 2,
            payload: vec![1],
        },
    )
    .unwrap();

    let mismatched = TerminalFrameChunk {
        server_id: ServerId(9),
        session_id: SessionId(2),
        pane_id,
        revision: 7,
        chunk_index: 1,
        chunk_count: 2,
        payload: vec![2],
    };
    assert!(
        terminal_chunk_identity_matches(&mut assembly, &mismatched, ServerId(1), SessionId(2))
            .is_err()
    );
    assert!(assembly.is_none());
}

#[test]
fn reliable_message_is_a_terminal_chunk_protocol_fence() {
    let pane_id = pane_id();
    let mut assembly = None;
    assemble_terminal_frame_chunk(
        &mut assembly,
        TerminalFrameChunk {
            server_id: ServerId(1),
            session_id: SessionId(2),
            pane_id,
            revision: 7,
            chunk_index: 0,
            chunk_count: 2,
            payload: vec![1],
        },
    )
    .unwrap();

    let reliable = ServerMessage::ControlGranted {
        server_id: ServerId(1),
        session_id: SessionId(2),
    };
    assert!(enforce_terminal_chunk_reliable_fence(&mut assembly, &reliable).is_err());
    assert!(assembly.is_none());
}

#[test]
fn terminal_frame_batch_is_atomic_when_a_later_pane_has_a_gap() {
    let first_pane = pane_id();
    let second_pane = pane_id();
    let first_view = terminal_view(1, "a");
    let second_view = terminal_view(1, "b");
    let mut terminals = std::collections::HashMap::from([
        (
            first_pane,
            ClientTerminal::from(PaneTerminalSnapshot {
                pane_id: first_pane,
                view: first_view.clone(),
                exited: false,
                title: None,
                attention: false,
            }),
        ),
        (
            second_pane,
            ClientTerminal::from(PaneTerminalSnapshot {
                pane_id: second_pane,
                view: second_view.clone(),
                exited: false,
                title: None,
                attention: false,
            }),
        ),
    ]);
    let batch = vec![
        PaneTerminalFrame {
            pane_id: first_pane,
            frame: TerminalViewFrame::Delta(TerminalViewDelta {
                selection: None,
                base_revision: 1,
                revision: 2,
                display_offset: 0,
                mouse_tracking: TerminalMouseTracking::None,
                cursor: None,
                runs: vec![TerminalCellRun {
                    start: 0,
                    cells: vec![terminal_cell("x")],
                }],
            }),
        },
        PaneTerminalFrame {
            pane_id: second_pane,
            frame: TerminalViewFrame::Delta(TerminalViewDelta {
                selection: None,
                base_revision: 99,
                revision: 100,
                display_offset: 0,
                mouse_tracking: TerminalMouseTracking::None,
                cursor: None,
                runs: vec![TerminalCellRun {
                    start: 0,
                    cells: vec![terminal_cell("y")],
                }],
            }),
        },
    ];
    let mut terminal_hyperlinks = terminal_hyperlink_budgets(&mut terminals);

    assert!(apply_terminal_frame_batch(&mut terminals, &mut terminal_hyperlinks, batch).is_err());
    assert_eq!(*terminals[&first_pane].view, first_view);
    assert_eq!(*terminals[&second_pane].view, second_view);
}

#[test]
fn terminal_frame_batch_bounds_hyperlinks_across_retained_deltas() {
    let pane_id = pane_id();
    let mut terminals = std::collections::HashMap::from([(
        pane_id,
        ClientTerminal::from(PaneTerminalSnapshot {
            pane_id,
            view: terminal_view(1, "x"),
            exited: false,
            title: None,
            attention: false,
        }),
    )]);
    let mut terminal_hyperlinks = terminal_hyperlink_budgets(&mut terminals);
    let mut linked = terminal_cell("x");
    linked.hyperlink = Some("x".repeat(8 * 1024 + 1).into());

    apply_terminal_frame_batch(
        &mut terminals,
        &mut terminal_hyperlinks,
        vec![PaneTerminalFrame {
            pane_id,
            frame: TerminalViewFrame::Delta(TerminalViewDelta {
                selection: None,
                base_revision: 1,
                revision: 2,
                display_offset: 0,
                mouse_tracking: TerminalMouseTracking::None,
                cursor: None,
                runs: vec![TerminalCellRun {
                    start: 0,
                    cells: vec![linked],
                }],
            }),
        }],
    )
    .unwrap();

    assert!(terminals[&pane_id].view.cells[0].hyperlink.is_none());
}

#[test]
fn terminal_frame_batch_canonicalizes_links_across_retained_deltas() {
    let pane_id = pane_id();
    let uri = "https://example.com/a-long-target-shared-across-deltas";
    let mut view = terminal_view(1, "ab");
    view.cells[0].hyperlink = Some(uri.into());
    let mut terminals = std::collections::HashMap::from([(
        pane_id,
        ClientTerminal::from(PaneTerminalSnapshot {
            pane_id,
            view,
            exited: false,
            title: None,
            attention: false,
        }),
    )]);
    let mut terminal_hyperlinks = terminal_hyperlink_budgets(&mut terminals);
    let mut linked = terminal_cell("b");
    linked.hyperlink = Some(uri.into());

    apply_terminal_frame_batch(
        &mut terminals,
        &mut terminal_hyperlinks,
        vec![PaneTerminalFrame {
            pane_id,
            frame: TerminalViewFrame::Delta(TerminalViewDelta {
                selection: None,
                base_revision: 1,
                revision: 2,
                display_offset: 0,
                mouse_tracking: TerminalMouseTracking::None,
                cursor: None,
                runs: vec![TerminalCellRun {
                    start: 1,
                    cells: vec![linked],
                }],
            }),
        }],
    )
    .unwrap();

    let view = &terminals[&pane_id].view;
    assert_eq!(
        view.cells[0].hyperlink.as_ref().unwrap().as_ptr(),
        view.cells[1].hyperlink.as_ref().unwrap().as_ptr()
    );
}

#[test]
fn gui_visual_slot_composes_pending_deltas_into_one_signal() {
    let pane_id = pane_id();
    let slot = TerminalVisualSlot::default();
    let batch = |frame| TerminalFrameBatch {
        server_id: ServerId(1),
        session_id: SessionId(2),
        panes: vec![PaneTerminalFrame { pane_id, frame }],
    };
    let uri = "https://example.com/a-long-target-shared-across-pending-deltas";
    let mut first_cell = terminal_cell("X");
    first_cell.hyperlink = Some(uri.into());
    let mut second_cell = terminal_cell("Y");
    second_cell.hyperlink = Some(uri.into());
    let first = TerminalViewDelta {
        selection: None,
        base_revision: 1,
        revision: 2,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: vec![TerminalCellRun {
            start: 1,
            cells: vec![first_cell],
        }],
    };
    let second = TerminalViewDelta {
        selection: None,
        base_revision: 2,
        revision: 3,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: vec![TerminalCellRun {
            start: 2,
            cells: vec![second_cell],
        }],
    };

    assert_eq!(
        slot.publish(batch(TerminalViewFrame::Delta(first)))
            .unwrap(),
        Some(0)
    );
    assert_eq!(
        slot.publish(batch(TerminalViewFrame::Delta(second)))
            .unwrap(),
        None
    );
    let mut view = terminal_view(1, "abcd");
    let pending = slot.take(0).unwrap();
    assert_eq!(pending.panes.len(), 1);
    view.apply_frame(pending.panes.into_iter().next().unwrap().frame)
        .unwrap();
    assert_eq!(view.revision, 3);
    assert_eq!(view.cells[1].text.as_str(), "X");
    assert_eq!(view.cells[2].text.as_str(), "Y");
    assert_eq!(
        view.cells[1].hyperlink.as_ref().unwrap().as_ptr(),
        view.cells[2].hyperlink.as_ref().unwrap().as_ptr()
    );
}

#[test]
fn visual_slot_publish_is_atomic_across_panes() {
    let first_pane = pane_id();
    let second_pane = pane_id();
    let slot = TerminalVisualSlot::default();
    let batch = |panes| TerminalFrameBatch {
        server_id: ServerId(1),
        session_id: SessionId(2),
        panes,
    };
    let delta = |base_revision, text| {
        TerminalViewFrame::Delta(TerminalViewDelta {
            selection: None,
            base_revision,
            revision: base_revision + 1,
            display_offset: 0,
            mouse_tracking: TerminalMouseTracking::None,
            cursor: None,
            runs: vec![TerminalCellRun {
                start: 0,
                cells: vec![terminal_cell(text)],
            }],
        })
    };

    assert_eq!(
        slot.publish(batch(vec![
            PaneTerminalFrame {
                pane_id: first_pane,
                frame: TerminalViewFrame::Full(terminal_view(1, "a")),
            },
            PaneTerminalFrame {
                pane_id: second_pane,
                frame: TerminalViewFrame::Full(terminal_view(1, "b")),
            },
        ]))
        .unwrap(),
        Some(0)
    );

    // The first pane merges cleanly; the second pane's delta does not match its base.
    assert!(
        slot.publish(batch(vec![
            PaneTerminalFrame {
                pane_id: first_pane,
                frame: delta(1, "x"),
            },
            PaneTerminalFrame {
                pane_id: second_pane,
                frame: delta(5, "y"),
            },
        ]))
        .is_err()
    );
    {
        let state = slot.state.lock().unwrap();
        assert_eq!(state.generation, 0);
        assert!(state.signaled);
        let pending = state.pending.as_ref().unwrap();
        assert_eq!(pending.panes.len(), 2);
        assert!(matches!(
            &pending.panes[&first_pane],
            TerminalViewFrame::Full(view) if view.revision == 1 && view.cells[0].text == "a"
        ));
        assert!(matches!(
            &pending.panes[&second_pane],
            TerminalViewFrame::Full(view) if view.revision == 1 && view.cells[0].text == "b"
        ));
    }
    let taken = slot.take(0).unwrap();
    assert_eq!(taken.panes.len(), 2);
    assert!(taken.panes.iter().all(|pane| matches!(
        &pane.frame,
        TerminalViewFrame::Full(view) if view.revision == 1
    )));
}

#[test]
fn bootstrap_generation_discards_an_old_visual_signal() {
    let pane_id = pane_id();
    let slot = TerminalVisualSlot::default();
    let batch = |revision, text| TerminalFrameBatch {
        server_id: ServerId(1),
        session_id: SessionId(2),
        panes: vec![PaneTerminalFrame {
            pane_id,
            frame: TerminalViewFrame::Full(terminal_view(revision, text)),
        }],
    };

    assert_eq!(slot.publish(batch(1, "old")).unwrap(), Some(0));
    slot.advance();
    assert_eq!(slot.publish(batch(2, "new")).unwrap(), Some(1));
    assert!(slot.take(0).is_none());
    let current = slot.take(1).unwrap();
    assert!(matches!(
        &current.panes[0].frame,
        TerminalViewFrame::Full(view) if view.revision == 2
    ));
}

#[test]
fn delta_composition_keeps_later_cell_values_and_latest_metadata() {
    let previous = TerminalViewDelta {
        selection: None,
        base_revision: 4,
        revision: 5,
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: vec![TerminalCellRun {
            start: 1,
            cells: vec![terminal_cell("A"), terminal_cell("B")],
        }],
    };
    let next = TerminalViewDelta {
        selection: None,
        base_revision: 5,
        revision: 6,
        display_offset: 3,
        mouse_tracking: TerminalMouseTracking::None,
        cursor: None,
        runs: vec![TerminalCellRun {
            start: 2,
            cells: vec![terminal_cell("C")],
        }],
    };

    let merged = merge_terminal_deltas(previous, next).unwrap();
    assert_eq!((merged.base_revision, merged.revision), (4, 6));
    assert_eq!(merged.display_offset, 3);
    assert_eq!(merged.runs.len(), 1);
    assert_eq!(merged.runs[0].start, 1);
    assert_eq!(merged.runs[0].cells[0].text.as_str(), "A");
    assert_eq!(merged.runs[0].cells[1].text.as_str(), "C");
}
