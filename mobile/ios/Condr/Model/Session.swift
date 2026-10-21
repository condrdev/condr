import CondrKit
import Observation
import Synchronization
import UIKit

/// A Pane on a Device, as the lists and routes name one.
nonisolated struct PaneRef: Hashable, Sendable {
    let address: String
    let pane: UInt64
}

/// One Agent with where it runs, for the lists.
struct AgentItem: Identifiable {
    var id: PaneRef { ref }
    let ref: PaneRef
    /// The Device's name.
    let device: String
    let workspace: String
    let pane: PaneView
    let agent: AgentView
}

/// What the library tells the app: the paired Devices, their Workspaces and Agents, the
/// "Needs you" list, and the screen of the one Pane on display (ADR 0039, ADR 0041). It
/// re-reads what `changed` names and turns what it saw change into banners.
@Observable
@MainActor
final class Session {
    private(set) var devices: [DeviceView] = []
    private(set) var workspaces: [String: [WorkspaceView]] = [:]
    private(set) var needsYou: [NeedsYouItem] = []
    /// The Pane on screen: the one the phone watches and sizes (ADR 0041).
    private(set) var shown: PaneRef?
    /// The shown Pane's screen, once its first full frame arrived.
    private(set) var screen: TerminalScreen?
    /// This phone's key, as a Device's Paired devices shows it.
    let fingerprint: String

    private let client: any CompanionProtocol
    private let saved: SavedDeviceStore
    private let alerts: Alerts
    private let router: Router
    /// Every Agent's last seen status, to notice the changes worth a banner.
    private var statuses: [PaneRef: AgentStatus] = [:]

    init(
        client: any CompanionProtocol, fingerprint: String, saved: SavedDeviceStore,
        alerts: Alerts, router: Router
    ) {
        self.client = client
        self.fingerprint = fingerprint
        self.saved = saved
        self.alerts = alerts
        self.router = router
        devices = client.devices()
        for device in devices {
            workspaces[device.address] = client.workspaces(address: device.address)
        }
        needsYou = client.needsYou()
        statuses = Dictionary(uniqueKeysWithValues: agents.map { ($0.ref, $0.agent.status) })
    }

    /// At least one Device is paired, so the app shows its tabs rather than the welcome.
    var paired: Bool { !devices.isEmpty }

    // MARK: Hearing the library

    /// The library said something changed: read what, on the main thread.
    func refresh() {
        for change in client.takeChanges() {
            switch change {
            case .devices:
                devices = client.devices()
                for device in devices where workspaces[device.address] == nil {
                    workspaces[device.address] = client.workspaces(address: device.address)
                }
                workspaces = workspaces.filter { address, _ in
                    devices.contains { $0.address == address }
                }
            case .needsYou:
                needsYou = client.needsYou()
            case .device(let address):
                workspaces[address] = client.workspaces(address: address)
            case .terminal(let address, let pane):
                if shown == PaneRef(address: address, pane: pane) {
                    screen = client.terminal(address: address, pane: pane)
                }
            }
        }
        for event in client.takeEvents() {
            switch event {
            case .error(_, let message):
                alerts.report(message)
            case .copied(let text), .clipboard(let text):
                UIPasteboard.general.string = text
                alerts.notify("Copied to clipboard")
            }
        }
        noticeChanges()
    }

    /// An Agent elsewhere that came to need you or finished since the last look earns a
    /// banner (ADR 0024); one first seen in that state does not, the list shows it.
    private func noticeChanges() {
        var seen: [PaneRef: AgentStatus] = [:]
        for item in agents {
            seen[item.ref] = item.agent.status
            guard let before = statuses[item.ref], before != item.agent.status,
                item.agent.status.needsYou || item.agent.status == .done,
                shown != item.ref
            else { continue }
            let title =
                switch item.agent.status {
                case .blocked: "\(item.agent.label) needs you"
                case .error: "\(item.agent.label) failed"
                default: "\(item.agent.label) is done"
                }
            alerts.raise(
                Banner(
                    ref: item.ref, status: item.agent.status, kind: item.agent.kind,
                    title: title,
                    detail:
                        "\(item.pane.name(fallback: item.agent.label)) · \(place(item.workspace, item.device))",
                    quiet: router.showsHome))
        }
        statuses = seen
    }

