use std::cell::RefCell;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use condr_core::SplitDirection;
use condr_core::protocol::{
    ClientMessage, LayoutCommand, PaneAgentSnapshot, PaneTerminalFrame, RuntimeEpoch, ServerId,
    ServerMessage, SessionBootstrap, SessionEvent, SessionId, TerminalFrameBatch,
};
use condr_core::{
    AgentKind, AgentSnapshot, AgentState, AgentTracker, PaneId, PaneLayout, Session, TabId,
    TerminalCell, TerminalColor, TerminalCommand, TerminalMouseButton, TerminalMouseEvent,
    TerminalMousePosition, TerminalMouseTracking, TerminalPosition, TerminalSelection,
    TerminalSide, TerminalSize, TerminalView, TerminalViewFrame, WorkspaceId,
};
use condr_server::{BoundServer, ClientConnection, Endpoint, ServerConfig, ServerHandle};
use gpui::{
    AppContext as _, Background, ClipboardItem, Entity, Modifiers, MouseButton, MouseDownEvent,
    MouseUpEvent, Task, TestAppContext, VisualTestContext, point, px, size,
};

use crate::terminal_element::{TerminalElement, TerminalElementProps, TerminalRenderCache};
use gpui_component::dialog::Confirm;
use gpui_component::{ActiveTheme as _, Root, WindowExt as _};

use super::super::{
    Appearance, CONTROL_BUSY_REASON, ClientIo, Condr, ConnectionResult, ConnectionStatus,
    DEFAULT_WINDOW_SIZE, DockSurfaceKey, Incoming, LocalTerminalSelection,
    PendingWorkspaceSelection, ReportedTerminalMouseMotion, ServerConnection, TerminalFont,
    TerminalPalette, color_scheme_is_dirty, default_window_options, reset_color_scheme,
    select_appearance, select_server_shell, select_settings_server, select_terminal_font_family,
    select_terminal_font_size, selected_appearance, server_shell, step_terminal_font_size,
    terminal_font_family, terminal_font_size,
};

/// A ceiling, not an expected wait: the loops return as soon as the condition holds.
/// Five seconds was not enough on Linux when the whole workspace runs in parallel.
const TEST_TIMEOUT: Duration = Duration::from_secs(15);
const TEST_POLL_INTERVAL: Duration = Duration::from_millis(2);
static NEXT_TEST_SERVER_ID: AtomicU64 = AtomicU64::new(1);
static VISUAL_TEST_LOCK: Mutex<()> = Mutex::new(());

struct TestServer {
    handle: ServerHandle,
    thread: Option<JoinHandle<std::io::Result<()>>>,
}

impl TestServer {
    fn stop(&mut self) {
        self.handle.stop();
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap().unwrap();
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn acquire_visual_test_lock() -> MutexGuard<'static, ()> {
    VISUAL_TEST_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

fn start_server() -> (TestServer, Endpoint) {
    let endpoint = Endpoint::local(std::env::temp_dir().join(format!(
        "condr-{}-{}.sock",
        std::process::id(),
        NEXT_TEST_SERVER_ID.fetch_add(1, Ordering::Relaxed),
    )));
    let server = start_server_with_config(ServerConfig::ephemeral(endpoint.clone()));
    (server, endpoint)
}

fn start_tcp_server() -> (TestServer, Endpoint) {
    let server = BoundServer::bind(ServerConfig::ephemeral(Endpoint::tcp(
        "127.0.0.1:0".parse().unwrap(),
    )))
    .unwrap();
    let endpoint = Endpoint::tcp(server.local_addr().unwrap().unwrap());
    let handle = server.handle();
    let thread = std::thread::spawn(move || server.run());
    (
        TestServer {
            handle,
            thread: Some(thread),
        },
        endpoint,
    )
}

fn start_server_with_config(config: ServerConfig) -> TestServer {
    let server = BoundServer::bind(config).unwrap();
    let handle = server.handle();
    let thread = std::thread::spawn(move || server.run());
    TestServer {
        handle,
        thread: Some(thread),
    }
}

fn connected_condr(cx: &mut TestAppContext) -> (Entity<Condr>, &mut VisualTestContext, TestServer) {
    let (server, endpoint) = start_server();
    connected_condr_with(cx, server, endpoint)
}

fn connected_condr_with(
    cx: &mut TestAppContext,
    server: TestServer,
    endpoint: Endpoint,
) -> (Entity<Condr>, &mut VisualTestContext, TestServer) {
    let mut initial = None;
    let deadline = Instant::now() + TEST_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(connection) = ClientConnection::connect(&endpoint, "condr-test") {
            initial = Some(connection);
            break;
        }
        std::thread::sleep(TEST_POLL_INTERVAL);
    }
    let initial = initial.map_or_else(
        || Err("test server did not accept a client connection".into()),
        Ok,
    );
    let view_holder = Rc::new(RefCell::new(None));
    let view_holder_for_window = view_holder.clone();
    let (_root, window) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| Condr::new(endpoint, initial, None, window, cx));
        view_holder_for_window.borrow_mut().replace(view.clone());
        Root::new(view, window, cx)
    });
    let view = view_holder
        .borrow_mut()
        .take()
        .expect("Condr view should be created with the Root");
    if wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .is_some_and(ServerConnection::can_mutate)
        })
    }) {
        return (view, window, server);
    }
    panic!("GUI did not acquire control from the test server");
}

