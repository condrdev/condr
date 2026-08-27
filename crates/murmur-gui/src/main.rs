mod terminal_element;

use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::{ActiveTheme as _, Disableable as _, Root, StyledExt as _, v_flex};
use murmur_core::protocol::{
    ClientMessage, PaneTerminalSnapshot, ServerId, ServerMessage, SessionBootstrap, SessionEvent,
    SessionId,
};
use murmur_core::{
    PaneId, Session, SessionSnapshot, TerminalCommand, TerminalKey, TerminalModifiers,
    TerminalPosition, TerminalScroll, TerminalSize,
};
use murmur_server::ClientConnection;

use crate::terminal_element::TerminalElement;

enum Incoming {
    Message(ServerMessage),
    Disconnected(String),
}

struct ClientIo {
    outgoing: mpsc::Sender<ClientMessage>,
    incoming: mpsc::Receiver<Incoming>,
}

impl ClientIo {
    fn start(connection: ClientConnection) -> std::io::Result<Self> {
        let mut reader = connection.into_stream();
        let mut writer = reader.try_clone()?;
        let (outgoing, outgoing_rx) = mpsc::channel();
        let (incoming_tx, incoming) = mpsc::channel();
        let writer_events = incoming_tx.clone();

        thread::Builder::new()
            .name("murmur-client-writer".into())
            .spawn(move || {
                while let Ok(message) = outgoing_rx.recv() {
                    if let Err(error) = murmur_core::protocol::write_message(&mut writer, &message)
                    {
                        let _ = writer_events.send(Incoming::Disconnected(error.to_string()));
                        break;
                    }
                }
            })?;
        thread::Builder::new()
            .name("murmur-client-reader".into())
            .spawn(move || {
                loop {
                    match murmur_core::protocol::read_message(&mut reader) {
                        Ok(message) => {
                            if incoming_tx.send(Incoming::Message(message)).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = incoming_tx.send(Incoming::Disconnected(error.to_string()));
                            break;
                        }
                    }
                }
            })?;

        Ok(Self { outgoing, incoming })
    }
}

pub(crate) struct Murmur {
    server_id: ServerId,
    session_id: SessionId,
    sequence: u64,
    snapshot: SessionSnapshot,
    terminals: HashMap<PaneId, PaneTerminalSnapshot>,
    active_pane: Option<PaneId>,
    outgoing: mpsc::Sender<ClientMessage>,
    incoming: mpsc::Receiver<Incoming>,
    focus_handle: FocusHandle,
    controlling: bool,
    error: Option<String>,
    selecting_from: Option<TerminalPosition>,
    pending_size: Option<TerminalSize>,
    terminal_bounds: Option<Bounds<Pixels>>,
    cell_size: Size<Pixels>,
    marked_text: Option<String>,
}

impl Murmur {
    fn new(bootstrap: SessionBootstrap, io: ClientIo, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            server_id: bootstrap.server_id,
            session_id: bootstrap.session_id,
            sequence: bootstrap.sequence,
            snapshot: bootstrap.snapshot,
            terminals: bootstrap
                .terminals
                .into_iter()
                .map(|terminal| (terminal.pane_id, terminal))
                .collect(),
            active_pane: None,
            outgoing: io.outgoing,
            incoming: io.incoming,
            focus_handle: cx.focus_handle(),
            controlling: false,
            error: None,
            selecting_from: None,
            pending_size: None,
            terminal_bounds: None,
            cell_size: size(px(1.), px(1.)),
            marked_text: None,
        };
        this.refresh_active_pane();
        this.send(ClientMessage::AcquireControl {
            session_id: this.session_id,
        });
        this.send(ClientMessage::Subscribe {
            session_id: this.session_id,
            after_sequence: this.sequence,
        });
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        let mut changed = false;
                        while let Ok(incoming) = this.incoming.try_recv() {
                            this.handle_incoming(incoming, cx);
                            changed = true;
                        }
                        if changed {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        this
    }

    fn send(&mut self, message: ClientMessage) {
        if self.outgoing.send(message).is_err() {
            self.error = Some("Disconnected from murmur-server".into());
            self.controlling = false;
        }
    }

