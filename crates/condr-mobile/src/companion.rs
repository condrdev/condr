//! Every paired Device's connection and model, and the notifier that tells the app (ADR
//! 0039, ADR 0040). One lock guards the whole state: the app reads it on its main thread,
//! each connection's pump writes it, and nothing slow runs under it.

use crate::views::{
    AgentView, Change, DeviceStatus, DeviceView, Event, Key, NeedsYouItem, PaneView, SavedDevice,
    TerminalScreen, WorkspaceView, screen,
};
use crate::{Listener, MobileError};
use condr_client::frames::TerminalVisualSlot;
use condr_client::p2p::Network;
use condr_client::{
    Cancellation, ClientConnection, ConnectionIo, DeviceKey, Incoming, P2pEndpoint, P2pNode,
    RemoteEndpoint, RemoteStream, SessionModel, TcpEndpoint,
};
use condr_core::protocol::{ClientMessage, ServerMessage, SessionEvent};
use condr_core::{
    PaneId, ProxySetting, TerminalCommand, TerminalKey, TerminalKeyEventKind, TerminalModifiers,
    TerminalScroll, TerminalSize,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, mpsc};
use std::thread;

/// How many one-time results wait for the app before the oldest goes.
const MAX_EVENTS: usize = 64;
/// Backpressure between a connection's reader and its pump, as the GUI's channel has.
const INCOMING_CAPACITY: usize = 256;

#[derive(uniffi::Object)]
pub struct Companion {
    shared: Arc<Shared>,
}

struct Shared {
    key: DeviceKey,
    /// What a Server records this phone as (ADR 0040).
    name: String,
    listener: Arc<dyn Listener>,
    runtime: tokio::runtime::Runtime,
    state: Mutex<State>,
    wake: Condvar,
}

#[derive(Default)]
struct State {
    devices: BTreeMap<String, Device>,
    node: Option<Arc<P2pNode>>,
    /// The Pane on screen: it takes the terminal's size and focus (ADR 0041), its Agent's
    /// idle is seen, and only its OSC 52 copies reach the clipboard.
    shown: Option<(String, PaneId)>,
    changes: BTreeSet<Change>,
    /// A `changed` is owed or delivered and `take_changes` has not run since.
    notified: bool,
    /// The notifier thread should call `changed`.
    wake_notifier: bool,
    events: VecDeque<Event>,
    suspended: bool,
    closed: bool,
}

struct Device {
    label: String,
    status: DeviceStatus,
    model: SessionModel,
    /// Bumped by every connect, suspend and forget, so a late result of an older
    /// connection changes nothing.
    generation: u64,
    cancellation: Cancellation<RemoteStream>,
    io: Option<ConnectionIo>,
    subscription: Subscription,
    snapshot_pending: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Subscription {
    None,
    Pending,
    Active,
}

impl State {
    fn mark(&mut self, change: Change) {
        self.changes.insert(change);
        if !self.notified {
            self.notified = true;
            self.wake_notifier = true;
        }
    }

    fn mark_device(&mut self, address: &str) {
        self.mark(Change::Device {
            address: address.to_owned(),
        });
        self.mark(Change::NeedsYou);
    }

    /// A new connection to `address` starts with nothing of the old one waiting: no error
    /// it raised, and no copy, which belonged to the Pane on screen then (ADR 0039).
    fn forget_events_of(&mut self, address: &str) {
        self.events.retain(|event| match event {
            Event::Error { address: from, .. } => from != address,
            Event::Copied { .. } | Event::Clipboard { .. } => false,
        });
    }

    fn push_event(&mut self, event: Event) {
        if matches!(event, Event::Clipboard { .. }) {
            self.events
                .retain(|waiting| !matches!(waiting, Event::Clipboard { .. }));
        }
        if self.events.len() == MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
        self.mark(Change::Devices);
    }
}

impl Device {
    fn new(label: String) -> Self {
        Self {
            label,
            status: DeviceStatus::Suspended,
            model: SessionModel::default(),
            generation: 0,
            cancellation: Cancellation::default(),
            io: None,
            subscription: Subscription::None,
            snapshot_pending: false,
        }
    }

