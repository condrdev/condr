use condr_core::protocol::*;
use condr_core::{
    AgentSnapshot, PaneId, SplitDirection, TabId, TerminalView, TerminalViewFrame, WorkspaceId,
};
use std::path::PathBuf;

#[test]
fn hello_round_trip_uses_length_prefix() {
    let message = ClientHandshake::Hello(Hello::new("test"));
    let mut bytes = Vec::new();
    write_message(&mut bytes, &message).unwrap();
    assert_eq!(
        u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize,
        bytes.len() - 4
    );
    assert_eq!(
        read_message::<_, ClientHandshake>(&mut bytes.as_slice()).unwrap(),
        message
    );
}

/// The handshake is frozen (ADR 0027): these bytes are what every past and future build
/// puts on the wire first. A change here is a change no released Condr can read.
#[test]
fn handshake_wire_layout_is_frozen() {
    let hello = ClientHandshake::Hello(Hello {
        protocol: 7,
        build: "1.2.3+abc".into(),
        client_name: "gui".into(),
    });
    let mut bytes = Vec::new();
    write_message(&mut bytes, &hello).unwrap();
    assert_eq!(
        bytes,
        [
            [36, 0, 0, 0].as_slice(), // frame length
            &[0, 0, 0, 0],            // ClientHandshake::Hello
            &[7, 0, 0, 0],            // protocol
            &[9, 0, 0, 0, 0, 0, 0, 0],
            b"1.2.3+abc", // build
            &[3, 0, 0, 0, 0, 0, 0, 0],
            b"gui", // client_name
        ]
        .concat()
    );

    let welcome = Welcome {
        protocol: 7,
        build: "1.2.3".into(),
        server_id: ServerId(0x0102),
        session_id: SessionId(1),
        refusal: Some(Refusal::IncompatibleProtocol),
    };
    let mut bytes = Vec::new();
    write_message(&mut bytes, &welcome).unwrap();
    assert_eq!(
        bytes,
        [
            [38, 0, 0, 0].as_slice(), // frame length
            &[7, 0, 0, 0],            // protocol
            &[5, 0, 0, 0, 0, 0, 0, 0],
            b"1.2.3",                  // build
            &[2, 1, 0, 0, 0, 0, 0, 0], // server_id
            &[1, 0, 0, 0, 0, 0, 0, 0], // session_id
            &[1],
            &[0, 0, 0, 0], // Some(Refusal::IncompatibleProtocol)
        ]
        .concat()
    );
    assert_eq!(
        read_message::<_, Welcome>(&mut bytes.as_slice()).unwrap(),
        welcome
    );
}

#[test]
fn layout_command_round_trip_preserves_creation_options() {
    let message = ClientMessage::Layout {
        server_id: ServerId(4),
        session_id: SessionId(1),
        request_id: 9,
        command: LayoutCommand::CreateWorkspace {
            name: Some("worker".into()),
            root_directory: PathBuf::from("projects/condr"),
        },
    };
    let mut bytes = Vec::new();
    write_message(&mut bytes, &message).unwrap();
    assert_eq!(
        read_message::<_, ClientMessage>(&mut bytes.as_slice()).unwrap(),
        message
    );
}

#[test]
fn worktree_command_round_trip_preserves_its_target() {
    let mut session = condr_core::Session::new();
    let parent_workspace_id = session
        .create_workspace(PathBuf::from("projects/condr"))
        .expect("Workspace capacity");
    let message = ClientMessage::Layout {
        server_id: ServerId(4),
        session_id: SessionId(1),
        request_id: 10,
        command: LayoutCommand::CreateWorktree {
            parent_workspace_id,
            branch: "feature/phase-five".into(),
        },
    };
    let mut bytes = Vec::new();
    write_message(&mut bytes, &message).unwrap();
    assert_eq!(
        read_message::<_, ClientMessage>(&mut bytes.as_slice()).unwrap(),
        message
    );
}

#[test]
fn subscription_rejection_round_trip_preserves_identity() {
    let message = ServerMessage::SubscriptionRejected {
        server_id: ServerId(4),
        session_id: SessionId(7),
        reason: "event cursor expired".into(),
    };
    let mut bytes = Vec::new();
    write_message(&mut bytes, &message).unwrap();
    assert_eq!(
        read_message::<_, ServerMessage>(&mut bytes.as_slice()).unwrap(),
        message
    );
}

