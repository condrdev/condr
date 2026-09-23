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

/// Two cells, "a" and "bc", named colors, no flags, the first linked.
fn cells() -> TerminalCells {
    TerminalCells {
        text: "abc".into(),
        text_len: vec![1, 2],
        foreground: vec![256 << 2 | 1, 256 << 2 | 1],
        background: vec![257 << 2 | 1, 257 << 2 | 1],
        flags: Vec::new(),
        hyperlink: vec![1, 0],
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
            },
        )),
    };
    let Some(BootstrapRecord::Terminal(terminal)) = decode_bootstrap_record(record).unwrap() else {
        panic!("a Terminal record with a newer mouse mode is still applied");
    };
    assert_eq!(terminal.view.mouse_tracking, TerminalMouseTracking::None);

    let event = super::SessionEvent {
        event: Some(session_event::Event::AgentChanged(super::AgentChanged {
            pane_id: 1,
            agent: Some(super::AgentSnapshot {
                session_id: None,
                kind: 99,
                state: 99,
                blocked_on: None,
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
}

#[test]
fn an_unset_enum_is_malformed() {
    let agent = super::AgentSnapshot {
        session_id: None,
        kind: 0,
        state: 1,
        blocked_on: None,
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
    let omitted = super::SessionEvent::try_from(&SessionEvent::Omitted).unwrap();
    assert_eq!(
        SessionEvent::try_from(omitted).unwrap(),
        SessionEvent::Omitted
    );
}
