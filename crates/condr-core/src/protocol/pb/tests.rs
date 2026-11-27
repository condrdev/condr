//! The rules ADR 0028 sets for values a newer peer sends, and for peer input that is
//! wrong on any protocol.

use super::*;
use crate::protocol::{
    BootstrapRecord, ClientMessage, ServerId, ServerMessage, SessionEvent, UnknownMessage,
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
        history_rows: 0,
        search: None,
        mouse_tracking: super::TerminalMouseTracking::None as i32,
        hyperlinks: vec!["https://example.com".into()],
        cells: Some(cells),
        cursor: None,
        selection: None,
        images: None,
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
        SessionEvent::TerminalProgressChanged {
            pane_id: crate::PaneId::from_u64(3),
            progress: Some(crate::TerminalProgress {
                state: crate::TerminalProgressState::Error,
                percent: None,
            }),
        },
        SessionEvent::TerminalProgressChanged {
            pane_id: crate::PaneId::from_u64(3),
            progress: None,
        },
        SessionEvent::TerminalNotification {
            pane_id: crate::PaneId::from_u64(3),
            title: None,
            body: "Build finished".into(),
        },
        SessionEvent::CommandFinished {
            pane_id: crate::PaneId::from_u64(3),
            exit_code: Some(-1),
            duration_secs: 42,
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
fn terminal_colors_round_trip_and_need_at_least_sixteen_ansi_colours() {
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
    wire.ansi.extend([0, 0x00ff_00ff]);
    let ClientMessage::TerminalColors(decoded) = ClientMessage::try_from(encoded).unwrap() else {
        unreachable!()
    };
    assert_eq!(decoded.foreground, 0x12_3456, "only RGB is kept");
    assert_eq!(
        decoded.ansi[15], 0,
        "a newer peer's extra colours are ignored"
    );
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
        decode_bootstrap_record((&record).try_into().unwrap()).unwrap(),
        Some(record)
    );
}

fn task(id: u128, description: Option<&str>) -> crate::Task {
    crate::Task {
        id: crate::TaskId::from_u128(id),
        title: "Fix the login bug".into(),
        description: description.map(str::to_owned),
        repository: Some("/src/api".into()),
        order: -2,
        archived: false,
        workspace: Some(crate::WorkspaceId::from_u64(9)),
        made_workspace: true,
        starting: true,
        last: Some(crate::LastPlace {
            path: "/src/api-fix".into(),
            branch: Some("worktree/fix".into()),
            base_branch: Some("main".into()),
            managed: true,
            repository: Some("/src/api".into()),
        }),
    }
}

/// ADR 0068: the Tasks record, its events, every request and every reply round trip.
#[test]
fn tasks_round_trip_in_records_events_requests_and_replies() {
    let record = BootstrapRecord::Tasks(vec![task(1, None), task(2, None)]);
    assert_eq!(
        decode_bootstrap_record((&record).try_into().unwrap()).unwrap(),
        Some(record)
    );
    let empty = BootstrapRecord::Tasks(Vec::new());
    assert_eq!(
        decode_bootstrap_record((&empty).try_into().unwrap()).unwrap(),
        Some(empty),
        "an empty record still says the Server keeps Tasks"
    );
    for event in [
        SessionEvent::TaskChanged(task(3, None)),
        SessionEvent::TaskRemoved {
            task: crate::TaskId::from_u128(3),
        },
    ] {
        let wire = super::SessionEvent::try_from(&event).unwrap();
        assert_eq!(SessionEvent::try_from(wire).unwrap(), event);
    }
    let id = crate::TaskId::from_u128(4);
    for command in [
        crate::TaskCommand::Create {
            title: "t".into(),
            description: "d".into(),
            repository: None,
        },
        crate::TaskCommand::Edit {
            task: id,
            title: None,
            description: Some("d".into()),
            repository: Some(None),
        },
        crate::TaskCommand::Edit {
            task: id,
            title: Some("t".into()),
            description: None,
            repository: Some(Some("/src/api".into())),
        },
        crate::TaskCommand::Edit {
            task: id,
            title: None,
            description: None,
            repository: None,
        },
        crate::TaskCommand::Delete { task: id },
        crate::TaskCommand::Archive {
            task: id,
            close_workspace: true,
            remove_worktree: false,
        },
        crate::TaskCommand::Restore { task: id },
        crate::TaskCommand::Reorder { task: id, index: 3 },
        crate::TaskCommand::Get { task: id },
        crate::TaskCommand::List { archived: true },
        crate::TaskCommand::Start {
            task: id,
            place: None,
            agent: crate::AgentKind::Codex,
        },
        crate::TaskCommand::Start {
            task: id,
            place: Some(crate::StartPlace::Continue),
            agent: crate::AgentKind::Claude,
        },
        crate::TaskCommand::Start {
            task: id,
            place: Some(crate::StartPlace::Workspace(crate::WorkspaceId::from_u64(
                2,
            ))),
            agent: crate::AgentKind::OpenCode,
        },
        crate::TaskCommand::Start {
            task: id,
            place: Some(crate::StartPlace::NewWorktree {
                branch: Some("worktree/x".into()),
            }),
            agent: crate::AgentKind::Copilot,
        },
    ] {
        let message = ClientMessage::Task {
            server_id: ServerId(1),
            request_id: 5,
            command,
        };
        let wire = super::ClientMessage::try_from(&message).unwrap();
        assert_eq!(ClientMessage::try_from(wire).unwrap(), message);
    }
    for result in [
        Ok(crate::TaskResponse::Task(task(1, Some("body")))),
        Ok(crate::TaskResponse::Tasks(vec![task(1, None)])),
        Ok(crate::TaskResponse::Started {
            workspace_id: crate::WorkspaceId::from_u64(1),
            tab_id: crate::TabId::from_u64(2),
            pane_id: crate::PaneId::from_u64(3),
        }),
        Ok(crate::TaskResponse::Done),
        Err(crate::TaskError::new(crate::task_error::TASK_STARTING, "busy").at("workspace")),
    ] {
        let message = ServerMessage::Task {
            request_id: 7,
            result,
        };
        let wire = super::ServerMessage::try_from(&message).unwrap();
        assert_eq!(ServerMessage::try_from(wire).unwrap(), message);
    }
}

/// A Start naming an agent this build does not know is a request it cannot answer,
/// not a malformed one (ADR 0028).
#[test]
fn a_start_with_an_unknown_agent_is_an_unknown_request() {
    let message = super::ClientMessage {
        message: Some(client_message::Message::Task(TaskRequest {
            server_id: 1,
            request_id: 1,
            command: Some(super::TaskCommand {
                command: Some(task_command::Command::Start(TaskStart {
                    task: vec![1; 16],
                    place: None,
                    agent: 99,
                })),
            }),
        })),
    };
    assert_eq!(
        ClientMessage::try_from(message).unwrap(),
        ClientMessage::Unknown
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
        ClientMessage::GitOldFile {
            server_id: crate::protocol::ServerId(1),
            session_id: crate::protocol::SessionId(2),
            request_id: 7,
            workspace_id: crate::WorkspaceId::from_u64(3),
            path: "src/lib.rs".into(),
            against: crate::protocol::DiffBase::MergeBase,
        },
        ClientMessage::GitLog {
            server_id: crate::protocol::ServerId(1),
            session_id: crate::protocol::SessionId(2),
            request_id: 5,
            workspace_id: crate::WorkspaceId::from_u64(3),
            from: Some("8f3c".into()),
            skip: 100,
            limit: 100,
            repository: "api".into(),
        },
        ClientMessage::GitCommit {
            server_id: crate::protocol::ServerId(1),
            session_id: crate::protocol::SessionId(2),
            request_id: 6,
            workspace_id: crate::WorkspaceId::from_u64(3),
            commit: "8f3c".into(),
            repository: "api".into(),
        },
    ] {
        let encoded = super::ClientMessage::try_from(&message).unwrap();
        assert_eq!(ClientMessage::try_from(encoded).unwrap(), message);
    }
}

#[test]
fn repositories_under_the_root_ride_beside_the_fields_an_older_peer_reads() {
    use crate::protocol::{RepositoryGitSnapshot, WorkspaceGitSnapshot};
    let repository = |directory: &str, branch: &str| RepositoryGitSnapshot {
        directory: directory.into(),
        branch: Some(branch.into()),
        linked_worktree: false,
        upstream: Some(crate::GitUpstream {
            ahead: 1,
            behind: 0,
        }),
        changes: crate::GitChanges {
            entries: vec![crate::GitChangeEntry {
                path: relative_path::RelativePath::new(directory).join("lib.rs"),
                old_path: None,
                status: crate::GitChangeStatus::Modified,
                stat: None,
            }],
            truncated: false,
        },
        base: None,
        head: Some(crate::protocol::GitHead {
            commit: Some("8f3c".into()),
        }),
    };
    let workspace_git = |repositories| {
        BootstrapRecord::WorkspaceGit(WorkspaceGitSnapshot {
            workspace_id: crate::WorkspaceId::from_u64(3),
            repositories,
        })
    };
    let wire = |record: &BootstrapRecord| {
        let encoded = super::BootstrapRecord::try_from(record).unwrap();
        let Some(bootstrap_record::Record::WorkspaceGit(git)) = encoded.record else {
            panic!("a Git record");
        };
        git
    };
    let decoded = |git: super::WorkspaceGitSnapshot| {
        decode_bootstrap_record(super::BootstrapRecord {
            record: Some(bootstrap_record::Record::WorkspaceGit(git)),
        })
    };

    let several = workspace_git(vec![repository("api", "main"), repository("web", "login")]);
    let git = wire(&several);
    // A 0.2 peer reads these: no branch, no History, and the `changes` it requires, empty.
    assert_eq!(git.branch, None);
    assert_eq!(git.head, None);
    assert_eq!(git.changes, Some(Default::default()));
    assert_eq!(git.repositories.len(), 2);
    assert_eq!(decoded(git).unwrap(), Some(several));

    // The Root's own repository rides the old fields alone, as an older Server sends it.
    let own = workspace_git(vec![repository("", "main")]);
    let git = wire(&own);
    assert_eq!(git.branch.as_deref(), Some("main"));
    assert!(git.repositories.is_empty());
    assert_eq!(decoded(git).unwrap(), Some(own));

    // Beside the ones under the Root, which a 0.2 peer leaves unread.
    let nested = workspace_git(vec![repository("", "main"), repository("sub", "login")]);
    let git = wire(&nested);
    assert_eq!(git.branch.as_deref(), Some("main"));
    assert_eq!(git.changes.as_ref().unwrap().entries.len(), 1);
    assert_eq!(git.repositories.len(), 1);
    assert_eq!(decoded(git).unwrap(), Some(nested));

    // A repository is one directory right under the Root.
    for directory in ["api/src", "..", ""] {
        let mut git = wire(&workspace_git(vec![repository("api", "main")]));
        git.repositories[0].directory = directory.into();
        assert!(decoded(git).is_err(), "{directory:?}");
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

#[test]
fn old_file_replies_round_trip() {
    for result in [
        Ok(crate::OldFile::Text("one\r\ntwo\r\n".into())),
        Ok(crate::OldFile::Text(String::new())),
        Ok(crate::OldFile::Added),
        Ok(crate::OldFile::None),
        Err("the Workspace has no base branch".into()),
    ] {
        let message = ServerMessage::GitOldFile {
            request_id: 8,
            workspace_id: crate::WorkspaceId::from_u64(3),
            path: "src/lib.rs".into(),
            result,
        };
        let encoded = super::ServerMessage::try_from(&message).unwrap();
        assert_eq!(ServerMessage::try_from(encoded).unwrap(), message);
    }
}

/// ADR 0051: every cell column round-trips: widths, underline styles, the other style
/// bits, the soft wrap, and default, indexed and RGB colours in all three colour columns.
#[test]
fn the_cell_columns_round_trip() {
    use crate::{
        CELL_FAINT, CELL_OVERLINE, CELL_SPACER_HEAD, CELL_SPACER_TAIL, CELL_UNDERLINE_CURLY,
        CELL_UNDERLINE_DASHED, CELL_WIDE, CELL_WRAPPED, TerminalCell, TerminalColor, TerminalSize,
    };
    let rgb = TerminalColor::Rgb {
        red: 1,
        green: 2,
        blue: 3,
    };
    let cell = |text: &str, flags, underline_color| TerminalCell {
        text: text.into(),
        foreground: TerminalColor::Indexed(196),
        background: rgb,
        underline_color,
        flags,
        hyperlink: None,
    };
    let view = TerminalView {
        revision: 7,
        size: TerminalSize::new(1, 4),
        display_offset: 0,
        history_rows: 0,
        search: Default::default(),
        mouse_tracking: TerminalMouseTracking::None,
        cells: vec![
            cell("中", CELL_WIDE | CELL_UNDERLINE_CURLY, rgb),
            cell("", CELL_SPACER_TAIL, TerminalColor::Default),
            cell(
                "a",
                CELL_FAINT | CELL_OVERLINE | CELL_UNDERLINE_DASHED,
                TerminalColor::Indexed(4),
            ),
            cell("", CELL_SPACER_HEAD | CELL_WRAPPED, TerminalColor::Default),
        ],
        cursor: None,
        selection: None,
        images: Vec::new(),
    };
    let encoded = super::TerminalView::from(&view);
    assert_eq!(TerminalView::try_from(encoded).unwrap(), view);
}

/// A progress state from a newer peer reads as indeterminate (ADR 0054).
#[test]
fn an_unknown_progress_state_reads_as_indeterminate() {
    let event = super::SessionEvent {
        event: Some(session_event::Event::TerminalProgressChanged(
            super::TerminalProgressChanged {
                pane_id: 3,
                progress: Some(super::TerminalProgress {
                    state: 99,
                    percent: Some(40),
                }),
            },
        )),
    };
    let SessionEvent::TerminalProgressChanged {
        progress: Some(progress),
        ..
    } = SessionEvent::try_from(event).unwrap()
    else {
        unreachable!()
    };
    assert_eq!(progress.state, crate::TerminalProgressState::Indeterminate);
    assert_eq!(progress.percent, Some(40));
}

mod streams {
    use crate::WorkspaceId;
    use crate::protocol::{
        ClientHandshake, Hello, MAX_STREAM_FRAME_SIZE, StreamClientMessage, StreamError,
        StreamFile, StreamMessage, StreamServerMessage, StreamTarget, encode_frame,
        read_message_with_limit,
    };
    use prost::Message as _;

    fn round_trip<M: crate::protocol::WireMessage + PartialEq + std::fmt::Debug>(message: &M) -> M {
        let frame = encode_frame(message, MAX_STREAM_FRAME_SIZE).unwrap();
        read_message_with_limit(&mut frame.as_slice(), MAX_STREAM_FRAME_SIZE)
            .unwrap()
            .0
    }

    fn file() -> StreamFile {
        StreamFile {
            workspace: WorkspaceId::from_u64(7),
            path: "src/main.rs".into(),
        }
    }

    #[test]
    fn every_stream_message_survives_a_round_trip_in_its_direction() {
        let client = [
            StreamMessage::Open {
                stream: 1,
                target: StreamTarget::Forward { port: 5173 },
            },
            StreamMessage::Open {
                stream: 2,
                target: StreamTarget::Download(file()),
            },
            StreamMessage::Open {
                stream: 3,
                target: StreamTarget::Upload {
                    file: file(),
                    replace: true,
                },
            },
            StreamMessage::Data {
                stream: 1,
                data: vec![1, 2, 3],
            },
            StreamMessage::Credit {
                stream: 1,
                max_offset: 1 << 21,
            },
            StreamMessage::Finish { stream: 1 },
            StreamMessage::Reset {
                stream: 2,
                error: StreamError::Cancelled,
                detail: "by hand".into(),
            },
            StreamMessage::Ping,
            StreamMessage::Close,
        ];
        for message in client {
            let message = StreamClientMessage(message);
            assert_eq!(round_trip(&message), message);
        }
        let server = [
            StreamMessage::Opened {
                stream: 2,
                size: Some(42),
            },
            StreamMessage::Opened {
                stream: 1,
                size: None,
            },
            StreamMessage::OpenFailed {
                stream: 3,
                error: StreamError::AlreadyExists,
                detail: String::new(),
            },
            StreamMessage::Data {
                stream: 1,
                data: Vec::new(),
            },
            StreamMessage::Pong,
            StreamMessage::Close,
        ];
        for message in server {
            let message = StreamServerMessage(message);
            assert_eq!(round_trip(&message), message);
        }
    }

    #[test]
    fn a_member_of_the_other_direction_is_refused_when_encoded() {
        let opened = StreamClientMessage(StreamMessage::Opened {
            stream: 1,
            size: None,
        });
        assert!(encode_frame(&opened, MAX_STREAM_FRAME_SIZE).is_err());
        let open = StreamServerMessage(StreamMessage::Open {
            stream: 1,
            target: StreamTarget::Forward { port: 1 },
        });
        assert!(encode_frame(&open, MAX_STREAM_FRAME_SIZE).is_err());
    }

    #[test]
    fn unknown_input_is_kept_apart_rather_than_refused() {
        // An unset top-level member is one a newer peer added.
        let decoded =
            StreamClientMessage::try_from(super::super::StreamClientMessage { message: None })
                .unwrap();
        assert_eq!(decoded.0, StreamMessage::Unknown);
        let decoded =
            StreamServerMessage::try_from(super::super::StreamServerMessage { message: None })
                .unwrap();
        assert_eq!(decoded.0, StreamMessage::Unknown);
        // An `Open` whose target this build does not know is answered, not dropped.
        let open = super::super::StreamClientMessage {
            message: Some(super::super::stream_client_message::Message::Open(
                super::super::StreamOpen {
                    stream: 4,
                    target: None,
                },
            )),
        };
        assert_eq!(
            StreamClientMessage::try_from(open).unwrap().0,
            StreamMessage::Open {
                stream: 4,
                target: StreamTarget::Unknown,
            }
        );
        // An error code from a newer peer reads as `Other`.
        let reset = super::super::StreamServerMessage {
            message: Some(super::super::stream_server_message::Message::Reset(
                super::super::StreamReset {
                    stream: 1,
                    error: 999,
                    detail: String::new(),
                },
            )),
        };
        let StreamMessage::Reset { error, .. } = StreamServerMessage::try_from(reset).unwrap().0
        else {
            panic!("a reset decodes as one");
        };
        assert_eq!(error, StreamError::Other);
    }

    #[test]
    fn the_streams_handshake_round_trips_and_an_unknown_kind_stays_unknown() {
        let handshake = ClientHandshake::Streams(Hello::new("gui"));
        let frame = encode_frame(&handshake, MAX_STREAM_FRAME_SIZE).unwrap();
        let decoded: ClientHandshake = read_message_with_limit(&mut frame.as_slice(), 1024)
            .unwrap()
            .0;
        assert_eq!(decoded, handshake);
        // What a Server from before this kind sees: an unset `oneof`, which it refuses.
        let empty = super::super::ClientHandshake { kind: None }.encode_to_vec();
        assert_eq!(
            ClientHandshake::try_from(
                super::super::ClientHandshake::decode(empty.as_slice()).unwrap()
            ),
            Err(super::WireError::Unknown)
        );
    }
}
