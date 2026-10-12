import Observation
import SwiftUI

// The prototype's world. Shapes follow condr-mobile's views (`DeviceView`, `WorkspaceView`,
// `PaneView`, `AgentView`, `NeedsYouItem`) so the app can swap this store for CondrKit.

enum AgentKind: String {
    case claude, codex, opencode

    var label: String {
        switch self {
        case .claude: "Claude"
        case .codex: "Codex"
        case .opencode: "OpenCode"
        }
    }

    var mark: String { "agent-\(rawValue)" }
}

enum AgentStatus: Hashable {
    case unknown, idle, working, blocked, done
}

struct Agent: Equatable {
    var kind: AgentKind
    var status: AgentStatus
    var blockedOn: String?
}

struct Pane: Identifiable, Equatable {
    let id: Int
    /// The title its program set, else its Agent's name.
    var title: String
    var agent: Agent?
    /// It rang the bell and nobody has looked since.
    var attention = false
    var exited = false
    // What the prototype draws in it.
    var script: Script
    var phase = 0
    var selection = 0
    var sent: [String] = []
    var replying = false
    var interrupted = false
    var since = Date()
}

struct Workspace: Identifiable, Equatable {
    let id: Int
    var name: String
    var root: String
    var panes: [Pane]
}

enum DeviceStatus: Equatable {
    case connecting
    case connected
    /// Retrying failed; the reason is the connection's own words (ADR 0020).
    case failed(String)
    /// The Server no longer knows this phone: `NotAuthorized` or `DeviceRevoked` (ADR 0040).
    case revoked
}

struct Device: Identifiable, Equatable {
    var id: String { address }
    let address: String
    var label: String
    var status: DeviceStatus
    var workspaces: [Workspace]

    var agents: [Pane] { workspaces.flatMap(\.panes).filter { $0.agent != nil } }
}

struct PaneRef: Hashable {
    let device: String
    let pane: Int
}

/// One Agent with where it runs, for the lists.
struct AgentItem: Identifiable {
    var id: PaneRef { ref }
    let ref: PaneRef
    let device: String
    let workspace: String
    let pane: Pane
    var agent: Agent { pane.agent! }
}

enum Key: Equatable {
    case enter, escape, tab, backspace, up, down, left, right
    case character(String)
}

struct Banner: Identifiable, Equatable {
    let id = UUID()
    let ref: PaneRef
    let status: AgentStatus
    let kind: AgentKind?
    let title: String
    let detail: String
    /// Raised while the first screen is showing, which shows the change itself: felt, not shown.
    var quiet = false
}

@Observable
final class Store {
    enum Tab: Hashable { case agents, devices, settings }

    var paired: Bool
    var locked: Bool
    var devices: [Device]
    var tab: Tab = .agents
    var agentsPath: [DeviceRoute] = []
    var devicesPath: [DeviceRoute] = []
    var settingsPath: [String] = []
    /// The Pane on screen: the one the phone watches and sizes (ADR 0041).
    var showing: PaneRef?
    var banner: Banner?
    var error: String?
    var pairingAnother = false
    var fontSize: CGFloat = 12.5
    var phoneName = "iPhone 18 Pro"
    let fingerprint = "q3Zr8wX0bH4nT1sVd9cK2mLp7fYg5jQe6uRa0iWo-Lk"
    let live: Bool

    init() {
        let defaults = UserDefaults.standard
        let scene = defaults.string(forKey: "scene") ?? ""
        let isPaired = !["welcome", "pair", "scan", "name", "network", "connecting", "paired"].contains(scene)
        var world = isPaired ? (scene.hasPrefix("crowd") ? Demo.crowd : scene == "many" ? Demo.many : Demo.devices) : []
        if scene == "empty" {
            for d in world.indices { for w in world[d].workspaces.indices { for p in world[d].workspaces[w].panes.indices {
                if world[d].workspaces[w].panes[p].agent?.status == .blocked {
                    world[d].workspaces[w].panes[p].agent?.status = .working
                    world[d].workspaces[w].panes[p].agent?.blockedOn = nil
                    world[d].workspaces[w].panes[p].phase = 1
                }
            } } }
        }
        if scene == "banner", let d = world.firstIndex(where: { $0.address == Demo.jpdev }) {
            world[d].workspaces[0].panes[1].agent?.status = .blocked
            world[d].workspaces[0].panes[1].agent?.blockedOn = "Bash: pnpm prisma migrate dev --name passkeys"
            world[d].workspaces[0].panes[1].phase = 1
        }
        live = !defaults.bool(forKey: "static")
        paired = isPaired
        locked = scene == "lock" || (scene.isEmpty && isPaired)
        devices = world
        switch scene {
        case "devices": tab = .devices
        case "device": tab = .devices; devicesPath = [.device(Demo.macmini)]
        case "offline": tab = .devices; devicesPath = [.device(Demo.studio)]
        case "settings": tab = .settings
        case "error": error = "Couldn't reach macmini: the connection closed while opening the Pane."
        case "terminal": agentsPath = [.pane(PaneRef(device: Demo.macmini, pane: 1))]
        case "terminal-error":
            agentsPath = [.pane(PaneRef(device: Demo.macmini, pane: 1))]
            error = "Couldn't send to macmini: the connection closed. Your reply is still in the field."
        case "question": agentsPath = [.pane(PaneRef(device: Demo.jpdev, pane: 11))]
        case "codex": agentsPath = [.pane(PaneRef(device: Demo.macmini, pane: 2))]
        case "computer": agentsPath = [.device(Demo.macmini)]
        case "crowd-device": agentsPath = [.device(Demo.buildbox)]
        case "shell": tab = .devices; devicesPath = [.device(Demo.macmini), .pane(PaneRef(device: Demo.macmini, pane: 3))]
        case "banner":
            tab = .devices
            banner = Banner(ref: PaneRef(device: Demo.jpdev, pane: 12), status: .blocked, kind: .claude,
                            title: "Claude needs you", detail: "Migrate auth to passkeys · jpdev")
        default: break
        }
        if live, paired { simulate() }
    }

