use super::*;

/// Creates a Workspace and returns its first Pane once the shell has drawn a prompt.
#[cfg(target_os = "linux")]
fn workspace_pane(
    stream: &mut EndpointStream,
    handle: &ServerHandle,
    views: &mut std::collections::HashMap<PaneId, condr_core::TerminalView>,
) -> PaneId {
    let server_id = handle.server_id();
    let session_id = handle.state.lock().unwrap().session_id;
    condr_core::protocol::write_message(
        stream,
        &ClientMessage::Layout {
            server_id,
            session_id,
            request_id: 1,
            command: LayoutCommand::CreateWorkspace {
                name: None,
                focus: true,
                root_directory: std::env::temp_dir(),
            },
        },
    )
    .unwrap();
    wait_for_message(stream, |message| {
        matches!(
            message,
            ServerMessage::Event {
                event: SessionEvent::LayoutChanged { .. },
                ..
            }
        )
    });
    let pane_id = handle
        .state
        .lock()
        .unwrap()
        .session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .unwrap()
        .id();
    // Leave room for the shell's echoed input markers.
    send_terminal(
        stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Resize(TerminalSize::new(8, 200)),
    );
    wait_for_terminal(stream, views, pane_id, |view| view.size.columns == 200);
    send_terminal(
        stream,
        server_id,
        session_id,
        pane_id,
        TerminalCommand::Text("printf 'pane-ready\\n'\r".into()),
    );
    wait_for_terminal_text(stream, views, pane_id, "pane-ready");
    pane_id
}