    fn handle_incoming(&mut self, incoming: Incoming, cx: &mut Context<Self>) {
        let message = match incoming {
            Incoming::Message(message) => message,
            Incoming::Disconnected(error) => {
                self.error = Some(format!("Server connection closed: {error}"));
                self.controlling = false;
                return;
            }
        };

        match message {
            ServerMessage::Bootstrap(bootstrap) => self.apply_bootstrap(bootstrap),
            ServerMessage::Event {
                sequence, event, ..
            } => {
                self.sequence = sequence;
                match event {
                    SessionEvent::SnapshotChanged => self.send(ClientMessage::SnapshotRequest {
                        session_id: self.session_id,
                    }),
                    SessionEvent::TerminalChanged { pane_id, view } => {
                        self.pending_size = self.pending_size.filter(|pending| {
                            pending.rows != view.size.rows
                                || pending.columns != view.size.columns
                                || pending.cell_width != view.size.cell_width
                                || pending.cell_height != view.size.cell_height
                        });
                        self.terminals.insert(
                            pane_id,
                            PaneTerminalSnapshot {
                                pane_id,
                                view,
                                exited: false,
                            },
                        );
                    }
                    SessionEvent::TerminalExited { pane_id, view } => {
                        self.terminals.insert(
                            pane_id,
                            PaneTerminalSnapshot {
                                pane_id,
                                view,
                                exited: true,
                            },
                        );
                    }
                }
            }
            ServerMessage::ControlGranted { .. } => {
                self.controlling = true;
                self.error = None;
            }
            ServerMessage::ControlReleased { .. } => self.controlling = false,
            ServerMessage::ControlDenied { reason, .. }
            | ServerMessage::Error { message: reason } => self.error = Some(reason),
            ServerMessage::TerminalCopied { text, .. } => {
                if let Some(text) = text {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            ServerMessage::ServerStopping => {
                self.error = Some("murmur-server stopped".into());
                self.controlling = false;
            }
            ServerMessage::Welcome { .. }
            | ServerMessage::Subscribed { .. }
            | ServerMessage::Pong { .. } => {}
        }
    }

    fn apply_bootstrap(&mut self, bootstrap: SessionBootstrap) {
        self.server_id = bootstrap.server_id;
        self.session_id = bootstrap.session_id;
        self.sequence = bootstrap.sequence;
        self.snapshot = bootstrap.snapshot;
        self.terminals = bootstrap
            .terminals
            .into_iter()
            .map(|terminal| (terminal.pane_id, terminal))
            .collect();
        self.refresh_active_pane();
    }

    fn refresh_active_pane(&mut self) {
        self.active_pane = Session::restore(self.snapshot.clone())
            .ok()
            .and_then(|session| Some(session.active_workspace()?.active_tab().focused_pane().id()));
    }

    fn active_terminal(&self) -> Option<&PaneTerminalSnapshot> {
        self.terminals.get(&self.active_pane?)
    }

    fn terminal_command(&mut self, command: TerminalCommand) {
        let Some(pane_id) = self.active_pane else {
            return;
        };
        self.send(ClientMessage::Terminal {
            server_id: self.server_id,
            session_id: self.session_id,
            pane_id,
            command,
        });
    }

    fn create_workspace(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let root_directory = home_directory().unwrap_or_else(|| PathBuf::from("."));
        self.send(ClientMessage::CreateWorkspace {
            server_id: self.server_id,
            session_id: self.session_id,
            root_directory,
        });
        cx.notify();
    }

    pub(crate) fn resize_terminal(&mut self, size: TerminalSize, cx: &mut Context<Self>) {
        if self.pending_size == Some(size)
            || self
                .active_terminal()
                .is_some_and(|terminal| terminal.view.size == size)
        {
            return;
        }
        self.pending_size = Some(size);
        self.terminal_command(TerminalCommand::Resize(size));
        cx.notify();
    }

    pub(crate) fn update_terminal_geometry(
        &mut self,
        bounds: Bounds<Pixels>,
        cell_size: Size<Pixels>,
    ) {
        self.terminal_bounds = Some(bounds);
        self.cell_size = cell_size;
    }

    pub(crate) fn begin_selection(
        &mut self,
        position: TerminalPosition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window, cx);
        self.selecting_from = Some(position);
        self.terminal_command(TerminalCommand::Select {
            start: position,
            end: position,
        });
    }

    pub(crate) fn update_selection(&mut self, position: TerminalPosition) {
        if let Some(start) = self.selecting_from {
            self.terminal_command(TerminalCommand::Select {
                start,
                end: position,
            });
        }
    }

    pub(crate) fn is_selecting(&self) -> bool {
        self.selecting_from.is_some()
    }

    pub(crate) fn end_selection(&mut self, position: TerminalPosition) {
        self.update_selection(position);
        self.selecting_from = None;
    }

    pub(crate) fn scroll_terminal(&mut self, lines: i32) {
        if lines != 0 {
            self.terminal_command(TerminalCommand::Scroll(TerminalScroll::Lines(lines)));
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        let modifiers = stroke.modifiers;
        let copy_paste = modifiers.platform || (modifiers.control && modifiers.shift);
        if copy_paste && stroke.key == "c" {
            self.terminal_command(TerminalCommand::Copy);
            cx.stop_propagation();
            return;
        }
        if copy_paste && stroke.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.terminal_command(TerminalCommand::Paste(text));
            }
            cx.stop_propagation();
            return;
        }
        if modifiers.shift && stroke.key == "pageup" {
            self.terminal_command(TerminalCommand::Scroll(TerminalScroll::PageUp));
            cx.stop_propagation();
            return;
        }
        if modifiers.shift && stroke.key == "pagedown" {
            self.terminal_command(TerminalCommand::Scroll(TerminalScroll::PageDown));
            cx.stop_propagation();
            return;
        }

        let key = match stroke.key.as_str() {
            "enter" => Some(TerminalKey::Enter),
            "tab" if modifiers.shift => Some(TerminalKey::BackTab),
            "tab" => Some(TerminalKey::Tab),
            "backspace" => Some(TerminalKey::Backspace),
            "delete" => Some(TerminalKey::Delete),
            "escape" => Some(TerminalKey::Escape),
            "up" => Some(TerminalKey::Up),
            "down" => Some(TerminalKey::Down),
            "right" => Some(TerminalKey::Right),
            "left" => Some(TerminalKey::Left),
            "home" => Some(TerminalKey::Home),
            "end" => Some(TerminalKey::End),
            "pageup" => Some(TerminalKey::PageUp),
            "pagedown" => Some(TerminalKey::PageDown),
            "insert" => Some(TerminalKey::Insert),
            key if key.len() > 1 && key.starts_with('f') => key[1..]
                .parse::<u8>()
                .ok()
                .filter(|number| (1..=12).contains(number))
                .map(TerminalKey::Function),
            _ if modifiers.control || modifiers.alt || modifiers.platform => {
                Some(TerminalKey::Character(
                    stroke
                        .key_char
                        .clone()
                        .unwrap_or_else(|| stroke.key.clone()),
                ))
            }
            _ => None,
        };
        if let Some(key) = key {
            self.terminal_command(TerminalCommand::Key {
                key,
                modifiers: TerminalModifiers {
                    shift: modifiers.shift,
                    alt: modifiers.alt,
                    control: modifiers.control,
                    platform: modifiers.platform,
                },
            });
            cx.stop_propagation();
        }
    }
}

