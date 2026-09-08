//! Client/Server contracts exercised entirely through public APIs.

use condr_core::Session;
use condr_core::protocol::{
    ClientMessage, Hello, LayoutCommand, LayoutResult, PROTOCOL_VERSION, ServerMessage,
};
use condr_server::{
    BoundServer, ClientConnection, Endpoint, EndpointStream, ServerConfig, ServerHandle,
};
use std::{io, sync::Arc, thread, time::Duration};

fn test_endpoint() -> Endpoint {
    Endpoint::local(std::env::temp_dir().join(format!(
        "condr-server-{}-{}.sock",
        std::process::id(),
        unique_suffix()
    )))
}

/// Random rather than clock-based: the Windows system clock ticks coarsely, so
/// parallel tests used to share an endpoint path and talk to each other's Server.
fn unique_suffix() -> u128 {
    uuid::Uuid::new_v4().as_u128()
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

fn connect_and_bootstrap(endpoint: &Endpoint) -> EndpointStream {
    ClientConnection::connect(endpoint, "test")
        .unwrap()
        .into_stream()
}

#[test]
fn concurrent_creations_return_their_own_ids_and_keep_the_current_selection() {
    let (handle, endpoint, server_thread) = start();
    let mut owner = ClientConnection::connect_overview(&endpoint, "owner").unwrap();
    let create = |name: &str, focus| LayoutCommand::CreateWorkspace {
        root_directory: std::env::temp_dir(),
        name: Some(name.into()),
        focus,
    };
    let LayoutResult::WorkspaceCreated {
        workspace_id: anchor,
        tab_id: anchor_tab,
        pane_id: anchor_pane,
    } = owner.layout(create("anchor", true)).unwrap().unwrap()
    else {
        panic!("Workspace result")
    };
    let first = ClientConnection::connect_overview(&endpoint, "first").unwrap();
    let second = ClientConnection::connect_overview(&endpoint, "second").unwrap();
    // The user's focus changes after both clients have read their initial structure.
    let LayoutResult::WorkspaceCreated {
        workspace_id: selected,
        ..
    } = owner.layout(create("selected", true)).unwrap().unwrap()
    else {
        panic!("Workspace result")
    };
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let workers = [first, second]
        .into_iter()
        .enumerate()
        .map(|(index, mut client)| {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                let name = format!("worker {index}");
                let result = client
                    .layout(LayoutCommand::CreateWorkspace {
                        root_directory: std::env::temp_dir(),
                        name: Some(name.clone()),
                        focus: false,
                    })
                    .unwrap()
                    .unwrap();
                let LayoutResult::WorkspaceCreated {
                    workspace_id,
                    tab_id,
                    pane_id,
                } = result
                else {
                    panic!("Workspace result")
                };
                let session = client.session().unwrap();
                assert_eq!(session.workspace(workspace_id).unwrap().name(), name);
                assert_eq!(session.tab(tab_id).unwrap().focused_pane().id(), pane_id);
                let LayoutResult::TabCreated { tab_id, pane_id } = client
                    .layout(LayoutCommand::CreateTab {
                        workspace_id: anchor,
                        name: Some(name.clone()),
                        focus: false,
                    })
                    .unwrap()
                    .unwrap()
                else {
                    panic!("Tab result")
                };
                assert_eq!(client.session().unwrap().tab(tab_id).unwrap().name(), name);
                assert_eq!(
                    client
                        .session()
                        .unwrap()
                        .tab(tab_id)
                        .unwrap()
                        .focused_pane()
                        .id(),
                    pane_id
                );
                let LayoutResult::PaneCreated { pane_id: split } = client
                    .layout(LayoutCommand::SplitPane {
                        pane_id: anchor_pane,
                        direction: condr_core::SplitDirection::Horizontal,
                        focus: false,
                    })
                    .unwrap()
                    .unwrap()
                else {
                    panic!("Pane result")
                };
                assert!(
                    client
                        .session()
                        .unwrap()
                        .tab(anchor_tab)
                        .unwrap()
                        .panes()
                        .iter()
                        .any(|pane| pane.id() == split)
                );
                (workspace_id, tab_id, split)
            })
        })
        .collect::<Vec<_>>();
    let results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_ne!(results[0].0, results[1].0);
    assert_ne!(results[0].1, results[1].1);
    assert_ne!(results[0].2, results[1].2);
    let session = Session::restore(handle.snapshot()).unwrap();
    assert_eq!(session.active_workspace_id(), Some(selected));
    assert_eq!(
        session.workspace(anchor).unwrap().active_tab().id(),
        anchor_tab
    );
    assert_eq!(
        session.tab(anchor_tab).unwrap().focused_pane().id(),
        anchor_pane
    );
    let before = session.snapshot();
    assert!(owner.layout(create(" ", false)).unwrap().is_err());
    assert_eq!(
        handle.snapshot(),
        before,
        "invalid name must not leave a Workspace behind"
    );
    drop(owner);
    handle.stop();
    server_thread.join().unwrap().unwrap();
}

