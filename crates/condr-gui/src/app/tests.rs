mod connection;
mod input;
mod session_model;
mod sidebar;
mod startup;
#[cfg(feature = "test-support")]
mod visual;

use super::ClientTerminal;
use super::terminal_input::{
    clipboard_image_format, is_image_paste_gesture, next_hovered_link, prepare_clipboard_image,
    terminal_key_for,
};
use super::{
    ActivateTab, BELL_SIDEBAR_STATUS, ClientIo, ClosePane, CondrAssets, ConnectionStatus,
    FocusLeft, NewTab, NextTab, NextWorkspace, OpenSettings, PreviousTab, PreviousWorkspace,
    ServerConnection, SidebarGlyph, SidebarIconTone, SplitDown, SplitRight,
    TerminalClipboardShortcut, ToggleChanges, ToggleSidebar, ToggleZoom, accepted_text_input,
    agent_sidebar_status, agent_status_summary, clear_pending_sizes_for_bootstrap,
    connect_to_server_with, fixed_shortcut, lock_exclusively, reorder_connection,
    should_defer_to_character_input, single_instance_lock_path, terminal_clipboard_shortcut,
    upstream_label,
};
use crate::terminal_element::HoveredTerminalLink;
use condr_client::frames::read_bootstrap_batches;
use condr_core::TerminalKey;
use condr_core::protocol::{
    AgentCommand, BootstrapBatch, BootstrapHeader, BootstrapRecord, ClientMessage,
    PaneTerminalSnapshot, RuntimeEpoch, ServerId, ServerMessage, SessionBootstrap, SessionId,
    encode_bootstrap_record,
};
use condr_core::{
    AgentDisplayState, Session, TerminalCell, TerminalColor, TerminalMouseTracking, TerminalSize,
    TerminalView,
};
use condr_server::{DeviceKey, Endpoint, TcpEndpoint};
use gpui_kit::{AssetSource as _, KeyDownEvent, Keystroke, Task};

fn terminal_cell(text: &str) -> TerminalCell {
    TerminalCell {
        text: text.into(),
        foreground: TerminalColor::Named(0),
        background: TerminalColor::Named(0),
        flags: 0,
        hyperlink: None,
    }
}

/// A TCP endpoint with fixed keys, so two calls with one address compare equal.
pub(super) fn tcp(address: &str) -> Endpoint {
    Endpoint::tcp(TcpEndpoint::at(
        address.parse().unwrap(),
        DeviceKey::from_seed([7; 32]).public(),
        DeviceKey::from_seed([9; 32]),
    ))
}

fn connection_with_io() -> ServerConnection {
    let mut connection = ServerConnection::new(1, "test".into(), tcp("127.0.0.1:9"));
    connection.status = ConnectionStatus::Connected;
    connection.model.server_id = Some(ServerId(1));
    connection.model.runtime_epoch = Some(RuntimeEpoch(2));
    connection.model.session_id = Some(SessionId(3));
    connection.model.sequence = 7;
    let (outgoing, outgoing_rx) = std::sync::mpsc::channel();
    std::mem::forget(outgoing_rx);
    connection.io = Some(ClientIo {
        outgoing,
        _incoming_task: Task::ready(()),
    });
    connection
}

fn terminal_view(revision: u64, text: &str) -> TerminalView {
    let cells = text
        .chars()
        .map(|character| terminal_cell(&character.to_string()))
        .collect::<Vec<_>>();
    TerminalView {
        selection: None,
        revision,
        size: TerminalSize::new(1, u16::try_from(cells.len()).unwrap()),
        display_offset: 0,
        mouse_tracking: TerminalMouseTracking::None,
        cells,
        cursor: None,
    }
}

fn pane_id() -> condr_core::PaneId {
    let mut session = Session::new();
    session
        .create_workspace(std::env::temp_dir())
        .expect("Workspace capacity");
    session.workspaces()[0].tabs()[0]
        .focused_pane()
        .unwrap()
        .id()
}
