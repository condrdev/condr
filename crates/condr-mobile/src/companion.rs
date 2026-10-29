//! Every paired Device's connection and model, and the notifier that tells the app (ADR
//! 0039, ADR 0040). One lock guards the whole state: the app reads it on its main thread,
//! each connection's pump writes it, and nothing slow runs under it.

use crate::views::{
    AgentView, Change, DeviceStatus, DeviceView, Event, Key, NeedsYouItem, PaneView, SavedDevice,
    TerminalScreen, WorkspaceView, screen,
};
use crate::{Listener, MobileError, input};
use condr_client::frames::TerminalVisualSlot;
use condr_client::p2p::Network;
use condr_client::{
    Cancellation, ClientConnection, ConnectionIo, DeviceKey, Incoming, P2pEndpoint, P2pNode,
    RemoteEndpoint, RemoteStream, SessionModel, TcpEndpoint,
};
use condr_core::protocol::{ClientMessage, Hello, ServerMessage, SessionEvent};
use condr_core::{
    PaneId, ProxySetting, TerminalCommand, TerminalPosition, TerminalScroll, TerminalSelectionUnit,
    TerminalSide, TerminalSize,
};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// The name this phone goes by to a Server; a Server learns it at pairing (ADR 0040).
    name: Mutex<String>,
    /// What every `Hello` says as `push` (ADR 0047): one flag for the whole app.
    push: AtomicBool,
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
    /// The grid the shown Pane last measured, sent again whenever the phone takes the
    /// size back: after a reconnect, or when the app is active again.
    shown_size: Option<TerminalSize>,
    /// The app is on screen but not active, under the app switcher, a call or Control
    /// Center: the shown Pane's focus and size are given back until it is (ADR 0041).
    inactive: bool,
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

    /// The phone takes the Pane, as a window that comes to the front does (ADR 0036,
    /// ADR 0041): focus, then the grid it measured, if it has measured one.
    fn take_terminal(&self, pane_id: PaneId, size: Option<TerminalSize>) {
        self.terminal(pane_id, TerminalCommand::Focus(true));
        if let Some(size) = size {
            self.terminal(pane_id, TerminalCommand::Resize(size));
        }
    }

    /// Frames only for `panes` from now on (ADR 0041): nothing when the phone shows no
    /// Pane of this Device, the one Pane when it does. Once disconnected, the next
    /// connection starts from the set kept here.
    fn watch(&mut self, panes: HashSet<PaneId>) {
        if self.model.watched() == Some(&panes) {
            return;
        }
        self.model.watch_terminals(panes.clone());
        self.send(ClientMessage::WatchTerminals { panes });
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
            name: Mutex::new(name),
            push: AtomicBool::new(false),
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
    pub fn pair(&self, link: String, label: Option<String>) -> Result<SavedDevice, MobileError> {
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
        let connection = ClientConnection::connect_overview(&endpoint, self.shared.hello())
            .map_err(|error| MobileError::Connection(error.to_string()))?;
        // The Device is listed by the Server's machine name unless the app names it; a
        // Server too old to say its name leaves the link's host or id.
        let label = label
            .filter(|label| !label.trim().is_empty())
            .or_else(|| {
                Some(connection.server_device_name().to_owned()).filter(|name| !name.is_empty())
            })
            .unwrap_or_else(|| device_label_fallback(&address));
        drop(connection);
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

    /// The name this phone goes by from now on (ADR 0040): a Server learns it when the
    /// phone pairs and lists the phone by it under Paired devices.
    pub fn set_name(&self, name: String) {
        let name = name.trim().to_owned();
        if !name.is_empty() {
            *self.shared.name.lock().expect("name lock poisoned") = name;
        }
    }

    /// Whether this phone takes pushes through Condr Cloud (ADR 0047): the app says true
    /// once it has notification permission and its registration succeeded. When the
    /// answer changes, every open connection is made again so each Server learns it now
    /// rather than on the next connection.
    pub fn set_push(&self, push: bool) {
        if self.shared.push.swap(push, Ordering::Relaxed) == push {
            return;
        }
        let addresses: Vec<String> = self
            .shared
            .lock()
            .devices
            .iter()
            .filter(|(_, device)| {
                matches!(
                    device.status,
                    DeviceStatus::Connected | DeviceStatus::Connecting
                )
            })
            .map(|(address, _)| address.clone())
            .collect();
        for address in addresses {
            self.shared.connect(&address);
        }
    }

    /// A token for Condr Cloud signed with this phone's key, good for five minutes (ADR
    /// 0047). The app sends it as `Authorization: Condr <token>` to [`crate::cloud_url`]
    /// and never logs it.
    pub fn cloud_token(&self) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        condr_client::cloud::token(&self.shared.key, condr_client::cloud::DEFAULT_URL, now)
    }

    /// The name this phone shows a Device under from now on; a blank one changes
    /// nothing. The app keeps it with the Device (ADR 0040).
    pub fn rename(&self, address: String, label: String) {
        let label = label.trim();
        if label.is_empty() {
            return;
        }
        let mut state = self.shared.lock();
        let Some(device) = state.devices.get_mut(&address) else {
            return;
        };
        if device.label == label {
            return;
        }
        device.label = label.to_owned();
        state.mark(Change::Devices);
        // "Needs you" names the Device beside each Agent.
        state.mark(Change::NeedsYou);
        drop(state);
        self.shared.wake.notify_one();
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
        // A Device still connected, or still being reached, is left alone: returning from
        // an inactive moment, or retrying one Device, costs the others nothing.
        let addresses: Vec<String> = {
            let mut state = self.shared.lock();
            state.suspended = false;
            state
                .devices
                .iter()
                .filter(|(_, device)| {
                    !matches!(
                        device.status,
                        DeviceStatus::Connected | DeviceStatus::Connecting
                    )
                })
                .map(|(address, _)| address.clone())
                .collect()
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

    /// Ends everything; the object is unusable afterwards. Named for what uniffi's Kotlin
    /// gives every object already: a `close()` of its own (ADR 0048).
    pub fn shutdown(&self) {
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
                device_id: self.shared.device_id(address),
                label: device.label.clone(),
                status: device.status.clone(),
            })
            .collect()
    }

    /// Every Blocked or Error Agent on every connected Device (ADR 0024, ADR 0044).
    pub fn needs_you(&self) -> Vec<NeedsYouItem> {
        let state = self.shared.lock();
        let mut items = Vec::new();
        for (address, device) in &state.devices {
            if device.status != DeviceStatus::Connected {
                continue;
            }
            for (workspace, pane_id, agent) in device.model.agents_needing_you() {
                items.push(NeedsYouItem {
                    address: address.clone(),
                    device: device.label.clone(),
                    workspace: workspace.name().to_owned(),
                    pane: pane_id.as_u64(),
                    kind: agent.kind.spec().id.to_owned(),
                    agent: agent.kind.label().to_owned(),
                    detail: agent.detail.clone(),
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
                root: workspace.root_directory().display().to_string(),
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
                                kind: agent.kind.spec().id.to_owned(),
                                label: agent.kind.label().to_owned(),
                                status: model
                                    .agent_display_state(pane_id)
                                    .map_or(crate::AgentStatus::Unknown, Into::into),
                                detail: agent.detail.clone(),
                            }),
                            attention: model.attention.contains(&pane_id),
                            exited: model.exited.contains(&pane_id),
                        }
                    })
                    .collect(),
            })
            .collect()
    }

    /// The Pane's screen, once its view has arrived; only the Pane on screen gets one
    /// (ADR 0041).
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
        let size = (rows > 0 && columns > 0).then(|| TerminalSize::new(rows, columns));
        if state.shown != next {
            state.shown_size = None;
        }
        state.shown_size = size.or(state.shown_size);
        let (inactive, shown_size) = (state.inactive, state.shown_size);
        if let Some((address, pane_id)) = state.shown.take()
            && next.as_ref() != Some(&(address.clone(), pane_id))
            && let Some(device) = state.devices.get_mut(&address)
        {
            device.terminal(pane_id, TerminalCommand::Focus(false));
            device.watch(HashSet::new());
        }
        if let Some((address, pane_id)) = &next
            && let Some(device) = state.devices.get_mut(address)
        {
            device.watch(HashSet::from([*pane_id]));
            // An inactive app keeps the measure for when it is active again.
            if !inactive {
                device.take_terminal(*pane_id, shown_size);
            }
            device.model.mark_seen(*pane_id);
            let address = address.clone();
            state.mark_device(&address);
        }
        state.shown = next;
        drop(state);
        self.shared.wake.notify_one();
    }

    /// The app became active, or inactive while still on screen: under the app switcher,
    /// a call or Control Center. An inactive app gives the shown Pane's focus and size
    /// back, and takes both again once it is active (ADR 0041).
    pub fn set_active(&self, active: bool) {
        let mut state = self.shared.lock();
        if state.inactive != active {
            return;
        }
        state.inactive = !active;
        if let Some((address, pane_id)) = &state.shown
            && let Some(device) = state.devices.get(address)
        {
            if active {
                device.take_terminal(*pane_id, state.shown_size);
            } else {
                device.terminal(*pane_id, TerminalCommand::Focus(false));
            }
        }
    }

    pub fn send_text(&self, address: String, pane: u64, text: String) {
        self.shared
            .command(&address, pane, TerminalCommand::Text(text));
    }

    /// Pasted text, which the Server brackets when the program asked, so a pasted line
    /// break does not submit what is half typed (ADR 0041).
    pub fn paste(&self, address: String, pane: u64, text: String) {
        self.shared
            .command(&address, pane, TerminalCommand::Paste(text));
    }

    /// The field's draft: a paste, then Return once the program has taken it, as the
    /// Server submits an agent prompt. A Return sent right after plain text is absorbed
    /// by a TUI's paste detector (Codex), leaving the draft in its composer (ADR 0041).
    pub fn submit(&self, address: String, pane: u64, text: String) {
        self.shared
            .command(&address, pane, TerminalCommand::Submit(text));
    }

    /// A key from the row above the soft keyboard, with its sticky Ctrl (ADR 0041).
    pub fn send_key(&self, address: String, pane: u64, key: Key, control: bool) {
        self.shared
            .command(&address, pane, input::key(key, control));
    }

    /// A swipe of `lines` over the cell at `row`, `column`, positive towards what came
    /// before: wheel motion, which the Server gives to a program that tracks the mouse,
    /// turns into arrow keys on an alternate screen that asks, or uses to scroll the
    /// Pane for every viewer (ADR 0036, ADR 0041).
    pub fn wheel(&self, address: String, pane: u64, row: u16, column: u16, lines: i32) {
        if let Some(command) = input::wheel(row, column, lines) {
            self.shared.command(&address, pane, command);
        }
    }

    /// A tap where the program tracks the mouse: a click on the cell (ADR 0041).
    pub fn click(&self, address: String, pane: u64, row: u16, column: u16) {
        for command in input::click(row, column) {
            self.shared.command(&address, pane, command);
        }
    }

    /// A long press: the word around the cell becomes the Pane's selection, the one every
    /// viewer shares (ADR 0008, ADR 0041).
    pub fn select_word(&self, address: String, pane: u64, row: u16, column: u16) {
        let display_offset = self.shared.display_offset(&address, pane);
        self.shared.command(
            &address,
            pane,
            TerminalCommand::SelectAt {
                position: TerminalPosition {
                    row,
                    column,
                    side: TerminalSide::Left,
                },
                display_offset,
                unit: TerminalSelectionUnit::Word,
            },
        );
    }

    /// A drag after a long press: the cells from one to the other become the selection.
    pub fn select_range(
        &self,
        address: String,
        pane: u64,
        from_row: u16,
        from_column: u16,
        to_row: u16,
        to_column: u16,
    ) {
        let display_offset = self.shared.display_offset(&address, pane);
        self.shared.command(
            &address,
            pane,
            TerminalCommand::Select(Some(input::range(
                (from_row, from_column),
                (to_row, to_column),
                display_offset,
            ))),
        );
    }

    pub fn clear_selection(&self, address: String, pane: u64) {
        self.shared
            .command(&address, pane, TerminalCommand::Select(None));
    }

    /// Copies the Pane's selection; its text comes back as [`Event::Copied`].
    pub fn copy_selection(&self, address: String, pane: u64) {
        self.shared
            .command(&address, pane, TerminalCommand::Copy { selection: None });
    }

    /// Back to the newest output, for every viewer (ADR 0036).
    pub fn scroll_to_bottom(&self, address: String, pane: u64) {
        self.shared.command(
            &address,
            pane,
            TerminalCommand::Scroll(TerminalScroll::Bottom),
        );
    }
}

