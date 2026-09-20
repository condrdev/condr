mod connection;
mod input;
mod sidebar;
mod startup;
mod terminal_frames;
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
    TerminalClipboardShortcut, TerminalVisualSlot, ToggleChanges, ToggleSidebar, ToggleZoom,
    accepted_text_input, agent_sidebar_status, agent_status_summary, apply_terminal_frame_batch,
    assemble_terminal_frame_chunk, clear_pending_sizes_for_bootstrap, connect_to_server_with,
    enforce_terminal_chunk_reliable_fence, fixed_shortcut, lock_exclusively, merge_terminal_deltas,
    read_bootstrap_batches, reorder_connection, should_defer_to_character_input,
    single_instance_lock_path, terminal_chunk_identity_matches, terminal_clipboard_shortcut,
    upstream_label,
};
use crate::terminal_element::HoveredTerminalLink;
use condr_core::TerminalKey;
use condr_core::protocol::{
    AgentCommand, BootstrapBatch, BootstrapHeader, BootstrapRecord, ClientMessage,
    PaneTerminalFrame, PaneTerminalSnapshot, RuntimeEpoch, ServerId, ServerMessage,
    SessionBootstrap, SessionEvent, SessionId, TerminalFrameBatch, TerminalFrameChunk,
    encode_bootstrap_record, encode_pane_terminal_frame,
};
use condr_core::{
    AgentDisplayState, Session, TerminalCell, TerminalCellRun, TerminalColor,
    TerminalHyperlinkBudget, TerminalMouseTracking, TerminalSize, TerminalView, TerminalViewDelta,
    TerminalViewFrame,
};
use condr_server::{Endpoint, StaticKey, TcpEndpoint};
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

fn terminal_hyperlink_budgets(
    terminals: &mut std::collections::HashMap<condr_core::PaneId, ClientTerminal>,
) -> std::collections::HashMap<condr_core::PaneId, TerminalHyperlinkBudget> {
    terminals
        .iter_mut()
        .map(|(&pane_id, terminal)| {
            (
                pane_id,
                TerminalHyperlinkBudget::new(std::sync::Arc::make_mut(&mut terminal.view)),
            )
        })
        .collect()
}

/// A TCP endpoint with fixed keys, so two calls with one address compare equal.
pub(super) fn tcp(address: &str) -> Endpoint {
    Endpoint::tcp(TcpEndpoint::at(
        address.parse().unwrap(),
        StaticKey::from_private([7; 32]).public(),
        StaticKey::from_private([9; 32]),
    ))
}

fn connection_with_io() -> ServerConnection {
    let mut connection = ServerConnection::new(1, "test".into(), tcp("127.0.0.1:9"));
    connection.status = ConnectionStatus::Connected;
    connection.server_id = Some(ServerId(1));
    connection.runtime_epoch = Some(RuntimeEpoch(2));
    connection.session_id = Some(SessionId(3));
    connection.sequence = 7;
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