#[cfg(target_os = "linux")]
fn paste_image(
    stream: &mut EndpointStream,
    server_id: ServerId,
    session_id: SessionId,
    pane_id: PaneId,
    bytes: Vec<u8>,
) {
    condr_core::protocol::write_client_message(
        stream,
        &ClientMessage::PasteImage {
            server_id,
            session_id,
            pane_id,
            format: condr_core::protocol::ClipboardImageFormat::Png,
            bytes,
        },
    )
    .unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn a_pasted_image_is_staged_privately_pasted_as_a_path_and_removed_with_its_client() {
    let (handle, endpoint, thread) = start();
    let mut stream = connect_and_bootstrap(&endpoint);
    let server_id = handle.server_id();
    let session_id = handle.state.lock().unwrap().session_id;
    acquire_control(&mut stream, session_id);
    subscribe(&mut stream, session_id, 0);
    let mut views = std::collections::HashMap::new();
    let pane_id = workspace_pane(&mut stream, &handle, &mut views);

    let png = b"\x89PNG\r\n\x1a\ncondr".to_vec();
    paste_image(&mut stream, server_id, session_id, pane_id, png.clone());
    // The shell echoes the pasted path; the file behind it holds the image.
    wait_for_terminal(&mut stream, &mut views, pane_id, |view| {
        let text = view_text(view);
        text.contains("client-") && text.contains(".png")
    });
    // The Server owns the path; shell echo loses whitespace and wide-cell fidelity.
    let path = handle
        .state
        .lock()
        .unwrap()
        .staged_images
        .values()
        .flatten()
        .next()
        .cloned()
        .expect("image staged for the Client");
    assert!(path.starts_with(crate::server::clipboard_image::staging_directory().unwrap()));
    assert_eq!(std::fs::read(&path).unwrap(), png);
    {
        let state = handle.state.lock().unwrap();
        assert_eq!(state.staged_images.values().flatten().count(), 1);
        assert!(
            !format!("{:?}", state.session.snapshot()).contains("client-"),
            "staged paths are not Session state"
        );
    }
    // A non-controlling Client can paste, but only to valid identities and live Panes.
    let mut uploader = connect_and_bootstrap(&endpoint);
    paste_image(
        &mut uploader,
        ServerId(server_id.0 + 1),
        session_id,
        pane_id,
        png.clone(),
    );
    assert!(matches!(
        read_server(&mut uploader),
        ServerMessage::Error { message } if message == "unknown Server"
    ));
    paste_image(
        &mut uploader,
        server_id,
        session_id,
        PaneId::from_u64(9999),
        png.clone(),
    );
    assert!(matches!(
        read_server(&mut uploader),
        ServerMessage::Error { message } if message == "unknown Pane"
    ));
    paste_image(
        &mut uploader,
        server_id,
        SessionId(session_id.0 + 1),
        pane_id,
        png.clone(),
    );
    assert!(matches!(
        read_server(&mut uploader),
        ServerMessage::Error { message } if message == "unknown Session"
    ));
    paste_image(
        &mut uploader,
        server_id,
        session_id,
        pane_id,
        vec![0; condr_core::protocol::MAX_CLIPBOARD_IMAGE_BYTES + 1],
    );
    assert!(matches!(
        read_server(&mut uploader),
        ServerMessage::Error { message } if message.contains("16 MiB")
    ));
    handle
        .state
        .lock()
        .unwrap()
        .closing_terminals
        .insert(pane_id);
    paste_image(&mut uploader, server_id, session_id, pane_id, png.clone());
    assert!(matches!(
        read_server(&mut uploader),
        ServerMessage::Error { message } if message == "terminal is closing"
    ));
    {
        let mut state = handle.state.lock().unwrap();
        state.closing_terminals.remove(&pane_id);
        state.exited_terminals.insert(pane_id);
    }
    paste_image(&mut uploader, server_id, session_id, pane_id, png.clone());
    assert!(matches!(
        read_server(&mut uploader),
        ServerMessage::Error { message } if message == "terminal has exited"
    ));
    {
        let mut state = handle.state.lock().unwrap();
        state.exited_terminals.remove(&pane_id);
        assert_eq!(state.staged_images.values().flatten().count(), 1);
    }

    paste_image(&mut uploader, server_id, session_id, pane_id, png);
    condr_core::protocol::write_message(
        &mut uploader,
        &ClientMessage::Ping {
            server_id,
            nonce: 29,
        },
    )
    .unwrap();
    assert!(matches!(
        read_server(&mut uploader),
        ServerMessage::Pong { nonce: 29, .. }
    ));
    let other_path = handle
        .state
        .lock()
        .unwrap()
        .staged_images
        .values()
        .flatten()
        .find(|staged| **staged != path)
        .unwrap()
        .clone();
    drop(uploader);
    let deadline = Instant::now() + Duration::from_secs(5);
    while other_path.exists() {
        assert!(
            Instant::now() < deadline,
            "image outlived the uploading Client"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert!(path.exists(), "another Client's image must remain");

    // Any other message that size is refused as a bad frame and ends the connection.
    let mut oversized = connect_and_bootstrap(&endpoint);
    condr_core::protocol::write_message_with_limit(
        &mut oversized,
        &ClientMessage::Terminal {
            server_id,
            session_id,
            pane_id,
            command: TerminalCommand::Text("x".repeat(3 * 1024 * 1024)),
        },
        condr_core::protocol::MAX_IMAGE_FRAME_SIZE,
    )
    .unwrap();
    assert!(matches!(
        read_server(&mut oversized),
        ServerMessage::Error { message } if message.contains("invalid client frame")
    ));

    // A PTY writer can fail after validation. Keep the Pane routable but close its
    // runtime, so this exercises staging followed by an actual failed paste.
    let client_id = {
        let mut state = handle.state.lock().unwrap();
        state.terminal_instances.remove(&pane_id);
        state.terminals.get_mut(&pane_id).unwrap().close().unwrap();
        *state.staged_images.keys().next().unwrap()
    };
    paste_image(&mut stream, server_id, session_id, pane_id, vec![1]);
    loop {
        if let ServerMessage::Error { message } = read_server(&mut stream) {
            assert_eq!(message, "terminal writer stopped");
            break;
        }
    }
    let prefix = format!("client-{}-{client_id}-", std::process::id());
    let remaining: Vec<_> =
        std::fs::read_dir(crate::server::clipboard_image::staging_directory().unwrap())
            .unwrap()
            .map(Result::unwrap)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
            .map(|entry| entry.path())
            .collect();
    assert_eq!(
        remaining,
        vec![path.clone()],
        "failed routing leaked an image"
    );

    // The uploading Client leaving takes its file with it.
    drop(stream);
    let deadline = Instant::now() + Duration::from_secs(5);
    while path.exists() {
        assert!(
            Instant::now() < deadline,
            "staged image outlived its Client"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert!(handle.state.lock().unwrap().staged_images.is_empty());
    handle.stop();
    thread.join().unwrap().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn a_large_image_crosses_the_noise_tcp_transport_and_dies_with_the_server() {
    let server =
        BoundServer::bind(ServerConfig::ephemeral_tcp("127.0.0.1:0".parse().unwrap()).unwrap())
            .unwrap();
    let handle = server.handle();
    let endpoint = server.endpoint().clone();
    let thread = thread::spawn(move || server.run());

    let connection = ClientConnection::connect(&endpoint, "tcp-image").unwrap();
    let bootstrap = connection.bootstrap().unwrap().clone();
    let server_id = bootstrap.server_id;
    let session_id = bootstrap.session_id;
    let mut stream = connection.into_stream();
    stream
        .set_handshake_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    acquire_control(&mut stream, session_id);
    subscribe(&mut stream, session_id, bootstrap.sequence);
    let mut views = std::collections::HashMap::new();
    let pane_id = workspace_pane(&mut stream, &handle, &mut views);

    // Well past the ordinary 2 MiB frame: the framed message spans many Noise records.
    let mut image = vec![0x42; 5 * 1024 * 1024];
    image[..4].copy_from_slice(b"BIG!");
    paste_image(&mut stream, server_id, session_id, pane_id, image.clone());
    wait_for_terminal(&mut stream, &mut views, pane_id, |view| {
        let text = view_text(view);
        text.contains("client-") && text.contains(".png")
    });
    let path = handle
        .state
        .lock()
        .unwrap()
        .staged_images
        .values()
        .flatten()
        .next()
        .cloned()
        .expect("image staged for the Client");
    assert_eq!(std::fs::read(&path).unwrap(), image);

    // Stopping the Server removes what its Clients staged.
    handle.stop();
    thread.join().unwrap().unwrap();
    assert!(!path.exists());
}
