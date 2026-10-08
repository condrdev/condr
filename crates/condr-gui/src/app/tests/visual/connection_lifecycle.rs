use super::*;

#[test]
fn status_refresh_during_reconnect_does_not_discard_the_new_connection() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.start_connect(1);
            // Settings performs this poll while its selected Device is reconnecting.
            this.server_admin(1, condr_core::protocol::ServerAdminCommand::Status);
            let connection = this.connection(1).unwrap();
            assert!(connection.status == ConnectionStatus::Connecting);
            assert!(connection.io.is_none());
        });
    });
    assert!(
        wait_until(window, |window| {
            window.read(|app| {
                view.read(app)
                    .connection(1)
                    .is_some_and(ServerConnection::can_mutate)
            })
        }),
        "the replacement connection must be installed and acquire control"
    );
}

#[test]
fn failed_enqueue_does_not_register_layout_or_resource_work() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            let connection = this.connection_mut(1).unwrap();
            let (outgoing, receiver) = std::sync::mpsc::channel();
            drop(receiver);
            connection.io.as_mut().unwrap().outgoing = outgoing;
            this.request_directory(1, WorkspaceId::from_u64(1), "src".into());
            assert!(
                this.connection(1)
                    .unwrap()
                    .resources
                    .pending_directories
                    .is_empty()
            );
            assert!(
                this.send_layout_to(
                    1,
                    LayoutCommand::CreateWorkspace {
                        root_directory: std::env::temp_dir(),
                        name: None
                    }
                )
                .is_none()
            );
            let connection = this.connection_mut(1).unwrap();
            connection.subscription_pending = false;
            connection.subscribe();
            assert!(!connection.subscription_pending);
            assert!(!connection.request_snapshot());
            assert!(connection.bootstrap_resync_session_id.is_none());
            assert!(connection.status == ConnectionStatus::Connected);
            assert!(connection.io.is_some());
        })
    });
}

#[test]
fn disconnect_retires_bootstrap_and_control_messages_already_in_the_batch() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let connection = this.connection(1).unwrap();
            let generation = connection.connect_generation;
            let server_id = connection.server_id.unwrap();
            let session_id = connection.session_id.unwrap();
            let bootstrap = SessionBootstrap {
                settings: connection.settings.clone(),
                server_id,
                runtime_epoch: connection.runtime_epoch.unwrap(),
                session_id,
                sequence: connection.sequence,
                snapshot: connection.session().unwrap().snapshot(),
                terminals: Vec::new(),
                agents: Vec::new(),
                workspace_git: Vec::new(),
                zoomed_panes: Vec::new(),
            };
            this.mark_disconnected(1, 0, "writer closed".into());
            this.handle_incoming(1, generation, Incoming::Bootstrap(bootstrap), cx);
            this.handle_incoming(
                1,
                generation,
                Incoming::Message(ServerMessage::ControlGranted {
                    server_id,
                    session_id,
                }),
                cx,
            );
            let connection = this.connection(1).unwrap();
            assert!(connection.status == ConnectionStatus::Disconnected);
            assert!(!connection.controlling);
            assert!(connection.io.is_none());
            assert_eq!(connection.error.as_deref(), Some("writer closed"));
        })
    });
}
