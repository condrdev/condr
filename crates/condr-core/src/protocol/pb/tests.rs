//! The rules ADR 0028 sets for values a newer peer sends, and for peer input that is
//! wrong on any protocol.

use super::*;
use crate::protocol::{
    BootstrapRecord, ClientMessage, ServerMessage, SessionEvent, UnknownMessage,
};
use crate::{AgentKind, TerminalMouseTracking, TerminalView};

fn view(cells: TerminalCells, rows: u32, columns: u32) -> super::TerminalView {
    super::TerminalView {
        revision: 1,
        size: Some(TerminalSize {
            rows,
            columns,
            cell_width: 0,
            cell_height: 0,
        }),
        display_offset: 0,
        mouse_tracking: super::TerminalMouseTracking::None as i32,
        hyperlinks: vec!["https://example.com".into()],
        cells: Some(cells),
        cursor: None,
        selection: None,
    }
}

/// Two cells, "a" and "bc", default colors as a peer from before names them, no flags,
/// the first linked.
fn cells() -> TerminalCells {
    TerminalCells {
        text: "abc".into(),
        text_len: vec![1, 2],
        foreground: vec![256 << 2 | 1, 256 << 2 | 1],
        background: vec![257 << 2 | 1, 257 << 2 | 1],
        flags: Vec::new(),
        hyperlink: vec![1, 0],
        underline: Vec::new(),
    }
}

#[test]
fn a_valid_columnar_view_decodes() {
    let decoded = TerminalView::try_from(view(cells(), 1, 2)).unwrap();
    assert_eq!(decoded.cells[1].text, "bc");
    assert_eq!(
        decoded.cells[0].hyperlink.as_deref(),
        Some("https://example.com")
    );
    assert_eq!(decoded.cells[1].hyperlink, None);
}

#[test]
fn malformed_columnar_views_are_refused_without_a_panic() {
    type Corrupt = fn(&mut TerminalCells);
    let cases: [(&str, Corrupt, (u32, u32)); 8] = [
        (
            "unequal columns",
            |cells| {
                cells.foreground.pop();
            },
            (1, 2),
        ),
        ("length sum short", |cells| cells.text_len[1] = 1, (1, 2)),
        ("length sum long", |cells| cells.text_len[1] = 3, (1, 2)),
        (
            "off a character boundary",
            |cells| {
                cells.text = "aé".into();
                cells.text_len = vec![2, 1];
            },
            (1, 2),
        ),
        (
            "hyperlink out of range",
            |cells| cells.hyperlink[0] = 2,
            (1, 2),
        ),
        (
            "undefined color kind",
            |cells| cells.background[0] = 7 << 2,
            (1, 2),
        ),
        (
            "cell over the byte limit",
            |cells| {
                cells.text = "x".repeat(258);
                cells.text_len = vec![1, 257];
            },
            (1, 2),
        ),
        ("cell count against size", |_| {}, (2, 2)),
    ];
    for (name, corrupt, (rows, columns)) in cases {
        let mut columns_value = cells();
        corrupt(&mut columns_value);
        assert!(
            matches!(
                TerminalView::try_from(view(columns_value, rows, columns)),
                Err(WireError::Malformed(_))
            ),
            "{name}"
        );
    }
}

#[test]
fn unknown_enum_values_decode_to_their_fallback() {
    let mut terminal = view(cells(), 1, 2);
    terminal.mouse_tracking = 99;
    let record = super::BootstrapRecord {
        record: Some(bootstrap_record::Record::Terminal(
            super::PaneTerminalSnapshot {
                pane_id: 1,
                view: Some(terminal),
                exited: false,
                title: None,
                attention: false,
                progress: None,
            },
        )),
    };
    let Some(BootstrapRecord::Terminal(terminal)) = decode_bootstrap_record(record).unwrap() else {
        panic!("a Terminal record with a newer mouse mode is still applied");
    };
    assert_eq!(
        terminal.view.unwrap().mouse_tracking,
        TerminalMouseTracking::None
    );

    let event = super::SessionEvent {
        event: Some(session_event::Event::AgentChanged(super::AgentChanged {
            pane_id: 1,
            agent: Some(super::AgentSnapshot {
                session_id: None,
                kind: 99,
                state: 99,
                detail: None,
            }),
        })),
    };
    let SessionEvent::AgentChanged {
        agent: Some(agent), ..
    } = SessionEvent::try_from(event).unwrap()
    else {
        panic!("an agent of a newer kind is still reported");
    };
    assert_eq!(agent.kind, AgentKind::Other);
    assert_eq!(agent.state, crate::AgentState::Unknown);

    // A key from a peer that predates event kinds, or that names a newer one, is a press.
    for kind in [0, 99] {
        let key = super::TerminalCommand {
            command: Some(terminal_command::Command::Key(TerminalKeyCommand {
                key: Some(super::TerminalKey {
                    key: Some(terminal_key::Key::Function(5)),
                }),
                modifiers: None,
                kind,
            })),
        };
        assert!(
            matches!(
                crate::TerminalCommand::try_from(key).unwrap(),
                crate::TerminalCommand::Key {
                    kind: crate::TerminalKeyEventKind::Press,
                    ..
                }
            ),
            "{kind}"
        );
    }
}