    // MARK: Reading

    var connected: [Device] { devices.filter { $0.status == .connected } }

    var agents: [AgentItem] {
        connected.flatMap { device in
            device.workspaces.flatMap { workspace in
                workspace.panes.compactMap { pane in
                    pane.agent == nil ? nil : AgentItem(
                        ref: PaneRef(device: device.address, pane: pane.id),
                        device: device.label, workspace: workspace.name, pane: pane)
                }
            }
        }
    }

    func agents(_ status: AgentStatus...) -> [AgentItem] {
        agents.filter { status.contains($0.agent.status) }
    }

    func device(_ address: String) -> Device? { devices.first { $0.address == address } }

    func pane(_ ref: PaneRef) -> Pane? {
        device(ref.device)?.workspaces.flatMap(\.panes).first { $0.id == ref.pane }
    }

    func workspace(of ref: PaneRef) -> Workspace? {
        device(ref.device)?.workspaces.first { $0.panes.contains { $0.id == ref.pane } }
    }

    // MARK: Changing

    func update(_ ref: PaneRef, _ change: (inout Pane) -> Void) {
        guard let d = devices.firstIndex(where: { $0.address == ref.device }) else { return }
        for w in devices[d].workspaces.indices {
            if let p = devices[d].workspaces[w].panes.firstIndex(where: { $0.id == ref.pane }) {
                change(&devices[d].workspaces[w].panes[p])
            }
        }
    }

    /// Opening a Pane marks it seen: its Done becomes Idle, its bell is acknowledged.
    func show(_ ref: PaneRef) {
        showing = ref
        update(ref) { pane in
            pane.attention = false
            if pane.agent?.status == .done { pane.agent?.status = .idle }
        }
        if banner?.ref == ref { banner = nil }
    }

    func hide(_ ref: PaneRef) {
        if showing == ref { showing = nil }
    }

    /// Opens a Pane from anywhere: a banner, the score.
    func open(_ ref: PaneRef) {
        banner = nil
        tab = .agents
        agentsPath = [.pane(ref)]
    }

    func setAgent(_ ref: PaneRef, _ status: AgentStatus, blockedOn: String? = nil, phase: Int? = nil) {
        guard let before = pane(ref), let agent = before.agent else { return }
        let finished = status == .idle && (agent.status == .working || agent.status == .blocked)
        let watched = showing == ref
        let shown: AgentStatus = finished && !watched ? .done : status
        update(ref) { pane in
            pane.agent?.status = shown
            pane.agent?.blockedOn = blockedOn
            if let phase { pane.phase = phase }
            pane.since = Date()
        }
        guard !watched, let workspace = workspace(of: ref), let device = device(ref.device) else { return }
        if shown == .blocked || shown == .done {
            banner = Banner(
                ref: ref, status: shown, kind: agent.kind,
                title: shown == .blocked ? "\(agent.kind.label) needs you" : "\(agent.kind.label) is done",
                detail: "\(before.title) · \(place(workspace.name, device.label))",
                quiet: tab == .agents && agentsPath.isEmpty)
        }
    }

    func forget(_ address: String) {
        devices.removeAll { $0.address == address }
        devicesPath = []
        if devices.isEmpty { paired = false }
    }

    func retry(_ address: String) {
        guard let index = devices.firstIndex(where: { $0.address == address }) else { return }
        let before = devices[index].status
        devices[index].status = .connecting
        Task {
            try? await Task.sleep(for: .seconds(1.6))
            if let index = devices.firstIndex(where: { $0.address == address }) {
                devices[index].status = before
            }
        }
    }