fn terminal_selector(pane_id: PaneId) -> &'static str {
    Box::leak(format!("terminal-pane-{}", pane_id.as_u64()).into_boxed_str())
}

fn terminal_contains(
    window: &mut VisualTestContext,
    view: &Entity<Condr>,
    connection_key: u64,
    pane_id: PaneId,
    expected: &str,
) -> bool {
    window.read(|app| {
        view.read(app)
            .connection(connection_key)
            .and_then(|connection| connection.terminals.get(&pane_id))
            .is_some_and(|terminal| {
                terminal
                    .view
                    .cells
                    .iter()
                    .map(|cell| cell.text.as_str())
                    .collect::<String>()
                    .contains(expected)
            })
    })
}

fn bootstrap_for_session(connection: &ServerConnection, session: &Session) -> SessionBootstrap {
    SessionBootstrap {
        settings: Default::default(),
        server_id: connection.server_id.expect("connected Server has an ID"),
        runtime_epoch: connection
            .runtime_epoch
            .expect("connected Server has a runtime epoch"),
        session_id: connection
            .session_id
            .expect("connected Server has a Session ID"),
        sequence: connection.sequence,
        snapshot: session.snapshot(),
        terminals: connection.terminals.values().cloned().collect(),
        agents: connection
            .agents
            .iter()
            .map(|(&pane_id, agent)| PaneAgentSnapshot {
                pane_id,
                agent: *agent,
            })
            .collect(),
        workspace_git: connection.workspace_git.values().cloned().collect(),
        zoomed_panes: connection.zoomed_panes.iter().copied().collect(),
    }
}

fn tab_selector(tab_id: TabId) -> &'static str {
    Box::leak(format!("tab-{}", tab_id.as_u64()).into_boxed_str())
}

fn leaked_selector(selector: String) -> &'static str {
    Box::leak(selector.into_boxed_str())
}

fn sidebar_workspace_selector(workspace_id: WorkspaceId) -> &'static str {
    leaked_selector(format!("workspace-1-{}", workspace_id.as_u64()))
}

#[cfg(target_os = "linux")]
fn sidebar_agent_selector(pane_id: PaneId) -> &'static str {
    leaked_selector(format!("agent-1-{}", pane_id.as_u64()))
}

struct TestDirectory(std::path::PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "condr-{label}-{}-{}",
            std::process::id(),
            NEXT_TEST_SERVER_ID.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_git<I, S>(cwd: &std::path::Path, args: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let args = args
        .into_iter()
        .map(|argument| argument.as_ref().to_os_string())
        .collect::<Vec<_>>();
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(&args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Git {args:?} failed with {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn submit_text_dialog(window: &mut VisualTestContext, value: &str) {
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.update(|window, cx| window.has_active_dialog(cx)));
    assert!(window.update(|window, cx| window.has_focused_input(cx)));
    window.simulate_input(value);
    window.update(|window, cx| _ = window.draw(cx));
    let cancel = window
        .debug_bounds("dialog-cancel")
        .expect("text dialog should render Cancel");
    let primary = window
        .debug_bounds("dialog-primary-action")
        .expect("text dialog should render its primary action");
    assert!(
        cancel.left() < primary.left(),
        "use the default button order"
    );
    assert!(
        primary.left() - cancel.right() <= px(12.),
        "keep text dialog actions compact"
    );
    window.simulate_click(primary.center(), Modifiers::default());
}

fn confirm_alert_dialog(window: &mut VisualTestContext) {
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.update(|window, cx| window.has_active_dialog(cx)));
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        window.update(|window, cx| {
            window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
        });
        window.run_until_parked();
        if !window.update(|window, cx| window.has_active_dialog(cx)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "confirming the alert should eventually close it"
        );
        window.executor().advance_clock(Duration::from_millis(20));
        std::thread::sleep(TEST_POLL_INTERVAL);
    }
}

fn wait_until(
    window: &mut VisualTestContext,
    mut predicate: impl FnMut(&mut VisualTestContext) -> bool,
) -> bool {
    let deadline = Instant::now() + TEST_TIMEOUT;
    while Instant::now() < deadline {
        window.executor().advance_clock(Duration::from_millis(20));
        window.run_until_parked();
        if predicate(window) {
            return true;
        }
        std::thread::sleep(TEST_POLL_INTERVAL);
    }
    false
}

fn wait_until_event_driven(
    window: &mut VisualTestContext,
    mut predicate: impl FnMut(&mut VisualTestContext) -> bool,
) -> bool {
    let deadline = Instant::now() + TEST_TIMEOUT;
    while Instant::now() < deadline {
        window.run_until_parked();
        if predicate(window) {
            return true;
        }
        std::thread::sleep(TEST_POLL_INTERVAL);
    }
    false
}

mod connection;
mod layout;
mod terminal;
mod workflows;