#[test]
fn an_unset_enum_is_malformed() {
    let agent = super::AgentSnapshot {
        session_id: None,
        kind: 0,
        state: 1,
        detail: None,
    };
    assert!(matches!(
        crate::AgentSnapshot::try_from(agent),
        Err(WireError::Malformed(_))
    ));
}

#[test]
fn an_unknown_oneof_makes_the_enclosing_message_unknown() {
    use server_message::Message;
    let server = |message| {
        ServerMessage::try_from(super::ServerMessage {
            message: Some(message),
        })
        .unwrap()
    };

    assert_eq!(
        ServerMessage::try_from(super::ServerMessage { message: None }).unwrap(),
        ServerMessage::Unknown(UnknownMessage::Reply)
    );
    assert_eq!(
        server(Message::Event(SessionEventMessage {
            server_id: 1,
            session_id: 1,
            sequence: 7,
            event: Some(super::SessionEvent { event: None }),
        })),
        ServerMessage::Unknown(UnknownMessage::Event { sequence: 7 })
    );
    assert_eq!(
        server(Message::TerminalFrame(super::TerminalFrameBatch {
            server_id: 1,
            session_id: 1,
            panes: vec![super::PaneTerminalFrame {
                pane_id: 1,
                frame: Some(super::TerminalViewFrame { frame: None }),
            }],
        })),
        ServerMessage::Unknown(UnknownMessage::TerminalFrame)
    );
    assert_eq!(
        server(Message::LayoutApplied(super::LayoutApplied {
            server_id: 1,
            session_id: 1,
            request_id: 2,
            sequence: 3,
            result: Some(super::LayoutResult { result: None }),
        })),
        ServerMessage::Unknown(UnknownMessage::Reply)
    );

    // A command naming a mouse button this build does not know is not run.
    let click = super::ClientMessage {
        message: Some(client_message::Message::Terminal(TerminalRequest {
            server_id: 1,
            session_id: 1,
            pane_id: 1,
            command: Some(super::TerminalCommand {
                command: Some(terminal_command::Command::Mouse(
                    super::TerminalMouseEvent {
                        event: Some(terminal_mouse_event::Event::Button(
                            TerminalMouseButtonEvent {
                                button: 99,
                                pressed: true,
                                position: Some(TerminalMousePosition { row: 0, column: 0 }),
                                modifiers: None,
                            },
                        )),
                    },
                )),
            }),
        })),
    };
    assert_eq!(
        ClientMessage::try_from(click).unwrap(),
        ClientMessage::Unknown
    );
    assert_eq!(
        ClientMessage::try_from(super::ClientMessage { message: None }).unwrap(),
        ClientMessage::Unknown
    );
}

#[test]
fn an_unknown_bootstrap_record_is_skipped_and_an_omitted_event_round_trips() {
    assert_eq!(
        decode_bootstrap_record(super::BootstrapRecord { record: None }).unwrap(),
        None
    );
    for event in [
        SessionEvent::Omitted,
        SessionEvent::AgentUnseenChanged {
            pane_id: crate::PaneId::from_u64(3),
            unseen: true,
        },
    ] {
        let encoded = super::SessionEvent::try_from(&event).unwrap();
        assert_eq!(SessionEvent::try_from(encoded).unwrap(), event);
    }
}

