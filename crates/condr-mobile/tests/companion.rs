//! The Companion's library against a real Server over TCP (ADR 0039).

use condr_core::protocol::{LayoutCommand, LayoutResult};
use condr_core::{TerminalCommand, TerminalKey, TerminalKeyEventKind, TerminalModifiers};
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
    wait_until("the connection", || {
        companion.devices()[0].status == DeviceStatus::Connected
    });
    // Only the Pane on screen gets frames (ADR 0041).
    companion.show_pane(Some(address.clone()), pane, 0, 0);
    wait_until("the Pane's terminal", || {
        companion.terminal(address.clone(), pane).is_some()
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

/// Outside a Pane the phone watches no terminal (ADR 0041): Session events still arrive,
/// frames do not, though the shell behind the Pane echoes input.
#[test]
fn outside_a_pane_the_companion_gets_events_but_no_terminal() {
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
    let create = |desktop: &mut ClientConnection| {
        let Ok(LayoutResult::WorkspaceCreated { pane_id, .. }) = desktop
            .layout(LayoutCommand::CreateWorkspace {
                root_directory: std::env::temp_dir(),
                name: None,
            })
            .unwrap()
        else {
            panic!("the Workspace is created");
        };
        pane_id
    };
    let pane_id = create(&mut desktop);
    let pane = pane_id.as_u64();

    let companion = Companion::new(
        seed,
        "phone".into(),
        vec![SavedDevice {
            address: address.clone(),
            label: "desk".into(),
        }],
        Arc::new(Counter::default()),
    )
    .unwrap();
    companion.resume();
    wait_until("the connection", || {
        companion.devices()[0].status == DeviceStatus::Connected
            && companion.workspaces(address.clone()).len() == 1
    });

    desktop
        .terminal(pane_id, TerminalCommand::Text("echo hi".into()))
        .unwrap();
    let second = create(&mut desktop).as_u64();
    wait_until("the second Workspace's event", || {
        companion.workspaces(address.clone()).len() == 2
    });
    // Frames take the droppable slot, behind reliable events; give any a moment.
    std::thread::sleep(Duration::from_millis(300));
    assert!(companion.terminal(address.clone(), pane).is_none());
    assert!(companion.terminal(address.clone(), second).is_none());
    assert!(
        !companion
            .take_changes()
            .iter()
            .any(|change| matches!(change, Change::Terminal { .. })),
        "no terminal changed on the phone"
    );

    companion.close();
    handle.stop();
    drop(desktop);
    server_thread.join().unwrap().unwrap();
}

/// Scrolling is the Pane's, shared by every viewer (ADR 0036): the screen says how far up
/// the phone scrolled it, and "back to the newest" brings the offset to zero.
#[test]
fn scrolling_up_shows_in_the_screen_and_bottom_brings_it_back() {
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

    let companion = Companion::new(
        seed,
        "phone".into(),
        vec![SavedDevice {
            address: address.clone(),
            label: "desk".into(),
        }],
        Arc::new(Counter::default()),
    )
    .unwrap();
    companion.resume();
    wait_until("the connection", || {
        companion.devices()[0].status == DeviceStatus::Connected
    });
    companion.show_pane(Some(address.clone()), pane, 12, 40);
    wait_until("the Pane's terminal at the phone's size", || {
        companion
            .terminal(address.clone(), pane)
            .is_some_and(|screen| screen.rows == 12)
    });
    assert_eq!(
        companion
            .terminal(address.clone(), pane)
            .unwrap()
            .scrolled_lines,
        0
    );

    // Any shell prints a prompt per Enter, so forty of them leave history behind twelve rows.
    for _ in 0..40 {
        desktop
            .terminal(
                pane_id,
                TerminalCommand::Key {
                    key: TerminalKey::Enter,
                    modifiers: TerminalModifiers::default(),
                    kind: TerminalKeyEventKind::Press,
                },
            )
            .unwrap();
        std::thread::sleep(Duration::from_millis(10));
    }
    wait_until("the prompts to scroll the screen", || {
        companion.scroll(address.clone(), pane, 5);
        std::thread::sleep(Duration::from_millis(50));
        companion
            .terminal(address.clone(), pane)
            .is_some_and(|screen| screen.scrolled_lines >= 5)
    });

    companion.scroll_to_bottom(address.clone(), pane);
    wait_until("the viewport back at the bottom", || {
        companion
            .terminal(address.clone(), pane)
            .is_some_and(|screen| screen.scrolled_lines == 0)
    });

    companion.close();
    handle.stop();
    drop(desktop);
    server_thread.join().unwrap().unwrap();
}
