//! The Companion's library against a real Server over TCP (ADR 0039).

use condr_core::TerminalCommand;
use condr_core::protocol::{LayoutCommand, LayoutResult};
use condr_mobile::{Change, Companion, DeviceStatus, Listener, SavedDevice};
use condr_server::{BoundServer, ClientConnection, Endpoint, ServerConfig};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Counter(AtomicUsize);

impl Listener for Counter {
    fn changed(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The app does not take the changes while a shell echoes a burst of input: the library
/// calls `changed` once for all of it, and the terminal it reads is the latest revision.
#[test]
fn a_burst_of_frames_costs_one_changed_until_the_app_takes_it() {
    let server =
        BoundServer::bind(ServerConfig::ephemeral_tcp("127.0.0.1:0".parse().unwrap()).unwrap())
            .unwrap();
    let handle = server.handle();
    let endpoint = server.endpoint().clone();
    let Endpoint::Tcp(tcp) = &endpoint else {
        panic!("an ephemeral TCP Server answers over TCP");
    };
    let address = format!("tcp://{}@{}", tcp.server_key, tcp.authority());
    let seed = tcp.client_key.seed().to_vec();
    let server_thread = std::thread::spawn(move || server.run());

    let mut desktop = ClientConnection::connect_overview(&endpoint, "desktop").unwrap();
    let Ok(LayoutResult::WorkspaceCreated { pane_id, .. }) = desktop
        .layout(LayoutCommand::CreateWorkspace {
            root_directory: std::env::temp_dir(),
            name: None,
        })
        .unwrap()
    else {
        panic!("the Workspace is created");
    };
    let pane = pane_id.as_u64();

    let counter = Arc::new(Counter::default());
    let companion = Companion::new(
        seed,
        "phone".into(),
        vec![SavedDevice {
            address: address.clone(),
            label: "desk".into(),
        }],
        counter.clone(),
    )
    .unwrap();
    companion.resume();
    wait_until("the connection and the Pane's terminal", || {
        companion.devices()[0].status == DeviceStatus::Connected
            && companion.terminal(address.clone(), pane).is_some()
    });
    assert_eq!(companion.workspaces(address.clone()).len(), 1);

    companion.take_changes();
    let before = counter.0.load(Ordering::SeqCst);
    let revision = companion.terminal(address.clone(), pane).unwrap().revision;
    for _ in 0..20 {
        desktop
            .terminal(pane_id, TerminalCommand::Text("x".into()))
            .unwrap();
        std::thread::sleep(Duration::from_millis(15));
    }
    wait_until("the echoed burst", || {
        companion
            .terminal(address.clone(), pane)
            .is_some_and(|screen| screen.revision >= revision + 5)
    });
    assert_eq!(
        counter.0.load(Ordering::SeqCst),
        before + 1,
        "one `changed` for the whole burst"
    );
    let changes = companion.take_changes();
    assert!(
        changes.contains(&Change::Terminal {
            address: address.clone(),
            pane
        }),
        "{changes:?}"
    );

    desktop
        .terminal(pane_id, TerminalCommand::Text("y".into()))
        .unwrap();
    wait_until("the next change after the take", || {
        counter.0.load(Ordering::SeqCst) > before + 1
    });

    companion.close();
    handle.stop();
    drop(desktop);
    server_thread.join().unwrap().unwrap();
}