    // MARK: Lifecycle (ADR 0040)

    /// The app came to the foreground: connect to every Device, each with a fresh Bootstrap.
    func resume() {
        client.resume()
    }

    /// The app is leaving the screen: close every connection now, off the main thread,
    /// and return once they are closed, for the background task that covers it.
    func suspend() async {
        let client = client
        await Task.detached { client.suspend() }.value
    }

    func networkChanged() {
        client.networkChanged()
    }

    /// The app is on screen but not active, under Control Center, a call or the app
    /// switcher: the shown Pane's focus and size go back until it is active again, and
    /// are taken back then (ADR 0041).
    func setActive(_ active: Bool) {
        client.setActive(active: active)
    }

    /// The phone's new name (ADR 0040). A Server takes the name each `Hello` carries, so
    /// every connection is made again now and each computer lists the phone by it at once.
    func rename(_ name: String) async {
        client.setName(name: name)
        await suspend()
        resume()
    }

    // MARK: Devices

    /// Pairs with the Device an Invite link names (ADR 0040), as the phone named
    /// `phoneName`; the handshake runs off the main thread. The Device is kept here under
    /// the name its Server gave, and the library connects to it.
    func pair(link: String, phoneName: String) async throws -> SavedDevice {
        let client = client
        client.setName(name: phoneName)
        let device = try await Task.detached {
            try client.pair(link: link, label: nil)
        }.value
        saved.add(device)
        devices = client.devices()
        workspaces[device.address] = client.workspaces(address: device.address)
        return device
    }

    /// The name this iPhone shows a Device under from now on; a blank one changes nothing.
    func rename(_ address: String, to label: String) {
        let label = label.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !label.isEmpty else { return }
        client.rename(address: address, label: label)
        saved.rename(address, to: label)
        devices = client.devices()
        needsYou = client.needsYou()
    }

    /// Stops using a Device here; revoking the phone is done on the Device (ADR 0040).
    func forget(_ address: String) {
        if shown?.address == address {
            shown = nil
            screen = nil
        }
        client.forget(address: address)
        saved.remove(address)
        devices = client.devices()
        workspaces[address] = nil
        needsYou = client.needsYou()
        statuses = statuses.filter { $0.key.address != address }
        router.forget(address)
    }

    func device(_ address: String) -> DeviceView? {
        devices.first { $0.address == address }
    }

    /// The Pane a notification names, on a Device paired here.
    func ref(_ pushed: PushedPane) -> PaneRef? {
        devices.first { $0.deviceId == pushed.device }
            .map { PaneRef(address: $0.address, pane: pushed.pane) }
    }

    /// A notification shows in the foreground only while its Device is not connected here:
    /// a connected one's change already raises a `Banner`, and the Pane on screen none.
    func presents(_ pushed: PushedPane) -> Bool {
        guard let ref = ref(pushed) else { return true }
        return device(ref.address)?.status != .connected && shown != ref
    }

    func pane(_ ref: PaneRef) -> PaneView? {
        workspaces[ref.address]?.lazy.flatMap(\.panes).first { $0.id == ref.pane }
    }

    func workspace(of ref: PaneRef) -> WorkspaceView? {
        workspaces[ref.address]?.first { $0.panes.contains { $0.id == ref.pane } }
    }

    // MARK: Agents

    /// Every Agent on every connected Device, in Device and Workspace order.
    var agents: [AgentItem] {
        devices.filter { $0.status == .connected }.flatMap(agents(of:))
    }

    func agents(_ statuses: AgentStatus...) -> [AgentItem] {
        agents.filter { statuses.contains($0.agent.status) }
    }

    /// A Device's Agents, whatever its connection.
    func agents(of device: DeviceView) -> [AgentItem] {
        (workspaces[device.address] ?? []).flatMap { workspace in
            workspace.panes.compactMap { pane in
                pane.agent.map {
                    AgentItem(
                        ref: PaneRef(address: device.address, pane: pane.id),
                        device: device.label, workspace: workspace.name, pane: pane, agent: $0)
                }
            }
        }
    }

