import CondrKit
import Foundation
import Synchronization

@testable import Condr

/// The library's shape under a test's control: what it answers, and what it was asked.
nonisolated final class FakeClient: CompanionProtocol {
    nonisolated struct Shown: Equatable {
        let address: String?
        let pane: UInt64
        let rows: UInt16
        let columns: UInt16
    }

    nonisolated struct State {
        var devices: [DeviceView] = []
        var workspaces: [String: [WorkspaceView]] = [:]
        var screens: [PaneRef: TerminalScreen] = [:]
        var changes: [Change] = []
        var events: [Event] = []
        var shows: [Shown] = []
        var keys: [(Key, Bool)] = []
        var texts: [String] = []
        var forgotten: [String] = []
        var pairs: [(String, String?)] = []
        var names: [String] = []
        var scrolls: [Int32] = []
        var clicks: [(UInt16, UInt16)] = []
        var pastes: [String] = []
        var submits: [String] = []
        var selections: [String] = []
        var lifecycle: [String] = []
    }

    let state: Mutex<State>

    init(_ state: State = State()) {
        self.state = Mutex(state)
    }

    func update(_ change: (inout State) -> Void) {
        state.withLock { change(&$0) }
    }

    func close() { state.withLock { $0.lifecycle.append("close") } }
    func resume() { state.withLock { $0.lifecycle.append("resume") } }
    func suspend() { state.withLock { $0.lifecycle.append("suspend") } }
    func setActive(active: Bool) {
        state.withLock { $0.lifecycle.append(active ? "active" : "inactive") }
    }
    func networkChanged() { state.withLock { $0.lifecycle.append("network") } }
    func takeChanges() -> [Change] {
        state.withLock { state in
            let taken = state.changes
            state.changes = []
            return taken
        }
    }
    func takeEvents() -> [Event] {
        state.withLock { state in
            let taken = state.events
            state.events = []
            return taken
        }
    }
    func devices() -> [DeviceView] { state.withLock { $0.devices } }

    func needsYou() -> [NeedsYouItem] {
        state.withLock { state in
            state.devices.filter { $0.status == .connected }.flatMap { device in
                (state.workspaces[device.address] ?? []).flatMap { workspace in
                    workspace.panes.compactMap { pane -> NeedsYouItem? in
                        guard let agent = pane.agent, agent.status == .blocked else { return nil }
                        return NeedsYouItem(
                            address: device.address, device: device.label,
                            workspace: workspace.name,
                            pane: pane.id, kind: agent.kind, agent: agent.label,
                            blockedOn: agent.blockedOn)
                    }
                }
            }
        }
    }

    func workspaces(address: String) -> [WorkspaceView] {
        state.withLock { $0.workspaces[address] ?? [] }
    }

    func setName(name: String) { state.withLock { $0.names.append(name) } }

    func pair(link: String, label: String?) throws -> SavedDevice {
        try state.withLock { state in
            guard link.hasPrefix("tcp://") || link.hasPrefix("p2p://") else {
                throw MobileError.Invalid(message: "not a link")
            }
            state.pairs.append((link, label))
            let device = DeviceView(
                address: String(link.prefix(while: { $0 != "." })), label: label ?? "desk",
                status: .connected
            )
            state.devices.append(device)
            return SavedDevice(address: device.address, label: device.label)
        }
    }

    func forget(address: String) {
        state.withLock { state in
            state.forgotten.append(address)
            state.devices.removeAll { $0.address == address }
        }
    }

    func terminal(address: String, pane: UInt64) -> TerminalScreen? {
        state.withLock { $0.screens[PaneRef(address: address, pane: pane)] }
    }

    func showPane(address: String?, pane: UInt64, rows: UInt16, columns: UInt16) {
        state.withLock {
            $0.shows.append(Shown(address: address, pane: pane, rows: rows, columns: columns))
        }
    }

    func sendText(address: String, pane: UInt64, text: String) {
        state.withLock { $0.texts.append(text) }
    }
    func sendKey(address: String, pane: UInt64, key: Key, control: Bool) {
        state.withLock { $0.keys.append((key, control)) }
    }
    func wheel(address: String, pane: UInt64, row: UInt16, column: UInt16, lines: Int32) {
        state.withLock { $0.scrolls.append(lines) }
    }
    func click(address: String, pane: UInt64, row: UInt16, column: UInt16) {
        state.withLock { $0.clicks.append((row, column)) }
    }
    func paste(address: String, pane: UInt64, text: String) {
        state.withLock { $0.pastes.append(text) }
    }
    func submit(address: String, pane: UInt64, text: String) {
        state.withLock { $0.submits.append(text) }
    }
    func selectWord(address: String, pane: UInt64, row: UInt16, column: UInt16) {
        state.withLock { $0.selections.append("word \(row):\(column)") }
    }
    func selectRange(
        address: String, pane: UInt64, fromRow: UInt16, fromColumn: UInt16, toRow: UInt16,
        toColumn: UInt16
    ) {
        state.withLock {
            $0.selections.append("range \(fromRow):\(fromColumn)-\(toRow):\(toColumn)")
        }
    }
    func clearSelection(address: String, pane: UInt64) {
        state.withLock { $0.selections.append("clear") }
    }
    func copySelection(address: String, pane: UInt64) {
        state.withLock { $0.selections.append("copy") }
    }
    func scrollToBottom(address: String, pane: UInt64) { state.withLock { $0.scrolls.append(0) } }
}

/// One computer with one Workspace and one Pane, whose Agent is in `status`.
enum Fixture {
    static let address = "tcp://server@127.0.0.1:1"
    static let ref = PaneRef(address: address, pane: 7)

    static func world(_ status: AgentStatus, blockedOn: String? = nil) -> FakeClient.State {
        var state = FakeClient.State()
        state.devices = [DeviceView(address: address, label: "desk", status: .connected)]
        state.workspaces = [
            address: [
                WorkspaceView(
                    id: 1, name: "condr", root: "~/condr",
                    panes: [
                        PaneView(
                            id: 7, title: "Fix the thing",
                            agent: AgentView(
                                kind: "claude", label: "Claude", status: status,
                                blockedOn: blockedOn),
                            attention: false, exited: false)
                    ])
            ]
        ]
        return state
    }

    /// Fresh defaults for one test, never shared with another.
    static func defaults() -> UserDefaults {
        let name = "test-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name) ?? .standard
        defaults.removePersistentDomain(forName: name)
        return defaults
    }

    @MainActor
    static func session(_ client: FakeClient, saved: SavedDeviceStore? = nil) -> (
        Session, Router, Alerts, SavedDeviceStore
    ) {
        let router = Router()
        let alerts = Alerts()
        let store = saved ?? SavedDeviceStore(defaults: defaults())
        let session = Session(
            client: client, fingerprint: "fp", saved: store, alerts: alerts, router: router)
        return (session, router, alerts, store)
    }
}