impl Drop for Companion {
    fn drop(&mut self) {
        self.shutdown();
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

    /// How far the Pane's viewport sits above the newest output, as the phone last saw it;
    /// a selection names its cells against it.
    fn display_offset(&self, address: &str, pane: u64) -> u32 {
        self.lock()
            .devices
            .get(address)
            .and_then(|device| device.model.terminals.get(&PaneId::from_u64(pane)))
            .map_or(0, |terminal| terminal.view.display_offset)
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
        let shown = state
            .shown
            .as_ref()
            .filter(|(shown, _)| shown == address)
            .map(|(_, pane_id)| *pane_id);
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
        // Outside a Pane the phone takes only reliable events (ADR 0041); a reconnect while
        // a Pane of this Device is on screen keeps that Pane's frames.
        let watched: HashSet<PaneId> = shown.into_iter().collect();
        device.model.watch_terminals(watched.clone());
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
                let result = ClientConnection::connect_watching(
                    &endpoint,
                    shared.hello(),
                    watched.clone(),
                    cancellation,
                );
                shared.connected(&address, generation, &watched, result);
            });
        if let Err(error) = spawned {
            tracing::warn!("could not start a connection: {error}");
        }
    }

    fn connected(
        self: &Arc<Self>,
        address: &str,
        generation: u64,
        watched: &HashSet<PaneId>,
        result: std::io::Result<ClientConnection<RemoteStream>>,
    ) {
        let mut state = self.lock();
        // The Pane on screen when this Device's connection was made, if the app is active.
        let reclaim = state
            .shown
            .as_ref()
            .filter(|(shown, _)| shown == address && !state.inactive)
            .map(|(_, pane_id)| (*pane_id, state.shown_size));
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
                // The phone opened or left a Pane while this connection was being made.
                if let Some(panes) = device.model.watched().filter(|panes| *panes != watched) {
                    let panes = panes.clone();
                    device.send(ClientMessage::WatchTerminals { panes });
                }
                device.subscribe();
                // The Server gave the shown Pane's focus and size back when the old
                // connection ended; the phone takes them again (ADR 0041).
                if let Some((pane_id, size)) = reclaim {
                    device.take_terminal(pane_id, size);
                }
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
                // A Server that no longer knows this phone refuses it: over TCP in the
                // Noise handshake, otherwise with `NotAuthorized` or `DeviceRevoked`, all
                // `PermissionDenied` (ADR 0040). The app offers pairing again, not a retry.
                device.status = if error.kind() == std::io::ErrorKind::PermissionDenied {
                    DeviceStatus::Revoked
                } else {
                    DeviceStatus::Failed {
                        reason: error.to_string(),
                    }
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
                        // The Pane on screen is being looked at: its Done is seen.
                        if let Some(pane_id) = shown {
                            device.model.mark_seen(pane_id);
                        }
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
                    model.apply_agent(pane_id, agent);
                    changes.push(device_change());
                }
                SessionEvent::TerminalExited { pane_id } => {
                    model.exited.insert(pane_id);
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
                // A program's own notification reaches the desktop only (ADR 0054).
                SessionEvent::TerminalNotification { .. }
                | SessionEvent::CommandFinished { .. } => {}
                // Kept for when the apps draw it (ADR 0054).
                SessionEvent::TerminalProgressChanged { pane_id, progress } => {
                    model.apply_progress(pane_id, progress);
                }
                SessionEvent::TerminalAttentionChanged { pane_id, attention } => {
                    if attention {
                        model.attention.insert(pane_id);
                    } else {
                        model.attention.remove(&pane_id);
                    }
                    changes.push(device_change());
                }
                SessionEvent::AgentUnseenChanged { pane_id, unseen } => {
                    model.apply_agent_unseen(pane_id, unseen, shown == Some(pane_id));
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

impl Shared {
    /// The Device id `address` names: the Peer-to-peer id or the TCP Server's key.
    fn device_id(&self, address: &str) -> Option<String> {
        let id = if address.starts_with("p2p://") {
            P2pEndpoint::parse(address).ok()?.device
        } else {
            TcpEndpoint::parse(address, self.key.clone())
                .ok()?
                .server_key
        };
        Some(id.to_string())
    }

    /// What this phone says in `Hello`: its name and whether it takes pushes.
    fn hello(&self) -> Hello {
        Hello {
            push: self.push.load(Ordering::Relaxed),
            ..Hello::new(self.name.lock().expect("name lock poisoned").clone())
        }
    }
}

/// What to call a Device whose Server did not say: a TCP link's host, a Peer-to-peer
/// link's id cut short.
fn device_label_fallback(address: &str) -> String {
    if let Some(rest) = address.strip_prefix("tcp://") {
        let authority = rest.rsplit('@').next().unwrap_or(rest);
        let host = authority
            .rsplit_once(':')
            .map_or(authority, |(host, _)| host);
        host.trim_matches(|c| c == '[' || c == ']').to_owned()
    } else {
        address
            .strip_prefix("p2p://")
            .unwrap_or(address)
            .chars()
            .take(8)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_device_without_a_name_from_its_server_is_called_by_its_link() {
        assert_eq!(
            device_label_fallback("tcp://key@studio.local:7417"),
            "studio.local"
        );
        assert_eq!(device_label_fallback("tcp://key@[::1]:7417"), "::1");
        assert_eq!(device_label_fallback("p2p://h7k2x9q4m3v8w1n6"), "h7k2x9q4");
    }

    use super::*;

    /// OSC 52 copies reach every subscribed Client (ADR 0007); the phone applies only
    /// those from the Pane it shows (ADR 0041).
    #[test]
    fn only_the_pane_on_screen_reaches_the_clipboard() {
        let mut device = Device::new("desk".into());
        let (shown, other) = (PaneId::from_u64(3), PaneId::from_u64(4));
        let mut changes = Vec::new();
        let mut events = Vec::new();
        for (pane_id, text) in [(other, "elsewhere"), (shown, "here")] {
            apply_message(
                &mut device,
                "tcp://desk",
                Some(shown),
                ServerMessage::TerminalClipboard {
                    pane_id,
                    text: text.into(),
                },
                &mut changes,
                &mut events,
            );
        }
        assert_eq!(
            events,
            [Event::Clipboard {
                text: "here".into()
            }]
        );
    }

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