    fn send(&self, message: ClientMessage) {
        if let Some(io) = &self.io {
            let _ = io.outgoing.send(message);
        }
    }

    fn terminal(&self, pane_id: PaneId, command: TerminalCommand) {
        let (Some(server_id), Some(session_id)) = (self.model.server_id, self.model.session_id)
        else {
            return;
        };
        self.send(ClientMessage::Terminal {
            server_id,
            session_id,
            pane_id,
            command,
        });
    }

    fn subscribe(&mut self) {
        if self.subscription != Subscription::None {
            return;
        }
        if let Some(session_id) = self.model.session_id {
            self.send(ClientMessage::Subscribe {
                session_id,
                after_sequence: self.model.sequence,
            });
            self.subscription = Subscription::Pending;
        }
    }

    fn request_snapshot(&mut self) {
        if self.snapshot_pending {
            return;
        }
        if let Some(session_id) = self.model.session_id {
            self.send(ClientMessage::SnapshotRequest { session_id });
            self.snapshot_pending = true;
        }
    }

    /// Ends the connection without waiting for the other side (ADR 0040): the Server
    /// sees it close, and with it this phone's focus and terminal sizing.
    fn disconnect(&mut self) {
        self.generation += 1;
        self.cancellation.cancel();
        self.io = None;
        self.subscription = Subscription::None;
        self.snapshot_pending = false;
    }
}

#[uniffi::export]
impl Companion {
    /// The library for the Device key `seed` holds, named `name` to the Servers it pairs
    /// with, knowing `devices`. It connects nothing until [`Self::resume`].
    #[uniffi::constructor]
    pub fn new(
        seed: Vec<u8>,
        name: String,
        devices: Vec<SavedDevice>,
        listener: Arc<dyn Listener>,
    ) -> Result<Arc<Self>, MobileError> {
        let key = crate::device_key(&seed)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("condr-p2p")
            .enable_all()
            .build()
            .map_err(|error| MobileError::Connection(error.to_string()))?;
        let state = State {
            devices: devices
                .into_iter()
                .map(|device| (device.address, Device::new(device.label)))
                .collect(),
            suspended: true,
            ..State::default()
        };
        let shared = Arc::new(Shared {
            key,
            name,
            listener,
            runtime,
            state: Mutex::new(state),
            wake: Condvar::new(),
        });
        let notifier = Arc::clone(&shared);
        thread::Builder::new()
            .name("condr-notifier".into())
            .spawn(move || notifier.notify_loop())
            .map_err(|error| MobileError::Connection(error.to_string()))?;
        Ok(Arc::new(Self { shared }))
    }

    /// Pairs with the Device an Invite link names (ADR 0040) and returns what the app
    /// keeps of it; connects to it unless the app is in the background. Blocks for the
    /// handshake, so the app calls it off its main thread.
    pub fn pair(&self, link: String, label: String) -> Result<SavedDevice, MobileError> {
        let link = link.trim();
        let (endpoint, address) = if link.starts_with("p2p://") {
            let p2p = P2pEndpoint::parse(link).map_err(MobileError::invalid)?;
            if p2p.invite.is_none() {
                return Err(MobileError::Invalid("the link carries no invite".into()));
            }
            let address = format!("p2p://{}", p2p.device);
            let node = self.shared.node();
            (RemoteEndpoint::P2p { node, address: p2p }, address)
        } else {
            let tcp =
                TcpEndpoint::parse(link, self.shared.key.clone()).map_err(MobileError::invalid)?;
            if tcp.invite.is_none() {
                return Err(MobileError::Invalid("the link carries no invite".into()));
            }
            let address = format!("tcp://{}@{}", tcp.server_key, tcp.authority());
            (RemoteEndpoint::Tcp(tcp), address)
        };
        // Reading this `Hello` is what records the pairing on the Server.
        ClientConnection::connect_overview(&endpoint, self.shared.name.clone())
            .map_err(|error| MobileError::Connection(error.to_string()))?;
        let saved = SavedDevice {
            address: address.clone(),
            label,
        };
        let mut state = self.shared.lock();
        state
            .devices
            .insert(address.clone(), Device::new(saved.label.clone()));
        state.mark(Change::Devices);
        let suspended = state.suspended;
        drop(state);
        if !suspended {
            self.shared.connect(&address);
        }
        self.shared.wake.notify_one();
        Ok(saved)
    }

