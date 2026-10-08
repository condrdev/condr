use super::*;
use condr_core::AgentKind;
use condr_core::agent_hooks::{HooksAction, HooksState};
use condr_core::protocol::{AgentCommand, AgentResponse};

/// Only this child sees the fixture agent directory; parallel tests and the real
/// user's configuration must never observe a changed process environment.
fn isolated_hooks_case(case: &str) {
    let root = std::env::temp_dir().join(format!(
        "condr-server-hooks-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "server::tests::hooks::hooks_server_child",
            "--nocapture",
        ])
        .env("CONDR_TEST_SERVER_HOOK_CASE", case)
        .env("CLAUDE_CONFIG_DIR", &root)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(root);
    assert!(
        output.status.success(),
        "Hooks case {case} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn invalid_or_stopping_hooks_requests_do_not_touch_agent_configuration() {
    isolated_hooks_case("rejected");
}

#[test]
fn shutdown_waits_for_accepted_hooks_and_reports_the_completed_installation() {
    isolated_hooks_case("accepted");
}

#[test]
fn accepted_hooks_keep_their_real_failure_when_shutdown_starts() {
    isolated_hooks_case("failed");
}

fn install(stream: &mut EndpointStream, server_id: ServerId, session_id: SessionId) {
    condr_core::protocol::write_message(
        stream,
        &ClientMessage::Agent {
            server_id,
            session_id,
            command: AgentCommand::Hooks {
                agent: AgentKind::Claude,
                action: HooksAction::Install,
            },
        },
    )
    .unwrap();
}

#[test]
#[ignore = "spawned by the isolated Hooks request tests"]
fn hooks_server_child() {
    let case = std::env::var("CONDR_TEST_SERVER_HOOK_CASE").unwrap();
    let root = PathBuf::from(std::env::var_os("CLAUDE_CONFIG_DIR").unwrap());
    let (handle, endpoint, server_thread) = start();
    let connection = ClientConnection::connect(&endpoint, "hooks-test").unwrap();
    let bootstrap = connection.bootstrap().unwrap();
    let server_id = bootstrap.server_id;
    let session_id = bootstrap.session_id;
    let mut stream = connection.into_stream();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    if case == "rejected" {
        for (server, session) in [
            (ServerId(server_id.0.wrapping_add(1)), session_id),
            (server_id, SessionId(session_id.0.wrapping_add(1))),
        ] {
            install(&mut stream, server, session);
            assert!(
                matches!(read_server(&mut stream), ServerMessage::AgentResult {
                result: Err(error),
            } if error.code == "invalid_agent_request")
            );
            assert!(
                !root.exists(),
                "a rejected request touched the agent configuration"
            );
        }
        handle.lifecycle.begin_stop();
        install(&mut stream, server_id, session_id);
        assert!(
            matches!(read_server(&mut stream), ServerMessage::AgentResult {
            result: Err(error),
        } if error.code == "invalid_agent_request")
        );
        assert!(!root.exists(), "a stopping Server still installed hooks");
        handle.stop();
    } else {
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        std::fs::write(&path, r#"{"userSetting":true}"#).unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("settings.json.condr-lock"))
            .unwrap();
        lock.lock().unwrap();
        let mut observer = ClientConnection::connect(&endpoint, "hooks-observer")
            .unwrap()
            .into_stream();
        observer
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        install(&mut stream, server_id, session_id);
        let deadline = Instant::now() + Duration::from_secs(5);
        while handle.lifecycle.state.lock().unwrap().operations == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(2));
        }
        assert_ne!(
            handle.lifecycle.state.lock().unwrap().operations,
            0,
            "the blocked Hooks operation did not participate in Server shutdown"
        );
        condr_core::protocol::write_message(
            &mut observer,
            &ClientMessage::Ping {
                server_id,
                nonce: 42,
            },
        )
        .unwrap();
        assert!(
            matches!(
                read_server(&mut observer),
                ServerMessage::Pong { nonce: 42, .. }
            ),
            "a Hooks operation blocked unrelated clients on the Session lock"
        );
        handle.stop();
        thread::sleep(Duration::from_millis(150));
        assert!(
            !server_thread.is_finished(),
            "Server stopped before its accepted Hooks operation"
        );
        if case == "failed" {
            std::fs::write(&path, "{ invalid JSON").unwrap();
        }
        drop(lock);
        let result = read_server(&mut stream);
        if case == "failed" {
            assert!(
                matches!(result, ServerMessage::AgentResult { result: Err(error) }
                if error.code == "hooks_config_invalid")
            );
            assert_eq!(std::fs::read_to_string(path).unwrap(), "{ invalid JSON");
        } else {
            assert!(matches!(result, ServerMessage::AgentResult {
                result: Ok(AgentResponse::Hooks(report))
            } if report.state == HooksState::Installed));
            let config: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            assert_eq!(config["userSetting"], true);
            assert!(config["hooks"]["Stop"].is_array());
        }
    }
    drop(stream);
    server_thread.join().unwrap().unwrap();
}
