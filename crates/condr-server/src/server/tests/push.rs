//! Pushes to Condr Cloud (ADR 0047), against a stand-in that records each request.

use super::*;
use crate::noise::{Authority as _, AuthorizedClient, DeviceKey, ServerIdentity};
use crate::server::push::{PUSH_TIMEOUT, Push, push_body, start_sender};
use condr_core::protocol::Hello;
use condr_core::{AgentKind, AgentState, ProxyMode, ProxySetting};
use std::io::{BufRead as _, Read as _};

type Request = (String, serde_json::Value);

/// Condr Cloud on a local port: each request takes the next answer of `script`, 202 once
/// it runs out; `None` holds the request unanswered until the client gives up.
fn cloud(script: Vec<Option<u16>>) -> (String, mpsc::Receiver<Request>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let script = Arc::new(Mutex::new(script.into_iter()));
    let (requests, received) = mpsc::channel();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let (Ok(mut stream), requests, script) =
                (stream, requests.clone(), Arc::clone(&script))
            else {
                return;
            };
            thread::spawn(move || {
                let mut reader = io::BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut head = String::new();
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        if line == "\r\n" {
                            break;
                        }
                        head.push_str(&line);
                    }
                    let length = header(&head, "content-length").map_or(0, |n| n.parse().unwrap());
                    let mut body = vec![0; length];
                    reader.read_exact(&mut body).unwrap();
                    let answer = script.lock().unwrap().next().unwrap_or(Some(200));
                    let _ = requests.send((head, serde_json::from_slice(&body).unwrap()));
                    let Some(status) = answer else {
                        thread::sleep(Duration::from_secs(10));
                        return;
                    };
                    // Condr Cloud answers HTTP 200 and puts the outcome in `code`
                    let reply = format!("{{\"code\":\"{status}\"}}");
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{reply}",
                        reply.len()
                    );
                }
            });
        }
    });
    (url, received)
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

fn next(requests: &mpsc::Receiver<Request>) -> Request {
    requests
        .recv_timeout(Duration::from_secs(10))
        .expect("Condr Cloud got no push")
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap()
}

/// A local stand-in must not go through whatever proxy the environment names.
fn no_proxy() -> ProxySetting {
    ProxySetting {
        mode: ProxyMode::None,
        url: String::new(),
    }
}