#[test]
fn the_proxy_setting_round_trips_and_an_older_or_newer_peer_reads_as_system() {
    use crate::protocol::{ServerSetting, ServerSettings};
    use crate::{ProxyMode, ProxySetting};
    let settings = ServerSettings {
        shell: "nu".into(),
        default_shell: "sh".into(),
        proxy: ProxySetting {
            mode: ProxyMode::Manual,
            url: "http://proxy:8080".into(),
        },
        shell_integration: true,
    };
    let changed = SessionEvent::ServerSettingsChanged {
        settings: settings.clone(),
    };
    let encoded = super::SessionEvent::try_from(&changed).unwrap();
    assert_eq!(SessionEvent::try_from(encoded).unwrap(), changed);

    let event = |proxy| super::SessionEvent {
        event: Some(session_event::Event::ServerSettingsChanged(
            super::ServerSettings {
                shell: String::new(),
                default_shell: String::new(),
                proxy,
                shell_integration: false,
            },
        )),
    };
    for proxy in [
        None,
        Some(super::ProxySetting {
            mode: 99,
            url: String::new(),
        }),
    ] {
        let SessionEvent::ServerSettingsChanged { settings } =
            SessionEvent::try_from(event(proxy)).unwrap()
        else {
            panic!("settings stay settings");
        };
        assert_eq!(settings.proxy, ProxySetting::default());
    }

    for setting in [
        ServerSetting::Shell("fish".into()),
        ServerSetting::ProxyMode(ProxyMode::None),
        ServerSetting::ProxyUrl("https://proxy".into()),
    ] {
        let message = ClientMessage::SetServerSettings {
            server_id: crate::protocol::ServerId(1),
            setting,
        };
        let encoded = super::ClientMessage::try_from(&message).unwrap();
        assert_eq!(ClientMessage::try_from(encoded).unwrap(), message);
    }
    // Setting a mode this build does not know is not run as System.
    let newer = super::ClientMessage {
        message: Some(client_message::Message::SetServerSettings(
            super::SetServerSettings {
                server_id: 1,
                setting: Some(set_server_settings::Setting::ProxyMode(99)),
            },
        )),
    };
    assert_eq!(
        ClientMessage::try_from(newer).unwrap(),
        ClientMessage::Unknown
    );
}

#[test]
fn terminal_colors_round_trip_and_need_all_sixteen_ansi_colours() {
    let colors = crate::TerminalColors {
        background: 0xfafafa,
        ..crate::TerminalColors::default()
    };
    let message = ClientMessage::TerminalColors(colors);
    let mut encoded = super::ClientMessage::try_from(&message).unwrap();
    assert_eq!(ClientMessage::try_from(encoded.clone()).unwrap(), message);

    let Some(super::client_message::Message::TerminalColors(wire)) = encoded.message.as_mut()
    else {
        unreachable!()
    };
    wire.foreground = 0xff12_3456;
    wire.ansi.pop();
    assert!(ClientMessage::try_from(encoded.clone()).is_err());
    let Some(super::client_message::Message::TerminalColors(wire)) = encoded.message.as_mut()
    else {
        unreachable!()
    };
    wire.ansi.push(0);
    let ClientMessage::TerminalColors(decoded) = ClientMessage::try_from(encoded).unwrap() else {
        unreachable!()
    };
    assert_eq!(decoded.foreground, 0x12_3456, "only RGB is kept");
}

#[test]
fn a_watched_set_and_a_terminal_record_without_a_view_round_trip() {
    let message = ClientMessage::WatchTerminals {
        panes: [crate::PaneId::from_u64(3), crate::PaneId::from_u64(5)].into(),
    };
    let encoded = super::ClientMessage::try_from(&message).unwrap();
    assert_eq!(ClientMessage::try_from(encoded).unwrap(), message);

    // ADR 0041: a Pane outside the set still has its record, without a view.
    let record = BootstrapRecord::Terminal(crate::protocol::PaneTerminalSnapshot {
        pane_id: crate::PaneId::from_u64(3),
        view: None,
        exited: true,
        title: Some("build".into()),
        attention: true,
        progress: Some(crate::TerminalProgress {
            state: crate::TerminalProgressState::Paused,
            percent: Some(40),
        }),
    });
    assert_eq!(
        decode_bootstrap_record((&record).into()).unwrap(),
        Some(record)
    );
}

#[test]
fn a_submitted_draft_round_trips() {
    let command = crate::TerminalCommand::Submit("echo one\necho two".into());
    let wire = super::TerminalCommand::from(&command);
    assert_eq!(crate::TerminalCommand::try_from(wire).unwrap(), command);
}

#[test]
fn file_mutations_and_their_replies_round_trip() {
    use crate::{FileKind, FileMutation, FileMutationResult, WorkspaceId};
    let mutations = [
        FileMutation::Write {
            path: "plan.md".into(),
            text: "one\r\n".into(),
            base: Some(0),
        },
        FileMutation::Write {
            path: "plan.md".into(),
            text: String::new(),
            base: None,
        },
        FileMutation::Create {
            path: "docs".into(),
            kind: FileKind::Directory,
        },
        FileMutation::Rename {
            from: "a.txt".into(),
            to: "A.txt".into(),
        },
        FileMutation::Delete {
            path: "build".into(),
        },
    ];
    for mutation in mutations {
        let message = ClientMessage::MutateFile {
            server_id: crate::protocol::ServerId(1),
            session_id: crate::protocol::SessionId(2),
            request_id: 3,
            workspace_id: WorkspaceId::from_u64(4),
            mutation,
        };
        let encoded = super::ClientMessage::try_from(&message).unwrap();
        assert_eq!(ClientMessage::try_from(encoded).unwrap(), message);
    }
    // A version of zero is a version, not its absence.
    for result in [
        FileMutationResult::Done { version: Some(0) },
        FileMutationResult::Done { version: None },
        FileMutationResult::Conflict,
        FileMutationResult::Invalid("already exists".into()),
        FileMutationResult::Failed("disk full".into()),
    ] {
        let message = ServerMessage::FileMutated {
            request_id: 3,
            workspace_id: WorkspaceId::from_u64(4),
            result,
        };
        let encoded = super::ServerMessage::try_from(&message).unwrap();
        assert_eq!(ServerMessage::try_from(encoded).unwrap(), message);
    }
    let text = crate::FileContent::Text {
        text: "x".into(),
        version: Some(0),
        lossy: false,
    };
    assert_eq!(
        crate::FileContent::try_from(super::FileContent::from(&text)).unwrap(),
        text
    );
}