    func pair(name: String) {
        phoneName = name
        if !devices.contains(where: { $0.address == Demo.macmini }) {
            devices.insert(Demo.devices[0], at: 0)
        } else if !devices.contains(where: { $0.address == Demo.jpdev }) {
            devices.insert(Demo.devices[1], at: 1)
        }
        if !paired {
            paired = true
            if live { simulate() }
        }
    }

    // MARK: Input

    func send(_ ref: PaneRef, key: Key, control: Bool = false) {
        guard let pane = pane(ref) else { return }
        if control, case .character(let c) = key, c.lowercased() == "c" {
            interrupt(ref)
            return
        }
        switch (pane.script.menu, pane.agent?.status) {
        case (let options?, .blocked):
            switch key {
            case .up: update(ref) { $0.selection = max(0, $0.selection - 1) }
            case .down: update(ref) { $0.selection = min(options - 1, $0.selection + 1) }
            case .enter: choose(ref, pane.selection)
            case .escape: choose(ref, options - 1)
            case .character(let c):
                if let n = Int(c), (1...options).contains(n) { choose(ref, n - 1) }
            default: break
            }
        case (_, .working):
            if key == .escape { interrupt(ref) }
        default:
            break
        }
    }

    func send(_ ref: PaneRef, text: String) {
        guard let pane = pane(ref), !text.isEmpty else { return }
        if pane.agent?.status == .blocked {
            for character in text { send(ref, key: .character(String(character))) }
            return
        }
        update(ref) { $0.sent.append(text); $0.interrupted = false }
        guard pane.agent != nil else { return }
        update(ref) { $0.replying = true }
        setAgent(ref, .working)
        Task {
            try? await Task.sleep(for: .seconds(4))
            if self.pane(ref)?.replying == true {
                update(ref) { $0.replying = false }
                setAgent(ref, .idle)
            }
        }
    }

    private func choose(_ ref: PaneRef, _ option: Int) {
        guard let pane = pane(ref), let options = pane.script.menu else { return }
        update(ref) { $0.selection = option }
        if option == options - 1 {
            setAgent(ref, .idle, phase: 3)
            return
        }
        setAgent(ref, .working, phase: 1)
        Task {
            try? await Task.sleep(for: .seconds(4.5))
            if self.pane(ref)?.phase == 1 { setAgent(ref, .idle, phase: 2) }
        }
    }

    private func interrupt(_ ref: PaneRef) {
        guard let pane = pane(ref), pane.agent?.status == .working || pane.agent?.status == .blocked else { return }
        if pane.replying {
            update(ref) { $0.replying = false; $0.interrupted = true }
            setAgent(ref, .idle)
        } else {
            setAgent(ref, .idle, phase: Script.interrupted)
        }
    }

    // MARK: The world moving on its own

    private func simulate() {
        Task {
            try? await Task.sleep(for: .seconds(16))
            let codex = PaneRef(device: Demo.macmini, pane: 2)
            if pane(codex)?.agent?.status == .working { setAgent(codex, .idle, phase: 1) }
            try? await Task.sleep(for: .seconds(22))
            let passkeys = PaneRef(device: Demo.jpdev, pane: 12)
            if pane(passkeys)?.agent?.status == .working {
                setAgent(passkeys, .blocked, blockedOn: "Bash: pnpm prisma migrate dev --name passkeys", phase: 1)
            }
        }
    }
}

enum DeviceRoute: Hashable {
    case device(String)
    case pane(PaneRef)
}

// MARK: - Demo data

enum Demo {
    static let macmini = "tcp://192.168.1.20:7417"
    static let jpdev = "p2p://h7k2x9q4m3v8w1n6c5b0z2r7t4y9u3p8"
    static let studio = "tcp://studio.local:7417"

    static let devices: [Device] = [
        Device(address: macmini, label: "macmini", status: .connected, workspaces: [
            Workspace(id: 1, name: "condr", root: "~/Developer/project/condr", panes: [
                Pane(id: 1, title: "Implement statistics page",
                     agent: Agent(kind: .claude, status: .blocked, blockedOn: "Bash: cargo test -p condr-client"),
                     script: .claudeStats),
                Pane(id: 2, title: "Fix sidebar flicker",
                     agent: Agent(kind: .codex, status: .working), script: .codexSidebar,
                     since: Date().addingTimeInterval(-14)),
                Pane(id: 3, title: "condr", script: .shell),
            ]),
            Workspace(id: 2, name: "rime-config", root: "~/rime-config", panes: [
                Pane(id: 4, title: "Double pinyin schema",
                     agent: Agent(kind: .opencode, status: .done), script: .opencodeRime),
            ]),
        ]),
        Device(address: jpdev, label: "jpdev", status: .connected, workspaces: [
            Workspace(id: 11, name: "admin-dashboard", root: "~/work/admin-dashboard", panes: [
                Pane(id: 11, title: "Add usage chart",
                     agent: Agent(kind: .claude, status: .blocked, blockedOn: "Which chart library should the dashboard use?"),
                     script: .claudeChart),
                Pane(id: 12, title: "Migrate auth to passkeys",
                     agent: Agent(kind: .claude, status: .working), script: .claudePasskeys,
                     since: Date().addingTimeInterval(-128)),
            ]),
            Workspace(id: 12, name: "condr-mobile", root: "~/work/condr-mobile", panes: [
                Pane(id: 13, title: "Pairing screen copy",
                     agent: Agent(kind: .codex, status: .idle), script: .codexIdle),
            ]),
        ]),
        Device(address: studio, label: "studio", status: .failed("No answer from studio.local. Is Condr running there?"), workspaces: []),
    ]