impl Focusable for Murmur {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for Murmur {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        adjusted_range.replace(
            0..self
                .marked_text
                .as_ref()
                .map_or(0, |text| text.encode_utf16().count()),
        );
        Some(self.marked_text.clone().unwrap_or_default())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_text
            .as_ref()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_text = None;
        cx.notify();
    }

    fn paste(&mut self, item: ClipboardItem, _: &mut Window, _: &mut Context<Self>) {
        if let Some(text) = item.text() {
            self.terminal_command(TerminalCommand::Paste(text));
        }
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        self.marked_text = None;
        if !text.is_empty() {
            self.terminal_command(TerminalCommand::Text(text.into()));
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        new_text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = (!new_text.is_empty()).then(|| new_text.to_owned());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let bounds = self.terminal_bounds?;
        let cursor = self.active_terminal()?.view.cursor?;
        Some(Bounds::new(
            point(
                bounds.left() + self.cell_size.width * f32::from(cursor.column),
                bounds.top() + self.cell_size.height * f32::from(cursor.row),
            ),
            self.cell_size,
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }
}

impl Render for Murmur {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let terminal = self.active_terminal().cloned();
        let error = self.error.clone();
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .h(px(36.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().font_semibold().child(murmur_core::APP_NAME))
                    .when_some(error, |header, error| {
                        header.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().danger)
                                .truncate()
                                .child(error),
                        )
                    }),
            )
            .child(if let Some(terminal) = terminal {
                div()
                    .id("terminal-input")
                    .key_context("Terminal")
                    .track_focus(&self.focus_handle)
                    .on_key_down(cx.listener(Self::key_down))
                    .flex_1()
                    .overflow_hidden()
                    .p_2()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(cx.theme().mono_font_size)
                    .line_height(relative(1.35))
                    .border_1()
                    .border_color(if self.focus_handle.is_focused(window) {
                        cx.theme().ring
                    } else {
                        cx.theme().border
                    })
                    .child(TerminalElement::new(
                        cx.entity(),
                        terminal.view,
                        self.marked_text.clone(),
                    ))
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        Button::new("new-terminal-workspace")
                            .primary()
                            .label("New Terminal Workspace")
                            .disabled(!self.controlling)
                            .on_click(cx.listener(Self::create_workspace)),
                    )
                    .into_any_element()
            })
    }
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

fn main() {
    let endpoint =
        murmur_server::ensure_local_server().expect("failed to discover or start murmur-server");
    let connection = ClientConnection::connect(&endpoint, "murmur-gui")
        .expect("failed to connect to murmur-server");
    let bootstrap = connection.bootstrap().clone();
    let io = ClientIo::start(connection).expect("failed to start the GUI protocol connection");
    let app = gpui_platform::application();

    app.run(move |cx| {
        gpui_component::init(cx);
        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|cx| Murmur::new(bootstrap, io, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open Murmur window");
        })
        .detach();
    });
}