    /// Stops using a Device here; revoking the phone is done on the Device (ADR 0040).
    pub fn forget(&self, address: String) {
        let mut state = self.shared.lock();
        if let Some(mut device) = state.devices.remove(&address) {
            device.disconnect();
        }
        if state
            .shown
            .as_ref()
            .is_some_and(|(shown, _)| *shown == address)
        {
            state.shown = None;
        }
        state.mark(Change::Devices);
        state.mark(Change::NeedsYou);
        drop(state);
        self.shared.wake.notify_one();
    }

    /// The app came to the foreground: connect to every Device, each with a fresh
    /// Bootstrap (ADR 0040).
    pub fn resume(&self) {
        let addresses: Vec<String> = {
            let mut state = self.shared.lock();
            state.suspended = false;
            state.devices.keys().cloned().collect()
        };
        for address in addresses {
            self.shared.connect(&address);
        }
    }

    /// The app is leaving the screen: close every connection and the Peer-to-peer
    /// endpoint now, without waiting for the other side (ADR 0040).
    pub fn suspend(&self) {
        let node = {
            let mut state = self.shared.lock();
            state.suspended = true;
            for device in state.devices.values_mut() {
                device.disconnect();
                device.status = DeviceStatus::Suspended;
            }
            state.mark(Change::Devices);
            state.node.take()
        };
        if let Some(node) = node {
            node.close();
        }
        self.shared.wake.notify_one();
    }

    /// The platform's network monitor saw a change (ADR 0040).
    pub fn network_changed(&self) {
        let node = {
            let state = self.shared.lock();
            for device in state.devices.values() {
                // An answer, or the writer's failure, shows whether the path survived.
                if let Some(server_id) = device.model.server_id {
                    device.send(ClientMessage::Ping {
                        server_id,
                        nonce: 0,
                    });
                }
            }
            state.node.clone()
        };
        if let Some(node) = node {
            node.network_changed();
        }
    }

    /// Ends everything; the object is unusable afterwards.
    pub fn close(&self) {
        self.suspend();
        self.shared.lock().closed = true;
        self.shared.wake.notify_one();
    }

    /// What changed since the last call; `changed` comes again only after this ran.
    pub fn take_changes(&self) -> Vec<Change> {
        let mut state = self.shared.lock();
        state.notified = false;
        std::mem::take(&mut state.changes).into_iter().collect()
    }

    /// The one-time results waiting, oldest first; each is returned once.
    pub fn take_events(&self) -> Vec<Event> {
        self.shared.lock().events.drain(..).collect()
    }

    pub fn devices(&self) -> Vec<DeviceView> {
        let state = self.shared.lock();
        state
            .devices
            .iter()
            .map(|(address, device)| DeviceView {
                address: address.clone(),
                label: device.label.clone(),
                status: device.status.clone(),
            })
            .collect()
    }

    /// Every Blocked Agent on every connected Device (ADR 0024).
    pub fn needs_you(&self) -> Vec<NeedsYouItem> {
        let state = self.shared.lock();
        let mut items = Vec::new();
        for (address, device) in &state.devices {
            if device.status != DeviceStatus::Connected {
                continue;
            }
            for (workspace, pane_id, agent) in device.model.blocked_agents() {
                items.push(NeedsYouItem {
                    address: address.clone(),
                    device: device.label.clone(),
                    workspace: workspace.name().to_owned(),
                    pane: pane_id.as_u64(),
                    agent: agent.kind.label().to_owned(),
                    blocked_on: agent.blocked_on.clone(),
                });
            }
        }
        items
    }