    static let buildbox = "p2p://k9m2c4x7v1b8n3q6w5e0r2t9y4u7i1o3"

    /// One computer running fifteen Agents beside the usual two, to try the design at scale.
    static let crowd: [Device] = {
        func pane(_ id: Int, _ title: String, _ kind: AgentKind, _ status: AgentStatus, _ script: Script, _ blockedOn: String? = nil) -> Pane {
            Pane(id: id, title: title, agent: Agent(kind: kind, status: status, blockedOn: blockedOn), script: script,
                 since: Date().addingTimeInterval(-Double(id * 37)))
        }
        let box = Device(address: buildbox, label: "buildbox", status: .connected, workspaces: [
            Workspace(id: 21, name: "api", root: "~/src/api", panes: [
                pane(21, "Rate-limit the public API", .claude, .blocked, .claudeStats, "Bash: cargo test -p api --features limits"),
                pane(22, "Paginate the audit log", .codex, .working, .codexSidebar),
                pane(23, "Upgrade axum to 0.9", .claude, .working, .claudePasskeys),
                pane(24, "Trace slow queries", .codex, .working, .codexSidebar),
                pane(25, "Retry failed webhooks", .codex, .idle, .codexIdle),
            ]),
            Workspace(id: 22, name: "web", root: "~/src/web", panes: [
                pane(26, "Dark mode for settings", .claude, .working, .claudePasskeys),
                pane(27, "Fix the hydration warning", .opencode, .done, .opencodeRime),
                pane(28, "Onboarding checklist", .claude, .blocked, .claudeChart, "Should the checklist be dismissible?"),
                pane(29, "Storybook for buttons", .codex, .working, .codexSidebar),
                pane(30, "Lazy-load the charts", .claude, .working, .claudePasskeys),
            ]),
            Workspace(id: 23, name: "infra", root: "~/src/infra", panes: [
                pane(31, "Terraform the staging database", .claude, .working, .claudePasskeys),
                pane(32, "Rotate CI secrets", .codex, .working, .codexSidebar),
                pane(33, "Monthly cost report", .codex, .idle, .codexIdle),
                pane(34, "Pin the base images", .codex, .working, .codexSidebar),
                pane(35, "Alert on disk usage", .codex, .idle, .codexIdle),
            ]),
        ])
        return [box, devices[0], devices[2]]
    }()

    /// Five computers, one running twenty-four Agents: the score past the screen's width.
    static let many: [Device] = {
        let tasks = ["Shard the embeddings job", "Profile the tokenizer", "Resume failed epochs", "Cache the eval set",
                     "Quantize the 8B model", "Fix NaN in layer norm", "Compare optimizers", "Prune dead checkpoints"]
        let mix: [AgentStatus] = [.blocked, .done, .done] + Array(repeating: .working, count: 14) + Array(repeating: .idle, count: 7)
        let panes = mix.enumerated().map { index, status in
            Pane(id: 100 + index, title: "\(tasks[index % tasks.count])\(index < tasks.count ? "" : " #\(index / tasks.count + 1)")",
                 agent: Agent(kind: index % 3 == 0 ? .codex : .claude, status: status,
                              blockedOn: status == .blocked ? "Bash: python train.py --resume runs/0412" : nil),
                 script: status == .blocked ? .claudeStats : status == .working ? .claudePasskeys : status == .done ? .opencodeRime : .codexIdle,
                 since: Date().addingTimeInterval(-Double(index * 53)))
        }
        let gpu = Device(address: "p2p://g4p8u1x6c3n9m2v7b5z0q4w8e1r6t3y9", label: "gpu-01", status: .connected, workspaces: [
            Workspace(id: 31, name: "training", root: "~/ml/training", panes: Array(panes[..<12])),
            Workspace(id: 32, name: "evals", root: "~/ml/evals", panes: Array(panes[12...])),
        ])
        return [crowd[0], gpu, devices[0], devices[1], devices[2]]
    }()
}