#[test]
fn snapshot_rejection_round_trip_preserves_identity() {
    let message = ServerMessage::SnapshotRejected {
        server_id: ServerId(4),
        session_id: SessionId(7),
        reason: "unknown Session".into(),
    };
    let mut bytes = Vec::new();
    write_message(&mut bytes, &message).unwrap();
    assert_eq!(
        read_message::<_, ServerMessage>(&mut bytes.as_slice()).unwrap(),
        message
    );
}

#[test]
fn layout_rejection_round_trip_keeps_request_correlation() {
    let message = ServerMessage::LayoutRejected {
        server_id: ServerId(4),
        session_id: SessionId(7),
        request_id: 11,
        reason: "unknown Workspace".into(),
    };
    let mut bytes = Vec::new();
    write_message(&mut bytes, &message).unwrap();
    assert_eq!(
        read_message::<_, ServerMessage>(&mut bytes.as_slice()).unwrap(),
        message
    );
}

#[test]
fn layout_applied_round_trip_keeps_request_and_sequence_correlation() {
    let message = ServerMessage::LayoutApplied {
        server_id: ServerId(4),
        session_id: SessionId(7),
        request_id: 12,
        sequence: 31,
        result: LayoutResult::WorkspaceCreated {
            workspace_id: WorkspaceId::from_u64(21),
            tab_id: TabId::from_u64(22),
            pane_id: PaneId::from_u64(23),
        },
    };
    let mut bytes = Vec::new();
    write_message(&mut bytes, &message).unwrap();
    assert_eq!(
        read_message::<_, ServerMessage>(&mut bytes.as_slice()).unwrap(),
        message
    );
}

#[test]
fn bootstrap_header_round_trip_uses_the_flat_snapshot_schema() {
    let mut session = condr_core::Session::new();
    session
        .create_workspace(PathBuf::from("projects/condr"))
        .expect("Workspace capacity");
    let pane_id = session.workspaces()[0].tabs()[0]
        .focused_pane()
        .unwrap()
        .id();
    session
        .split_pane(pane_id, SplitDirection::Horizontal, 0.5)
        .expect("Pane exists");
    let message = ServerMessage::Bootstrap(BootstrapHeader {
        settings: Default::default(),
        server_id: ServerId(4),
        runtime_epoch: RuntimeEpoch(5),
        session_id: SessionId(1),
        sequence: 0,
        snapshot: session.snapshot(),
        batch_count: 0,
    });

    let mut bytes = Vec::new();
    write_message(&mut bytes, &message).unwrap();
    assert_eq!(
        read_message::<_, ServerMessage>(&mut bytes.as_slice()).unwrap(),
        message
    );
}

#[test]
fn oversized_frame_is_rejected_before_allocation() {
    let bytes = ((MAX_FRAME_SIZE as u32) + 1).to_le_bytes();
    assert!(matches!(
        read_message::<_, ClientMessage>(&mut bytes.as_slice()),
        Err(FramingError::Oversized { .. })
    ));
}