    // MARK: The Pane on screen (ADR 0041)

    /// The Pane now on screen, at the grid its font and visible area give: the phone
    /// reports focus and sizes the terminal like a window. Opening it marks it seen.
    func show(_ ref: PaneRef, rows: UInt16, columns: UInt16) {
        if shown != ref {
            shown = ref
            screen = client.terminal(address: ref.address, pane: ref.pane)
            alerts.dismissBanner(for: ref)
        }
        client.showPane(address: ref.address, pane: ref.pane, rows: rows, columns: columns)
    }

    /// The Pane left the screen or the app went inactive: focus and sizing go back.
    func hide(_ ref: PaneRef) {
        guard shown == ref else { return }
        client.showPane(address: nil, pane: ref.pane, rows: 0, columns: 0)
        shown = nil
        screen = nil
    }

    func send(text: String) {
        guard let shown else { return }
        client.sendText(address: shown.address, pane: shown.pane, text: text)
    }

    /// A key from the row above the keyboard, with its sticky Ctrl (ADR 0041).
    func send(key: Key, control: Bool = false) {
        guard let shown else { return }
        client.sendKey(address: shown.address, pane: shown.pane, key: key, control: control)
    }

    /// Pasted text, which the Server brackets when the program asked (ADR 0041).
    func paste(text: String) {
        guard let shown else { return }
        client.paste(address: shown.address, pane: shown.pane, text: text)
    }

    /// The field's draft: a paste, then Return once the program has taken it, as the
    /// Server submits an agent prompt (ADR 0041).
    func submit(text: String) {
        guard let shown else { return }
        client.submit(address: shown.address, pane: shown.pane, text: text)
    }

    /// A swipe of `lines` over a cell, positive towards what came before: wheel motion, as
    /// the desktop sends a trackpad's. The Server gives it to a program that tracks the
    /// mouse, turns it into arrow keys on an alternate screen, or scrolls the Pane for
    /// every viewer (ADR 0036, ADR 0041).
    func wheel(row: UInt16, column: UInt16, lines: Int32) {
        guard let shown else { return }
        client.wheel(
            address: shown.address, pane: shown.pane, row: row, column: column, lines: lines)
    }

    /// A tap where the program tracks the mouse: a click on the cell (ADR 0041).
    func click(row: UInt16, column: UInt16) {
        guard let shown else { return }
        client.click(address: shown.address, pane: shown.pane, row: row, column: column)
    }

    // MARK: The Pane's selection, the one every viewer shares (ADR 0008)

    /// A long press: the word around the cell.
    func selectWord(row: UInt16, column: UInt16) {
        guard let shown else { return }
        client.selectWord(address: shown.address, pane: shown.pane, row: row, column: column)
        refresh()
    }

    /// A drag after a long press: the cells from one to the other.
    func selectRange(from: (row: UInt16, column: UInt16), to: (row: UInt16, column: UInt16)) {
        guard let shown else { return }
        client.selectRange(
            address: shown.address, pane: shown.pane, fromRow: from.row, fromColumn: from.column,
            toRow: to.row, toColumn: to.column)
        refresh()
    }

    func clearSelection() {
        guard let shown else { return }
        client.clearSelection(address: shown.address, pane: shown.pane)
        refresh()
    }

    /// Copies the selection; its text comes back as an event, onto the clipboard.
    func copySelection() {
        guard let shown else { return }
        client.copySelection(address: shown.address, pane: shown.pane)
        refresh()
    }

    func scrollToBottom() {
        guard let shown else { return }
        client.scrollToBottom(address: shown.address, pane: shown.pane)
    }
}

/// Wakes the Session on the main thread when the library reports a change. The library
/// calls `changed` from its own thread, once until `takeChanges` ran (ADR 0039). It is
/// handed to the library before the Session exists, so the Session binds itself after.
nonisolated final class SessionListener: Listener {
    private let handler = Mutex<(@Sendable () -> Void)?>(nil)

    func bind(_ handler: @escaping @Sendable () -> Void) {
        self.handler.withLock { $0 = handler }
    }

    func changed() {
        handler.withLock { $0 }?()
    }
}