#[test]
fn the_largest_legal_write_fits_one_frame() {
    // Three-byte characters up to the limit, under a long path.
    let text = "€".repeat(crate::MAX_FILE_BYTES as usize / 3);
    let message = ClientMessage::MutateFile {
        server_id: crate::protocol::ServerId(u64::MAX),
        session_id: crate::protocol::SessionId(u64::MAX),
        request_id: u64::MAX,
        workspace_id: crate::WorkspaceId::from_u64(u64::MAX),
        mutation: crate::FileMutation::Write {
            path: "a/".repeat(2000).into(),
            text,
            base: Some(u64::MAX),
        },
    };
    assert!(crate::protocol::encode_frame(&message, message.frame_limit()).is_ok());
}

#[test]
fn a_refresh_and_a_full_context_diff_round_trip() {
    for message in [
        ClientMessage::RefreshWorkspace {
            server_id: crate::protocol::ServerId(1),
            session_id: crate::protocol::SessionId(2),
            workspace_id: crate::WorkspaceId::from_u64(3),
        },
        ClientMessage::GitDiff {
            server_id: crate::protocol::ServerId(1),
            session_id: crate::protocol::SessionId(2),
            request_id: 4,
            workspace_id: crate::WorkspaceId::from_u64(3),
            path: "src/lib.rs".into(),
            against: crate::protocol::DiffBase::MergeBase,
            full_context: true,
            commit: Some("8f3c".into()),
        },
        ClientMessage::GitLog {
            server_id: crate::protocol::ServerId(1),
            session_id: crate::protocol::SessionId(2),
            request_id: 5,
            workspace_id: crate::WorkspaceId::from_u64(3),
            from: Some("8f3c".into()),
            skip: 100,
            limit: 100,
        },
        ClientMessage::GitCommit {
            server_id: crate::protocol::ServerId(1),
            session_id: crate::protocol::SessionId(2),
            request_id: 6,
            workspace_id: crate::WorkspaceId::from_u64(3),
            commit: "8f3c".into(),
        },
    ] {
        let encoded = super::ClientMessage::try_from(&message).unwrap();
        assert_eq!(ClientMessage::try_from(encoded).unwrap(), message);
    }
}

#[test]
fn history_replies_round_trip() {
    let changes = crate::GitChanges {
        entries: vec![crate::GitChangeEntry {
            path: "src/new.rs".into(),
            old_path: Some("src/old.rs".into()),
            status: crate::GitChangeStatus::Renamed,
            stat: Some(crate::GitDiffStat {
                added: 2,
                deleted: 1,
            }),
        }],
        truncated: false,
    };
    for message in [
        crate::protocol::ServerMessage::GitLog {
            request_id: 5,
            workspace_id: crate::WorkspaceId::from_u64(3),
            result: Ok(crate::GitLog {
                from: Some("8f3c".into()),
                commits: vec![crate::CommitSummary {
                    id: "8f3c".into(),
                    subject: "Fix the parser".into(),
                    author: "Ada".into(),
                    time: 1_700_000_000,
                }],
                more: true,
            }),
        },
        crate::protocol::ServerMessage::GitCommit {
            request_id: 6,
            workspace_id: crate::WorkspaceId::from_u64(3),
            commit: "8f3c".into(),
            result: Ok(crate::CommitDetails {
                id: "8f3c".into(),
                parents: vec!["1a2b".into()],
                author: "Ada".into(),
                time: 1_700_000_000,
                message: "Fix the parser\n\nIt dropped the last token.\n".into(),
                changes,
            }),
        },
        crate::protocol::ServerMessage::GitCommit {
            request_id: 7,
            workspace_id: crate::WorkspaceId::from_u64(3),
            commit: "dead".into(),
            result: Err("not a commit".into()),
        },
    ] {
        let encoded = super::ServerMessage::try_from(&message).unwrap();
        assert_eq!(
            crate::protocol::ServerMessage::try_from(encoded).unwrap(),
            message
        );
    }
}