#[test]
fn forged_collection_length_is_rejected_without_allocating_it() {
    let mut payload = Vec::new();
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    payload.extend_from_slice(&u64::MAX.to_le_bytes());
    let mut frame = Vec::from((payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);

    assert!(matches!(
        read_message::<_, ClientMessage>(&mut frame.as_slice()),
        Err(FramingError::Codec(_))
    ));
}

#[test]
fn bootstrap_assembler_reassembles_multiple_record_chunks() {
    let (mut header, pane_id, workspace_id) = bootstrap_header(0);
    let records = vec![
        BootstrapRecord::Terminal(terminal_snapshot(
            pane_id,
            "x".repeat(MAX_CHUNK_PAYLOAD_SIZE + 1_024),
        )),
        BootstrapRecord::Agent(PaneAgentSnapshot {
            pane_id,
            agent: AgentSnapshot {
                session_id: None,
                kind: condr_core::AgentKind::Codex,
                state: condr_core::AgentState::Working,
                blocked_on: None,
            },
        }),
        BootstrapRecord::WorkspaceGit(WorkspaceGitSnapshot {
            workspace_id,
            branch: Some("feature/wire-chunks".into()),
            linked_worktree: true,
            upstream: Some(condr_core::GitUpstream {
                ahead: 2,
                behind: 1,
            }),
            changes: Default::default(),
        }),
        BootstrapRecord::ZoomedPane(pane_id),
    ];
    let batches = bootstrap_batches(header.server_id, header.session_id, &records);
    assert!(batches.len() > records.len());
    header.batch_count = batches.len() as u32;

    let mut assembler = BootstrapAssembler::new(header).unwrap();
    for batch in batches {
        assembler.push(batch).unwrap();
    }
    let assembled = assembler.finish().unwrap();

    assert_eq!(
        assembled.terminals,
        vec![match records[0].clone() {
            BootstrapRecord::Terminal(terminal) => terminal,
            _ => unreachable!(),
        }]
    );
    assert_eq!(
        assembled.agents,
        vec![match records[1].clone() {
            BootstrapRecord::Agent(agent) => agent,
            _ => unreachable!(),
        }]
    );
    assert_eq!(
        assembled.workspace_git,
        vec![match records[2].clone() {
            BootstrapRecord::WorkspaceGit(git) => git,
            _ => unreachable!(),
        }]
    );
    assert_eq!(assembled.zoomed_panes, vec![pane_id]);
}

#[test]
fn bootstrap_assembler_rejects_order_identity_and_chunk_errors() {
    let (header, pane_id, _) = bootstrap_header(1);
    let payload = encode_bootstrap_record(&BootstrapRecord::ZoomedPane(pane_id)).unwrap();

    let mut assembler = BootstrapAssembler::new(header.clone()).unwrap();
    let mut out_of_order = bootstrap_batch(&header, 1, 0, 0, 1, payload.clone());
    assert!(assembler.push(out_of_order.clone()).is_err());

    out_of_order.batch_index = 0;
    out_of_order.server_id = ServerId(header.server_id.0 + 1);
    let mut assembler = BootstrapAssembler::new(header.clone()).unwrap();
    assert!(assembler.push(out_of_order).is_err());

    let mut assembler = BootstrapAssembler::new(header.clone()).unwrap();
    assert!(
        assembler
            .push(bootstrap_batch(&header, 0, 0, 1, 2, payload.clone()))
            .is_err()
    );

    let mut assembler = BootstrapAssembler::new(header).unwrap();
    assert!(
        assembler
            .push(BootstrapBatch {
                server_id: ServerId(4),
                session_id: SessionId(1),
                batch_index: 0,
                record_index: 0,
                chunk_index: 0,
                chunk_count: 1,
                payload: vec![0; MAX_CHUNK_PAYLOAD_SIZE + 1],
            })
            .is_err()
    );
}

#[test]
fn maximum_chunk_payloads_fit_the_outer_protocol_frame() {
    let (header, pane_id, _) = bootstrap_header(1);
    let messages = [
        ServerMessage::BootstrapBatch(bootstrap_batch(
            &header,
            0,
            0,
            0,
            1,
            vec![0; MAX_CHUNK_PAYLOAD_SIZE],
        )),
        ServerMessage::TerminalFrameChunk(TerminalFrameChunk {
            server_id: header.server_id,
            session_id: header.session_id,
            pane_id,
            revision: 1,
            chunk_index: 0,
            chunk_count: 1,
            payload: vec![0; MAX_CHUNK_PAYLOAD_SIZE],
        }),
    ];

    for message in messages {
        let mut frame = Vec::new();
        write_message(&mut frame, &message).unwrap();
        assert!(frame.len() <= MAX_FRAME_SIZE + 4);
    }
}

#[test]
fn bootstrap_assembler_rejects_oversized_records_before_decode() {
    let (header, _, _) = bootstrap_header(17);
    let mut assembler = BootstrapAssembler::new(header.clone()).unwrap();
    let full_chunks = MAX_CHUNKED_RECORD_SIZE / MAX_CHUNK_PAYLOAD_SIZE;
    let tail = MAX_CHUNKED_RECORD_SIZE % MAX_CHUNK_PAYLOAD_SIZE;
    assert_eq!(full_chunks, 16);
    for chunk_index in 0..full_chunks {
        assembler
            .push(bootstrap_batch(
                &header,
                chunk_index as u32,
                0,
                chunk_index as u32,
                17,
                vec![0; MAX_CHUNK_PAYLOAD_SIZE],
            ))
            .unwrap();
    }
    assert!(
        assembler
            .push(bootstrap_batch(
                &header,
                full_chunks as u32,
                0,
                full_chunks as u32,
                17,
                vec![0; tail + 1],
            ))
            .is_err()
    );
}

#[test]
fn chunked_record_codecs_reject_trailing_and_oversized_payloads() {
    let (_, pane_id, _) = bootstrap_header(0);
    let record = BootstrapRecord::ZoomedPane(pane_id);
    let mut encoded = encode_bootstrap_record(&record).unwrap();
    assert_eq!(decode_bootstrap_record(&encoded).unwrap(), record);
    encoded.push(0);
    assert!(decode_bootstrap_record(&encoded).is_err());

    let frame = PaneTerminalFrame {
        pane_id,
        frame: TerminalViewFrame::Full(terminal_snapshot(pane_id, "frame".into()).view),
    };
    let encoded = encode_pane_terminal_frame(&frame).unwrap();
    assert_eq!(decode_pane_terminal_frame(&encoded).unwrap(), frame);
    assert!(decode_pane_terminal_frame(&vec![0; MAX_CHUNKED_RECORD_SIZE + 1]).is_err());
}

#[test]
fn bootstrap_assembler_rejects_duplicate_ids_per_record_kind() {
    let (header, pane_id, workspace_id) = bootstrap_header(2);
    let duplicate_records = [
        BootstrapRecord::Terminal(terminal_snapshot(pane_id, "terminal".into())),
        BootstrapRecord::Agent(PaneAgentSnapshot {
            pane_id,
            agent: AgentSnapshot {
                session_id: None,
                kind: condr_core::AgentKind::Claude,
                state: condr_core::AgentState::Idle,
                blocked_on: None,
            },
        }),
        BootstrapRecord::WorkspaceGit(WorkspaceGitSnapshot {
            workspace_id,
            branch: None,
            linked_worktree: false,
            upstream: None,
            changes: Default::default(),
        }),
        BootstrapRecord::ZoomedPane(pane_id),
    ];

    for record in duplicate_records {
        let payload = encode_bootstrap_record(&record).unwrap();
        let mut assembler = BootstrapAssembler::new(header.clone()).unwrap();
        assembler
            .push(bootstrap_batch(&header, 0, 0, 0, 1, payload.clone()))
            .unwrap();
        assert!(
            assembler
                .push(bootstrap_batch(&header, 1, 1, 0, 1, payload))
                .is_err()
        );
    }
}

#[test]
fn bootstrap_assembler_finishes_an_empty_bootstrap() {
    let (header, _, _) = bootstrap_header(0);
    let expected = SessionBootstrap {
        settings: Default::default(),
        server_id: header.server_id,
        runtime_epoch: header.runtime_epoch,
        session_id: header.session_id,
        sequence: header.sequence,
        snapshot: header.snapshot.clone(),
        terminals: Vec::new(),
        agents: Vec::new(),
        workspace_git: Vec::new(),
        zoomed_panes: Vec::new(),
    };
    assert_eq!(
        BootstrapAssembler::new(header).unwrap().finish().unwrap(),
        expected
    );
}

fn bootstrap_header(batch_count: u32) -> (BootstrapHeader, PaneId, WorkspaceId) {
    let mut session = condr_core::Session::new();
    let workspace_id = session
        .create_workspace(PathBuf::from("projects/condr"))
        .expect("Workspace capacity");
    let pane_id = session.workspaces()[0].tabs()[0]
        .focused_pane()
        .unwrap()
        .id();
    (
        BootstrapHeader {
            settings: Default::default(),
            server_id: ServerId(4),
            runtime_epoch: RuntimeEpoch(5),
            session_id: SessionId(1),
            sequence: 7,
            snapshot: session.snapshot(),
            batch_count,
        },
        pane_id,
        workspace_id,
    )
}

fn terminal_snapshot(pane_id: PaneId, text: String) -> PaneTerminalSnapshot {
    PaneTerminalSnapshot {
        pane_id,
        view: TerminalView {
            selection: None,
            revision: 11,
            size: condr_core::TerminalSize::new(1, 1),
            display_offset: 0,
            mouse_tracking: condr_core::TerminalMouseTracking::None,
            cells: vec![condr_core::TerminalCell {
                text: text.into(),
                foreground: condr_core::TerminalColor::Named(0),
                background: condr_core::TerminalColor::Named(0),
                flags: 0,
                hyperlink: None,
            }],
            cursor: None,
        },
        exited: false,
        title: None,
        attention: false,
    }
}

fn bootstrap_batches(
    server_id: ServerId,
    session_id: SessionId,
    records: &[BootstrapRecord],
) -> Vec<BootstrapBatch> {
    let mut batches = Vec::new();
    for (record_index, record) in records.iter().enumerate() {
        let payload = encode_bootstrap_record(record).unwrap();
        let chunk_count = payload.len().div_ceil(MAX_CHUNK_PAYLOAD_SIZE) as u32;
        for (chunk_index, payload) in payload.chunks(MAX_CHUNK_PAYLOAD_SIZE).enumerate() {
            batches.push(BootstrapBatch {
                server_id,
                session_id,
                batch_index: batches.len() as u32,
                record_index: record_index as u32,
                chunk_index: chunk_index as u32,
                chunk_count,
                payload: payload.to_vec(),
            });
        }
    }
    batches
}

fn bootstrap_batch(
    header: &BootstrapHeader,
    batch_index: u32,
    record_index: u32,
    chunk_index: u32,
    chunk_count: u32,
    payload: Vec<u8>,
) -> BootstrapBatch {
    BootstrapBatch {
        server_id: header.server_id,
        session_id: header.session_id,
        batch_index,
        record_index,
        chunk_index,
        chunk_count,
        payload,
    }
}

#[test]
fn bootstrap_batch_frame_overhead_covers_the_envelope_and_framing() {
    let mut bytes = Vec::new();
    write_message(
        &mut bytes,
        &ServerMessage::BootstrapBatch(BootstrapBatch {
            server_id: ServerId(u64::MAX),
            session_id: SessionId(u64::MAX),
            batch_index: u32::MAX,
            record_index: u32::MAX,
            chunk_index: u32::MAX,
            chunk_count: u32::MAX,
            payload: Vec::new(),
        }),
    )
    .unwrap();
    assert!(bytes.len() <= BOOTSTRAP_BATCH_FRAME_OVERHEAD);
}

#[test]
fn a_pasted_image_gets_its_own_frame_allowance_and_nothing_else_does() {
    let image = ClientMessage::PasteImage {
        server_id: ServerId(1),
        session_id: SessionId(2),
        pane_id: PaneId::from_u64(3),
        format: ClipboardImageFormat::Png,
        bytes: vec![7; 3 * 1024 * 1024],
    };
    let mut frame = Vec::new();
    assert!(
        write_message(&mut frame, &image).is_err(),
        "over the normal limit"
    );
    write_client_message(&mut frame, &image).unwrap();
    assert!(matches!(
        read_message::<_, ClientMessage>(&mut frame.as_slice()),
        Err(FramingError::Oversized { .. })
    ));
    let (decoded, size) =
        read_message_with_limit::<_, ClientMessage>(&mut frame.as_slice(), MAX_IMAGE_FRAME_SIZE)
            .unwrap();
    assert_eq!(decoded, image);
    assert!(size > MAX_FRAME_SIZE);

    let ping = ClientMessage::Ping {
        server_id: ServerId(1),
        nonce: 9,
    };
    assert_eq!(ping.frame_limit(), MAX_FRAME_SIZE);
    assert_eq!(ClipboardImageFormat::Jpeg.extension(), "jpg");
}

#[test]
fn server_admin_commands_round_trip() {
    let message = ClientMessage::ServerAdmin {
        server_id: ServerId(7),
        command: ServerAdminCommand::SaveListen {
            address: Some("127.0.0.1:4242".into()),
        },
    };
    let mut frame = Vec::new();
    write_client_message(&mut frame, &message).unwrap();
    assert_eq!(
        read_message::<_, ClientMessage>(&mut frame.as_slice()).unwrap(),
        message
    );
}