fn push(pane: &str, title_loc_key: &'static str) -> Push {
    Push {
        to: vec![DeviceKey::from_seed([9; 32]).public().to_string()],
        title_loc_key,
        title_loc_args: ["Claude Code"],
        body: "site · mac".into(),
        pane: pane.into(),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn a_push_body_is_cut_to_1_kb_at_a_character_boundary() {
    assert_eq!(
        push_body([Some("Bash: ls"), Some("site"), None, Some(""), Some("mac")]),
        "Bash: ls · site · mac"
    );
    // 341 three-byte characters fill 1023 bytes; the 342nd would pass 1024.
    let question = "确".repeat(400);
    let body = push_body([Some(question.as_str()), Some("site")]);
    assert_eq!(body.len(), 1023);
    assert!(question.starts_with(&body));
    // One byte, then four-byte emoji: 1 + 255 * 4 = 1021.
    let rockets = format!("a{}", "🚀".repeat(300));
    assert_eq!(push_body([Some(rockets.as_str())]).len(), 1021);
}

#[test]
fn a_push_posts_signed_json_and_retries_a_5xx_once() {
    let (url, requests) = cloud(vec![Some(503), Some(503)]);
    let runtime = runtime();
    let identity = Arc::new(ServerIdentity::ephemeral().unwrap());
    let key = identity.device_key().unwrap();
    let queue = start_sender(
        runtime.handle(),
        url.clone(),
        identity,
        &no_proxy(),
        PUSH_TIMEOUT,
    )
    .unwrap();
    let issued_from = unix_now();
    queue.send(push("7", "push_blocked")).unwrap();
    let (head, body) = next(&requests);
    let issued_by = unix_now();
    assert!(
        head.starts_with("POST /api/push/send HTTP/1.1\r\n"),
        "{head}"
    );
    assert_eq!(header(&head, "api-version"), Some("1"));
    assert_eq!(header(&head, "content-type"), Some("application/json"));
    let token = header(&head, "authorization")
        .and_then(|value| value.strip_prefix("Condr "))
        .expect("a Condr token");
    assert!(
        (issued_from..=issued_by)
            .any(|issued_at| token == condr_client::cloud::token(&key, &url, issued_at)),
        "the token is this Server's, for this Condr Cloud, issued now"
    );
    assert_eq!(
        body,
        serde_json::json!({
            "to": [DeviceKey::from_seed([9; 32]).public().to_string()],
            "titleLocKey": "push_blocked",
            "titleLocArgs": ["Claude Code"],
            "body": "site · mac",
            "pane": "7",
        })
    );
    assert_eq!(next(&requests).1, body, "a 5xx is retried");
    queue.send(push("8", "push_finished")).unwrap();
    assert_eq!(next(&requests).1["pane"], "8", "only once");
}

#[test]
fn a_timed_out_blocked_behind_a_newer_done_is_not_retried() {
    let (url, requests) = cloud(vec![None]);
    let runtime = runtime();
    let queue = start_sender(
        runtime.handle(),
        url,
        Arc::new(ServerIdentity::ephemeral().unwrap()),
        &no_proxy(),
        Duration::from_secs(1),
    )
    .unwrap();
    queue.send(push("7", "push_blocked")).unwrap();
    assert_eq!(next(&requests).1["titleLocKey"], "push_blocked");
    // Arrives while the Blocked is still in flight.
    queue.send(push("7", "push_finished")).unwrap();
    assert_eq!(
        next(&requests).1["titleLocKey"],
        "push_finished",
        "the late Blocked was not retried"
    );
    queue.send(push("8", "push_finished")).unwrap();
    assert_eq!(next(&requests).1["pane"], "8");
}

/// Marks `devices` paired, each with whether it takes pushes.
fn authorize(directory: &std::path::Path, devices: &[(&DeviceKey, bool)]) {
    let clients = devices
        .iter()
        .map(|(key, push)| AuthorizedClient {
            key: key.public(),
            name: "phone".into(),
            paired_at: 1,
            last_seen: 1,
            push: *push,
        })
        .collect::<Vec<_>>();
    std::fs::write(
        directory.join("authorized-clients.json"),
        serde_json::to_string(&clients).unwrap(),
    )
    .unwrap();
}

/// The Server's own thread finds the change, with no tokio context, as the monitor's
/// probe thread does.
fn change(handle: &ServerHandle, pane_id: PaneId, states: &[AgentState]) {
    for &state in states {
        apply_agent_refresh(
            &mut handle.state.lock().unwrap(),
            pane_id,
            Some(AgentSnapshot {
                session_id: None,
                kind: AgentKind::Claude,
                state,
                detail: None,
            }),
        );
    }
}

#[test]
fn a_device_watching_the_pane_is_skipped_and_nobody_left_posts_nothing() {
    let directory = std::env::temp_dir().join(format!(
        "condr-server-pushes-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let identity = ServerIdentity::load_or_create(&directory).unwrap();
    let server_key = identity.public_key();
    let watcher = DeviceKey::generate().unwrap();
    let away = DeviceKey::generate().unwrap();
    let silent = DeviceKey::generate().unwrap();
    authorize(
        &directory,
        &[(&watcher, true), (&away, true), (&silent, false)],
    );
    let (url, requests) = cloud(Vec::new());
    let server = BoundServer::bind(
        ServerConfig::ephemeral(directory.join("condr.sock"))
            .with_listen("127.0.0.1:0".parse().unwrap())
            .with_identity(identity)
            .with_cloud_url(url),
    )
    .unwrap();
    let handle = server.handle();
    handle.state.lock().unwrap().settings.proxy = no_proxy();
    let address = server.local_addr().unwrap().unwrap();
    let thread = thread::spawn(move || server.run());
    let (watched, other, workspace) = {
        let mut state = handle.state.lock().unwrap();
        for _ in 0..2 {
            state
                .session
                .create_workspace(std::env::temp_dir())
                .expect("Workspace capacity");
        }
        let workspaces = state.session.workspaces();
        let pane = |index: usize| workspaces[index].tabs()[0].focused_pane().unwrap().id();
        let panes = (pane(0), pane(1), workspaces[0].name().to_owned());
        state.terminal_titles.insert(panes.0, "build".into());
        panes
    };

    let watching = ClientConnection::connect_watching(
        &Endpoint::tcp(TcpEndpoint::at(address, server_key, watcher.clone())),
        Hello {
            push: true,
            ..Hello::new("phone")
        },
        [watched].into(),
        crate::ConnectionCancellation::default(),
    )
    .unwrap();
    change(&handle, watched, &[AgentState::Working, AgentState::Done]);
    let (_, body) = next(&requests);
    assert_eq!(
        body,
        serde_json::json!({
            "to": [away.public().to_string()],
            "titleLocKey": "push_finished",
            "titleLocArgs": [AgentKind::Claude.label()],
            "body": format!("{workspace} · build · {}", crate::noise::device_name()),
            "pane": watched.as_u64().to_string(),
        })
    );

    // Only the watcher takes pushes now: its own Pane posts nothing, another Pane does.
    authorize(&directory, &[(&watcher, true), (&away, false)]);
    change(&handle, watched, &[AgentState::Working, AgentState::Done]);
    change(&handle, other, &[AgentState::Idle, AgentState::Blocked]);
    let (_, body) = next(&requests);
    assert_eq!(body["pane"], other.as_u64().to_string());
    assert_eq!(
        body["to"],
        serde_json::json!([watcher.public().to_string()])
    );
    assert_eq!(body["titleLocKey"], "push_blocked");

    drop(watching);
    handle.stop();
    thread.join().unwrap().unwrap();
    let _ = std::fs::remove_dir_all(directory);
}
