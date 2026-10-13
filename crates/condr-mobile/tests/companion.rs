//! The Companion's library against a real Server over TCP (ADR 0039).

use condr_core::protocol::{LayoutCommand, LayoutResult};
use condr_core::{
    PaneId, TerminalCommand, TerminalKey, TerminalKeyEventKind, TerminalModifiers, TerminalSize,
};
use condr_mobile::{
    Change, Companion, DeviceStatus, Event, Key, Listener, SavedDevice, TerminalScreen,
};
use condr_server::{BoundServer, ClientConnection, Endpoint, ServerConfig};
use std::io::Read as _;
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

/// Scrolling is the Pane's, shared by every viewer (ADR 0036): a swipe over a shell scrolls
/// it, the screen says how far, and "back to the newest" brings the offset to zero.
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
    // A swipe is wheel motion; a shell does not track the mouse, so it scrolls.
    wait_until("the prompts to scroll the screen", || {
        companion.wheel(address.clone(), pane, 4, 4, 5);
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

/// A phone the Server does not know is told so, not "can't connect" (ADR 0040): the app
/// offers pairing again instead of a retry.
#[test]
fn a_server_that_does_not_know_the_phone_marks_it_revoked() {
    let server =
        BoundServer::bind(ServerConfig::ephemeral_tcp("127.0.0.1:0".parse().unwrap()).unwrap())
            .unwrap();
    let handle = server.handle();
    let endpoint = server.endpoint().clone();
    let Endpoint::Tcp(tcp) = &endpoint else {
        panic!("an ephemeral TCP Server answers over TCP");
    };
    let address = format!("tcp://{}@{}", tcp.server_key, tcp.authority());
    let server_thread = std::thread::spawn(move || server.run());

    let stranger = condr_mobile::generate_seed().unwrap();
    let companion = Companion::new(
        stranger,
        "phone".into(),
        vec![SavedDevice {
            address: address.clone(),
            label: "desk".into(),
        }],
        Arc::new(Counter::default()),
    )
    .unwrap();
    companion.resume();
    wait_until("the refusal", || {
        matches!(
            companion.devices()[0].status,
            DeviceStatus::Revoked | DeviceStatus::Failed { .. }
        )
    });
    assert_eq!(companion.devices()[0].status, DeviceStatus::Revoked);

    companion.close();
    handle.stop();
    server_thread.join().unwrap().unwrap();
}

/// One Server with one shell Pane, and a phone showing it at `rows` × `columns`.
struct Shown {
    companion: Arc<Companion>,
    address: String,
    pane: u64,
    endpoint: Endpoint,
    handle: condr_server::ServerHandle,
    thread: std::thread::JoinHandle<std::io::Result<()>>,
}

impl Shown {
    fn start(rows: u16, columns: u16) -> Self {
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
        let thread = std::thread::spawn(move || server.run());
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
        drop(desktop);
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
        let pane = pane_id.as_u64();
        companion.show_pane(Some(address.clone()), pane, rows, columns);
        wait_until("the Pane at the phone's size", || {
            companion
                .terminal(address.clone(), pane)
                .is_some_and(|screen| screen.rows == rows && screen.columns == columns)
        });
        Self {
            companion,
            address,
            pane,
            endpoint,
            handle,
            thread,
        }
    }

    fn screen(&self) -> Option<TerminalScreen> {
        self.companion.terminal(self.address.clone(), self.pane)
    }

    fn status(&self) -> DeviceStatus {
        self.companion.devices()[0].status.clone()
    }

    /// The Pane's size as a fresh Bootstrap tells it.
    fn server_size(&self) -> Option<TerminalSize> {
        let desktop = ClientConnection::connect(&self.endpoint, "check").unwrap();
        let bootstrap = desktop.bootstrap().unwrap();
        let terminal = bootstrap
            .terminals
            .iter()
            .find(|terminal| terminal.pane_id.as_u64() == self.pane)?;
        terminal.view.as_ref().map(|view| view.size)
    }

    /// A desktop window that never reported focus sizes the Pane, which it can only once
    /// the phone has given the size back (ADR 0036).
    fn desktop_resizes(&self, rows: u16, columns: u16) {
        let size = TerminalSize::new(rows, columns);
        let mut desktop = ClientConnection::connect_overview(&self.endpoint, "desktop").unwrap();
        wait_until("the phone to give the size back", || {
            desktop
                .terminal(PaneId::from_u64(self.pane), TerminalCommand::Resize(size))
                .unwrap();
            self.server_size() == Some(size)
        });
    }

    fn desktop_types(&self, text: &str) {
        let mut desktop = ClientConnection::connect_overview(&self.endpoint, "desktop").unwrap();
        desktop
            .terminal(
                PaneId::from_u64(self.pane),
                TerminalCommand::Text(text.into()),
            )
            .unwrap();
    }

    fn stop(self) {
        self.companion.close();
        self.handle.stop();
        self.thread.join().unwrap().unwrap();
    }
}

fn row_text(screen: &TerminalScreen, row: u16) -> String {
    let columns = usize::from(screen.columns);
    screen.cells[usize::from(row) * columns..][..columns]
        .iter()
        .map(|cell| {
            if cell.text.is_empty() {
                " "
            } else {
                cell.text.as_str()
            }
        })
        .collect()
}

/// Where `word` is on the screen, as (row, column).
fn find(screen: &TerminalScreen, word: &str) -> Option<(u16, u16)> {
    (0..screen.rows).find_map(|row| {
        row_text(screen, row)
            .find(word)
            .map(|column| (row, column as u16))
    })
}

/// A long press selects a word in the Pane's shared selection, shown in the selection's
/// colours, and Copy brings its text back (ADR 0008, ADR 0041).
#[test]
fn a_long_press_selects_a_word_and_copy_returns_it() {
    let shown = Shown::start(12, 60);
    shown
        .companion
        .send_text(shown.address.clone(), shown.pane, "echo CONDRWORD".into());
    shown
        .companion
        .send_key(shown.address.clone(), shown.pane, Key::Enter, false);
    // The word is printed under the command line that typed it.
    wait_until("the echoed word", || {
        shown.screen().is_some_and(|screen| {
            (0..screen.rows)
                .filter(|row| row_text(&screen, *row).trim() == "CONDRWORD")
                .count()
                == 1
        })
    });
    let screen = shown.screen().unwrap();
    let row = (0..screen.rows)
        .find(|row| row_text(&screen, *row).trim() == "CONDRWORD")
        .unwrap();
    let (_, column) = find(&screen, "CONDRWORD")
        .filter(|(found, _)| *found == row)
        .unwrap_or((row, 0));

    shown
        .companion
        .select_word(shown.address.clone(), shown.pane, row, column + 2);
    wait_until("the selection on screen", || {
        shown.screen().is_some_and(|screen| {
            screen.has_selection
                && screen.cells
                    [usize::from(row) * usize::from(screen.columns) + usize::from(column)]
                .background
                    != screen.background
        })
    });
    shown.companion.take_events();
    shown
        .companion
        .copy_selection(shown.address.clone(), shown.pane);
    let mut copied = None;
    wait_until("the copied text", || {
        copied = shown
            .companion
            .take_events()
            .into_iter()
            .find_map(|event| match event {
                Event::Copied { text } => Some(text),
                _ => None,
            });
        copied.is_some()
    });
    assert_eq!(copied.as_deref(), Some("CONDRWORD"));

    shown
        .companion
        .clear_selection(shown.address.clone(), shown.pane);
    wait_until("the selection gone", || {
        shown.screen().is_some_and(|screen| !screen.has_selection)
    });
    shown.stop();
}

/// Pasted text reaches the program as a paste, then Return runs it (ADR 0041).
#[test]
fn pasted_text_runs_once_return_is_pressed() {
    let shown = Shown::start(12, 60);
    shown.companion.paste(
        shown.address.clone(),
        shown.pane,
        "echo CONDR_PASTED".into(),
    );
    shown
        .companion
        .send_key(shown.address.clone(), shown.pane, Key::Enter, false);
    wait_until("the pasted command's output", || {
        shown.screen().is_some_and(|screen| {
            (0..screen.rows).any(|row| row_text(&screen, row).trim() == "CONDR_PASTED")
        })
    });
    shown.stop();
}

/// Leaving the screen gives the shown Pane's size back at once, and coming back takes it
/// again with a fresh Bootstrap, even when the two follow each other at once (ADR 0040).
#[test]
fn suspend_gives_the_size_back_and_resume_takes_it_again() {
    let shown = Shown::start(12, 60);
    shown.companion.suspend();
    assert_eq!(shown.status(), DeviceStatus::Suspended);
    shown.desktop_resizes(30, 100);
    shown.desktop_types("echo while-away\r");

    shown.companion.resume();
    wait_until("the phone's size again", || {
        shown.server_size() == Some(TerminalSize::new(12, 60))
    });
    wait_until("what happened while away", || {
        shown.screen().is_some_and(|screen| {
            (screen.rows, screen.columns) == (12, 60) && find(&screen, "while-away").is_some()
        })
    });

    shown.companion.suspend();
    shown.companion.resume();
    wait_until("the connection again", || {
        shown.status() == DeviceStatus::Connected
    });
    wait_until("the phone's size again", || {
        shown.server_size() == Some(TerminalSize::new(12, 60))
    });
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        shown.status(),
        DeviceStatus::Connected,
        "the closed connection does not mark the new one"
    );
    shown.stop();
}

/// Under Control Center or the app switcher the phone stays connected but gives the shown
/// Pane's size back, and takes it again, at the size it measured meanwhile, once the app
/// is active (ADR 0041).
#[test]
fn an_inactive_app_gives_the_size_back_until_it_is_active_again() {
    let shown = Shown::start(12, 60);
    shown.companion.set_active(false);
    shown.desktop_resizes(30, 100);
    wait_until("the phone to follow the desktop's size", || {
        shown.screen().is_some_and(|screen| screen.columns == 100)
    });
    shown
        .companion
        .show_pane(Some(shown.address.clone()), shown.pane, 14, 60);

    shown.companion.set_active(true);
    wait_until("the phone's size again", || {
        shown.server_size() == Some(TerminalSize::new(14, 60))
    });
    shown.stop();
}

/// Leaving the screen breaks off a dial still in its handshake: the socket closes at once
/// rather than when the handshake times out, and no failure turns up later (ADR 0040).
#[test]
fn suspend_breaks_off_a_dial_in_progress() {
    // A Server that takes the connection and never answers the handshake.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let server_key = condr_mobile::fingerprint(condr_mobile::generate_seed().unwrap()).unwrap();
    let address = format!("tcp://{server_key}@{}", listener.local_addr().unwrap());
    let companion = Companion::new(
        condr_mobile::generate_seed().unwrap(),
        "phone".into(),
        vec![SavedDevice {
            address,
            label: "desk".into(),
        }],
        Arc::new(Counter::default()),
    )
    .unwrap();
    companion.resume();
    let (mut socket, _) = listener.accept().unwrap();
    let mut first = [0; 1];
    socket.read_exact(&mut first).unwrap();

    let started = Instant::now();
    companion.suspend();
    assert_eq!(companion.devices()[0].status, DeviceStatus::Suspended);
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut rest = Vec::new();
    socket
        .read_to_end(&mut rest)
        .expect("the phone closes the socket");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "closed at once, not when the handshake timed out: {:?}",
        started.elapsed()
    );
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        companion.devices()[0].status,
        DeviceStatus::Suspended,
        "no failure turns up later"
    );
    companion.close();
}