    /// A Device's Workspaces and the Panes in them, Tabs flattened.
    pub fn workspaces(&self, address: String) -> Vec<WorkspaceView> {
        let state = self.shared.lock();
        let Some(device) = state.devices.get(&address) else {
            return Vec::new();
        };
        let model = &device.model;
        let Some(session) = model.session() else {
            return Vec::new();
        };
        session
            .workspaces()
            .iter()
            .map(|workspace| WorkspaceView {
                id: workspace.id().as_u64(),
                name: workspace.name().to_owned(),
                panes: workspace
                    .tabs()
                    .iter()
                    .flat_map(|tab| tab.panes())
                    .map(|pane| {
                        let pane_id = pane.id();
                        PaneView {
                            id: pane_id.as_u64(),
                            title: model.pane_title(pane_id),
                            agent: model.agents.get(&pane_id).map(|agent| AgentView {
                                label: agent.kind.label().to_owned(),
                                status: model
                                    .agent_display_state(pane_id)
                                    .map_or(crate::AgentStatus::Unknown, Into::into),
                                blocked_on: agent.blocked_on.clone(),
                            }),
                            attention: model.attention.contains(&pane_id),
                            exited: model
                                .terminals
                                .get(&pane_id)
                                .is_some_and(|terminal| terminal.exited),
                        }
                    })
                    .collect(),
            })
            .collect()
    }

    pub fn terminal(&self, address: String, pane: u64) -> Option<TerminalScreen> {
        let state = self.shared.lock();
        let terminal = state
            .devices
            .get(&address)?
            .model
            .terminals
            .get(&PaneId::from_u64(pane))?;
        Some(screen(&terminal.view))
    }

    /// The Pane now on screen at `rows` × `columns`, or none: the phone reports focus and
    /// sizes the terminal like a window (ADR 0041), and gives both back when it leaves.
    /// Call it again when the soft keyboard or the orientation settles on a new size.
    pub fn show_pane(&self, address: Option<String>, pane: u64, rows: u16, columns: u16) {
        let mut state = self.shared.lock();
        let next = address.map(|address| (address, PaneId::from_u64(pane)));
        if let Some((address, pane_id)) = state.shown.take()
            && next.as_ref() != Some(&(address.clone(), pane_id))
            && let Some(device) = state.devices.get(&address)
        {
            device.terminal(pane_id, TerminalCommand::Focus(false));
        }
        if let Some((address, pane_id)) = &next
            && let Some(device) = state.devices.get_mut(address)
        {
            device.terminal(*pane_id, TerminalCommand::Focus(true));
            if rows > 0 && columns > 0 {
                device.terminal(
                    *pane_id,
                    TerminalCommand::Resize(TerminalSize::new(rows, columns)),
                );
            }
            device.model.mark_seen(*pane_id);
            let address = address.clone();
            state.mark_device(&address);
        }
        state.shown = next;
        drop(state);
        self.shared.wake.notify_one();
    }

    pub fn send_text(&self, address: String, pane: u64, text: String) {
        self.shared
            .command(&address, pane, TerminalCommand::Text(text));
    }

    /// A key from the row above the soft keyboard, with its sticky Ctrl (ADR 0041).
    pub fn send_key(&self, address: String, pane: u64, key: Key, control: bool) {
        let key = match key {
            Key::Enter => TerminalKey::Enter,
            Key::Escape => TerminalKey::Escape,
            Key::Tab => TerminalKey::Tab,
            Key::Backspace => TerminalKey::Backspace,
            Key::Up => TerminalKey::Up,
            Key::Down => TerminalKey::Down,
            Key::Left => TerminalKey::Left,
            Key::Right => TerminalKey::Right,
            Key::Character { text } => TerminalKey::Character(text),
        };
        self.shared.command(
            &address,
            pane,
            TerminalCommand::Key {
                key,
                modifiers: TerminalModifiers {
                    control,
                    ..TerminalModifiers::default()
                },
                kind: TerminalKeyEventKind::Press,
            },
        );
    }

