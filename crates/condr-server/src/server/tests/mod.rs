#[cfg(target_os = "linux")]
mod clipboard_image;
mod control;
// Layout tests run only where CI exercises the Server's worktree and PTY paths.
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod layout;
mod persistence;
mod protocol;
mod shutdown;
mod subscriptions;
mod unit;

use super::*;
use crate::test_support::unique_suffix;

fn test_endpoint() -> Endpoint {
    Endpoint::local(std::env::temp_dir().join(format!(
        "condr-server-{}-{}.sock",
        std::process::id(),
        unique_suffix()
    )))
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn start() -> (ServerHandle, Endpoint, thread::JoinHandle<io::Result<()>>) {
    let endpoint = test_endpoint();
    let server =
        BoundServer::bind(ServerConfig::ephemeral(endpoint.as_local_path().unwrap())).unwrap();
    let handle = server.handle();
    let thread = thread::spawn(move || server.run());
    for _ in 0..100 {
        if endpoint.connect().is_ok() {
            return (handle, endpoint, thread);
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("server did not start");
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn write_snapshot_fixture(path: PathBuf, snapshot: condr_core::SessionSnapshot) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, snapshot.to_bytes().unwrap()).unwrap();
}

/// Mirrors the snapshot's Tab content enum, variant order included.
#[derive(serde::Serialize)]
enum EncodedTabContent {
    Terminals {
        panes: Vec<(PaneId, Option<PathBuf>, Option<condr_core::AgentResume>)>,
        focused_pane: PaneId,
        focus_history: Vec<PaneId>,
        layout: (u32, Vec<(u32, PaneId)>),
    },
    #[expect(dead_code, reason = "the Diff variant keeps the wire order honest")]
    Diff { path: PathBuf },
}

fn structurally_invalid_snapshot(root: PathBuf) -> Vec<u8> {
    let mut session = Session::new();
    let workspace_id = session
        .create_workspace(root.clone())
        .expect("Workspace capacity");
    let tab = session.active_workspace().unwrap().active_tab();
    let tab_id = tab.id();
    let pane_id = tab.focused_pane().unwrap().id();
    bincode::serialize(&(
        1u32,
        vec![(
            workspace_id,
            "invalid".to_string(),
            root.clone(),
            Option::<condr_core::WorktreeAssociation>::None,
            vec![(
                tab_id,
                String::new(),
                EncodedTabContent::Terminals {
                    panes: vec![(pane_id, Some(root), Option::<condr_core::AgentResume>::None)],
                    focused_pane: pane_id,
                    focus_history: Vec::<PaneId>::new(),
                    layout: (u32::MAX, vec![(0u32, pane_id)]),
                },
            )],
            tab_id,
        )],
        Some(workspace_id),
    ))
    .unwrap()
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn wait_for_connection(endpoint: &Endpoint) {
    for _ in 0..100 {
        if endpoint.connect().is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("server did not start");
}

fn connect_and_bootstrap(endpoint: &Endpoint) -> EndpointStream {
    ClientConnection::connect(endpoint, "test")
        .unwrap()
        .into_stream()
}

fn acquire_control(stream: &mut EndpointStream, session_id: SessionId) {
    condr_core::protocol::write_message(stream, &ClientMessage::AcquireControl { session_id })
        .unwrap();
    assert!(matches!(
        condr_core::protocol::read_message::<_, ServerMessage>(stream).unwrap(),
        ServerMessage::ControlGranted { .. }
    ));
}

fn subscribe(stream: &mut EndpointStream, session_id: SessionId, after_sequence: u64) {
    condr_core::protocol::write_message(
        stream,
        &ClientMessage::Subscribe {
            session_id,
            after_sequence,
        },
    )
    .unwrap();
    assert!(matches!(
        condr_core::protocol::read_message::<_, ServerMessage>(stream).unwrap(),
        ServerMessage::Subscribed { .. }
    ));
}

fn send_terminal(
    stream: &mut EndpointStream,
    server_id: ServerId,
    session_id: SessionId,
    pane_id: PaneId,
    command: TerminalCommand,
) {
    condr_core::protocol::write_message(
        stream,
        &ClientMessage::Terminal {
            server_id,
            session_id,
            pane_id,
            command,
        },
    )
    .unwrap();
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn wait_for_terminal_text(
    stream: &mut EndpointStream,
    views: &mut std::collections::HashMap<PaneId, condr_core::TerminalView>,
    pane_id: PaneId,
    needle: &str,
) -> condr_core::TerminalView {
    wait_for_terminal(stream, views, pane_id, |view| {
        view_text(view).contains(needle)
    })
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn wait_for_terminal(
    stream: &mut EndpointStream,
    views: &mut std::collections::HashMap<PaneId, condr_core::TerminalView>,
    pane_id: PaneId,
    predicate: impl Fn(&condr_core::TerminalView) -> bool,
) -> condr_core::TerminalView {
    loop {
        let message = read_server(stream);
        if let ServerMessage::Error { message } = &message {
            panic!("server returned an error: {message}");
        }
        let ServerMessage::TerminalFrame(batch) = message else {
            continue;
        };
        for pane in batch.panes {
            if let Some(view) = views.get_mut(&pane.pane_id) {
                view.apply_frame(pane.frame).unwrap();
            } else if let condr_core::TerminalViewFrame::Full(view) = pane.frame {
                views.insert(pane.pane_id, view);
            } else {
                panic!("first terminal frame for a Pane must be full");
            }
            let view = &views[&pane.pane_id];
            if pane.pane_id == pane_id && predicate(view) {
                return view.clone();
            }
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn wait_for_message(
    stream: &mut EndpointStream,
    predicate: impl Fn(&ServerMessage) -> bool,
) -> ServerMessage {
    loop {
        let message = read_server(stream);
        if let ServerMessage::Error { message } = &message {
            panic!("server returned an error: {message}");
        }
        if predicate(&message) {
            return message;
        }
    }
}

fn read_server(stream: &mut EndpointStream) -> ServerMessage {
    condr_core::protocol::read_message(stream).unwrap()
}

fn assert_layout_applied(
    stream: &mut EndpointStream,
    server_id: ServerId,
    session_id: SessionId,
    request_id: u64,
    sequence: u64,
) {
    assert!(matches!(
        read_server(stream),
        ServerMessage::LayoutApplied {
            server_id: applied_server,
            session_id: applied_session,
            request_id: applied_request,
            sequence: applied_sequence,
            ..
        } if applied_server == server_id
            && applied_session == session_id
            && applied_request == request_id
            && applied_sequence == sequence
    ));
}

fn view_text(view: &condr_core::TerminalView) -> String {
    (0..view.size.rows)
        .map(|row| {
            (0..view.size.columns)
                .filter_map(|column| view.cell(row, column))
                .map(|cell| cell.text.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn marker_value(text: &str, marker: &str) -> String {
    text.split(marker)
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .expect("terminal marker has a value")
        .to_owned()
}