#[test]
fn compatible_client_gets_bootstrap_and_reconnect_sees_same_epoch() {
    let (handle, endpoint, thread) = start();
    let first = connect_and_bootstrap(&endpoint);
    let first_message: ServerMessage = {
        let mut stream = endpoint.connect().unwrap();
        condr_core::protocol::write_message(
            &mut stream,
            &ClientMessage::Hello(Hello {
                version: PROTOCOL_VERSION,
                client_name: "first".into(),
            }),
        )
        .unwrap();
        condr_core::protocol::read_message(&mut stream).unwrap()
    };
    let (first_id, first_epoch) = match first_message {
        ServerMessage::Welcome {
            server_id,
            runtime_epoch,
            ..
        } => (server_id, runtime_epoch),
        other => panic!("unexpected message: {other:?}"),
    };
    drop(first);
    let mut second = endpoint.connect().unwrap();
    condr_core::protocol::write_message(
        &mut second,
        &ClientMessage::Hello(Hello {
            version: PROTOCOL_VERSION,
            client_name: "second".into(),
        }),
    )
    .unwrap();
    let welcome: ServerMessage = condr_core::protocol::read_message(&mut second).unwrap();
    assert!(matches!(
        welcome,
        ServerMessage::Welcome {
            server_id,
            runtime_epoch,
            error: None,
            ..
        } if server_id == first_id && runtime_epoch == first_epoch
    ));
    handle.stop();
    drop(second);
    thread.join().unwrap().unwrap();
}

#[test]
fn non_hello_first_frame_is_rejected_with_a_clear_error() {
    let (handle, endpoint, thread) = start();
    let mut stream = endpoint.connect().unwrap();
    condr_core::protocol::write_message(&mut stream, &ClientMessage::Detach).unwrap();

    assert!(matches!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
        ServerMessage::Welcome {
            error: Some(message),
            ..
        } if message == "expected Hello as first message"
    ));

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}

#[test]
fn incompatible_client_is_rejected() {
    let (handle, endpoint, thread) = start();
    let mut stream = endpoint.connect().unwrap();
    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::Hello(Hello {
            version: PROTOCOL_VERSION + 1,
            client_name: "old".into(),
        }),
    )
    .unwrap();
    let response: ServerMessage = condr_core::protocol::read_message(&mut stream).unwrap();
    assert!(matches!(
        response,
        ServerMessage::Welcome { error: Some(_), .. }
    ));
    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}

#[test]
fn oversized_client_frame_is_rejected_with_a_clear_error() {
    let (handle, endpoint, thread) = start();
    let mut stream = connect_and_bootstrap(&endpoint);
    let claimed = (condr_core::protocol::MAX_IMAGE_FRAME_SIZE as u32) + 1;
    std::io::Write::write_all(&mut stream, &claimed.to_le_bytes()).unwrap();

    let response: ServerMessage = condr_core::protocol::read_message(&mut stream).unwrap();
    assert!(matches!(
        response,
        ServerMessage::Error { message }
            if message.contains("exceeds maximum")
    ));

    handle.stop();
    drop(stream);
    thread.join().unwrap().unwrap();
}

#[test]
fn stop_message_ends_server_and_preserves_session_handle() {
    let (handle, endpoint, thread) = start();
    let mut stream = connect_and_bootstrap(&endpoint);
    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::StopServer {
            server_id: handle.server_id(),
        },
    )
    .unwrap();
    assert_eq!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
        ServerMessage::ServerStopping
    );
    drop(stream);
    thread.join().unwrap().unwrap();
    assert_eq!(handle.snapshot(), Session::new().snapshot());
}

#[test]
fn tcp_endpoint_uses_the_same_handshake_and_bootstrap() {
    let server =
        BoundServer::bind(ServerConfig::ephemeral_tcp("127.0.0.1:0".parse().unwrap()).unwrap())
            .unwrap();
    let handle = server.handle();
    let endpoint = server.endpoint().clone();
    let thread = thread::spawn(move || server.run());
    let mut stream = connect_and_bootstrap(&endpoint);
    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::StopServer {
            server_id: handle.server_id(),
        },
    )
    .unwrap();
    assert_eq!(
        condr_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
        ServerMessage::ServerStopping
    );
    drop(stream);
    thread.join().unwrap().unwrap();
}