    /// Scrolls the Pane for every viewer, as the desktop does (ADR 0036, ADR 0041).
    pub fn scroll(&self, address: String, pane: u64, lines: i32) {
        self.shared.command(
            &address,
            pane,
            TerminalCommand::Scroll(TerminalScroll::Lines(lines)),
        );
    }
}

impl Drop for Companion {
    fn drop(&mut self) {
        self.close();
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Calls `changed` once per `take_changes`, however many changes arrive between,
    /// never with the lock held (ADR 0039).
    fn notify_loop(&self) {
        loop {
            let mut state = self.lock();
            while !state.wake_notifier && !state.closed {
                state = self
                    .wake
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            if state.closed {
                return;
            }
            state.wake_notifier = false;
            drop(state);
            self.listener.changed();
        }
    }

    /// This phone's Peer-to-peer endpoint, which only dials (ADR 0040); it binds on the
    /// first dial.
    fn node(&self) -> Arc<P2pNode> {
        let mut state = self.lock();
        Arc::clone(state.node.get_or_insert_with(|| {
            P2pNode::new(
                self.runtime.handle().clone(),
                Arc::new(self.key.clone()),
                Network::Condr,
                ProxySetting::default(),
                None,
            )
        }))
    }

    fn command(&self, address: &str, pane: u64, command: TerminalCommand) {
        if let Some(device) = self.lock().devices.get(address) {
            device.terminal(PaneId::from_u64(pane), command);
        }
    }

    fn connect(self: &Arc<Self>, address: &str) {
        let endpoint = if address.starts_with("p2p://") {
            P2pEndpoint::parse(address).map(|p2p| RemoteEndpoint::P2p {
                node: self.node(),
                address: p2p,
            })
        } else {
            TcpEndpoint::parse(address, self.key.clone()).map(RemoteEndpoint::Tcp)
        };
        let mut state = self.lock();
        state.forget_events_of(address);
        let Some(device) = state.devices.get_mut(address) else {
            return;
        };
        device.disconnect();
        let endpoint = match endpoint {
            Ok(endpoint) => endpoint,
            Err(error) => {
                device.status = DeviceStatus::Failed {
                    reason: error.to_string(),
                };
                state.mark(Change::Devices);
                drop(state);
                self.wake.notify_one();
                return;
            }
        };
        device.cancellation = Cancellation::default();
        device.status = DeviceStatus::Connecting;
        let generation = device.generation;
        let cancellation = device.cancellation.clone();
        state.mark(Change::Devices);
        drop(state);
        self.wake.notify_one();
        let shared = Arc::clone(self);
        let address = address.to_owned();
        let spawned = thread::Builder::new()
            .name("condr-connect".into())
            .spawn(move || {
                let result = ClientConnection::connect_cancellable(
                    &endpoint,
                    shared.name.clone(),
                    cancellation,
                );
                shared.connected(&address, generation, result);
            });
        if let Err(error) = spawned {
            tracing::warn!("could not start a connection: {error}");
        }
    }

    fn connected(
        self: &Arc<Self>,
        address: &str,
        generation: u64,
        result: std::io::Result<ClientConnection<RemoteStream>>,
    ) {
        let mut state = self.lock();
        let Some(device) = state.devices.get_mut(address) else {
            return;
        };
        if device.generation != generation {
            return;
        }
        let started = result.and_then(|connection| {
            let bootstrap = connection
                .bootstrap()
                .cloned()
                .ok_or_else(|| std::io::Error::other("the device sent no Bootstrap"))?;
            device
                .model
                .apply_bootstrap(bootstrap)
                .map_err(std::io::Error::other)?;
            let (Some(server_id), Some(session_id)) =
                (device.model.server_id, device.model.session_id)
            else {
                return Err(std::io::Error::other("the Bootstrap named no Session"));
            };
            let (sender, incoming) = mpsc::sync_channel(INCOMING_CAPACITY);
            let io = ConnectionIo::start(
                connection.into_stream(),
                server_id,
                session_id,
                move |message| sender.send(message).map_err(drop),
                |_| Ok(true),
                |_| Ok(()),
            )?;
            Ok((io, incoming))
        });
        match started {
            Ok((io, incoming)) => {
                let visual = Arc::clone(&io.visual);
                device.io = Some(io);
                device.status = DeviceStatus::Connected;
                device.subscribe();
                let shared = Arc::clone(self);
                let address = address.to_owned();
                let spawned = thread::Builder::new()
                    .name("condr-pump".into())
                    .spawn(move || {
                        for message in incoming {
                            shared.apply(&address, generation, &visual, message);
                        }
                    });
                if let Err(error) = spawned {
                    tracing::warn!("could not start applying a connection: {error}");
                }
            }
            Err(error) => {
                device.status = DeviceStatus::Failed {
                    reason: error.to_string(),
                };
            }
        }
        state.mark(Change::Devices);
        state.mark_device(address);
        drop(state);
        self.wake.notify_one();
    }

    /// Applies what one connection's reader handed on, under the one lock.
    fn apply(
        &self,
        address: &str,
        generation: u64,
        visual: &TerminalVisualSlot,
        message: Incoming,
    ) {
        let mut state = self.lock();
        let shown = state
            .shown
            .as_ref()
            .filter(|(shown, _)| shown == address)
            .map(|(_, pane_id)| *pane_id);
        let Some(device) = state.devices.get_mut(address) else {
            return;
        };
        if device.generation != generation {
            return;
        }
        let mut changes = Vec::new();
        let mut events = Vec::new();
        match message {
            Incoming::Bootstrap(bootstrap) => {
                match device.model.apply_bootstrap(bootstrap) {
                    Ok(change) => {
                        device.snapshot_pending = false;
                        if change.authority_changed {
                            device.subscription = Subscription::None;
                        }
                        device.subscribe();
                        changes.push(Change::Device {
                            address: address.to_owned(),
                        });
                        changes.extend(device.model.terminals.keys().map(|pane_id| {
                            Change::Terminal {
                                address: address.to_owned(),
                                pane: pane_id.as_u64(),
                            }
                        }));
                    }
                    Err(reason) => {
                        device.disconnect();
                        device.status = DeviceStatus::Failed { reason };
                        changes.push(Change::Devices);
                    }
                }
            }
            Incoming::VisualReady(frame_generation) => {
                if let Some(batch) = visual.take(frame_generation)
                    && device.model.server_id == Some(batch.server_id)
                    && device.model.session_id == Some(batch.session_id)
                {
                    match device.model.apply_terminal_frames(batch.panes) {
                        Ok(panes) => {
                            changes.extend(panes.into_iter().map(|pane_id| Change::Terminal {
                                address: address.to_owned(),
                                pane: pane_id.as_u64(),
                            }))
                        }
                        Err(()) => device.request_snapshot(),
                    }
                }
            }
            Incoming::TerminalResync => device.request_snapshot(),
            Incoming::Disconnected(reason) => {
                device.disconnect();
                device.status = DeviceStatus::Failed { reason };
                changes.push(Change::Devices);
            }
            Incoming::Message(message) => {
                apply_message(device, address, shown, message, &mut changes, &mut events);
            }
        }
        let touched = !changes.is_empty();
        for change in changes {
            state.mark(change);
        }
        if touched {
            state.mark_device(address);
        }
        for event in events {
            state.push_event(event);
        }
        drop(state);
        self.wake.notify_one();
    }
}

fn apply_message(
    device: &mut Device,
    address: &str,
    shown: Option<PaneId>,
    message: ServerMessage,
    changes: &mut Vec<Change>,
    events: &mut Vec<Event>,
) {
    let terminal_change = |pane_id: PaneId| Change::Terminal {
        address: address.to_owned(),
        pane: pane_id.as_u64(),
    };
    let device_change = || Change::Device {
        address: address.to_owned(),
    };
    match message {
        ServerMessage::Event {
            server_id,
            session_id,
            sequence,
            event,
        } => {
            let model = &mut device.model;
            if model.server_id != Some(server_id) || model.session_id != Some(session_id) {
                return;
            }
            if sequence != model.sequence.saturating_add(1) {
                if sequence > model.sequence {
                    device.subscription = Subscription::None;
                    device.request_snapshot();
                }
                return;
            }
            // A malformed layout must not advance the cursor past it.
            if !matches!(event, SessionEvent::LayoutChanged { .. }) {
                model.sequence = sequence;
            }
            match event {
                SessionEvent::LayoutChanged {
                    snapshot,
                    zoomed_panes,
                } => match model.apply_layout(snapshot, zoomed_panes) {
                    Ok(_) => {
                        model.sequence = sequence;
                        changes.push(device_change());
                    }
                    Err(_) => {
                        device.subscription = Subscription::None;
                        device.request_snapshot();
                    }
                },
                SessionEvent::AgentChanged { pane_id, agent } => {
                    model.apply_agent(pane_id, agent, shown == Some(pane_id));
                    changes.push(device_change());
                }
                SessionEvent::TerminalExited { pane_id } => {
                    if let Some(terminal) = model.terminals.get_mut(&pane_id) {
                        terminal.exited = true;
                    }
                    changes.push(device_change());
                    changes.push(terminal_change(pane_id));
                }
                SessionEvent::TerminalTitleChanged { pane_id, title } => {
                    match title {
                        Some(title) => model.terminal_titles.insert(pane_id, title),
                        None => model.terminal_titles.remove(&pane_id),
                    };
                    changes.push(device_change());
                }
                SessionEvent::TerminalAttentionChanged { pane_id, attention } => {
                    if attention {
                        model.attention.insert(pane_id);
                    } else {
                        model.attention.remove(&pane_id);
                    }
                    changes.push(device_change());
                }
                SessionEvent::WorkspaceGitChanged { workspace_id, git } => match git {
                    Some(git) => {
                        model.workspace_git.insert(workspace_id, git);
                    }
                    None => {
                        model.workspace_git.remove(&workspace_id);
                    }
                },
                SessionEvent::ServerSettingsChanged { settings } => model.settings = settings,
                // The phone's view is its own and it shows one screen at a time (ADR 0041).
                SessionEvent::Activated { .. }
                | SessionEvent::WorkspaceFilesChanged { .. }
                | SessionEvent::Omitted => {}
            }
        }
        ServerMessage::Subscribed { .. } => device.subscription = Subscription::Active,
        ServerMessage::SubscriptionRejected { .. } | ServerMessage::SnapshotRejected { .. } => {
            device.subscription = Subscription::None;
            device.snapshot_pending = false;
            device.request_snapshot();
        }
        ServerMessage::TerminalClipboard { pane_id, text } if shown == Some(pane_id) => {
            events.push(Event::Clipboard { text });
        }
        ServerMessage::TerminalCopied {
            text: Some(text), ..
        } => events.push(Event::Copied { text }),
        ServerMessage::Error { message } => events.push(Event::Error {
            address: address.to_owned(),
            message,
        }),
        ServerMessage::ServerStopping => {
            device.disconnect();
            device.status = DeviceStatus::Failed {
                reason: "the device's Server is stopping".into(),
            };
            changes.push(Change::Devices);
        }
        // A message from a newer protocol: the Bootstrap carries what it would have said
        // (ADR 0028).
        ServerMessage::Unknown(_) => device.request_snapshot(),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_notify_once_until_the_app_takes_them() {
        let mut state = State::default();
        state.mark(Change::Devices);
        assert!(state.wake_notifier, "the first change notifies");
        state.wake_notifier = false;
        state.mark(Change::NeedsYou);
        state.mark(Change::Devices);
        assert!(!state.wake_notifier, "later ones wait for the take");
        // What `take_changes` does.
        state.notified = false;
        assert_eq!(
            std::mem::take(&mut state.changes)
                .into_iter()
                .collect::<Vec<_>>(),
            [Change::Devices, Change::NeedsYou]
        );
        state.mark(Change::Devices);
        assert!(
            state.wake_notifier,
            "a change after the take notifies again"
        );
    }

    #[test]
    fn events_come_once_in_order_with_only_the_newest_clipboard_and_none_across_a_reconnect() {
        let error = |address: &str, message: &str| Event::Error {
            address: address.into(),
            message: message.into(),
        };
        let mut state = State::default();
        state.push_event(error("a", "one"));
        state.push_event(Event::Clipboard { text: "old".into() });
        state.push_event(error("a", "two"));
        state.push_event(Event::Clipboard { text: "new".into() });
        assert_eq!(
            state.events.drain(..).collect::<Vec<_>>(),
            [
                error("a", "one"),
                error("a", "two"),
                Event::Clipboard { text: "new".into() }
            ]
        );
        assert!(state.events.is_empty(), "each is delivered once");

        state.push_event(error("a", "lost"));
        state.push_event(error("b", "kept"));
        state.push_event(Event::Copied {
            text: "copy".into(),
        });
        state.forget_events_of("a");
        assert_eq!(
            state.events.drain(..).collect::<Vec<_>>(),
            [error("b", "kept")]
        );
    }
}
