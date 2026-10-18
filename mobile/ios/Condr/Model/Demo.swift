import CondrKit
import Foundation
import Synchronization

/// What `-scene <name>` opens, for design review and the UI tests: a world that holds
/// still, drawn by the same views as the real one.
nonisolated enum DemoScene: String {
    case welcome, pair, scan, name, connecting, paired
    case agents
    case lock, devices, device, offline, settings, empty, banner, error, computer
    case terminal, question, codex, shell
    case terminalError = "terminal-error"
    case crowd, many
    case crowdDevice = "crowd-device"

    /// The scene the app was launched into, if any.
    static var launched: DemoScene? {
        UserDefaults.standard.string(forKey: "scene").flatMap(DemoScene.init(rawValue:))
    }

    /// The pairing scenes start with no Device.
    var unpaired: Bool {
        switch self {
        case .welcome, .pair, .scan, .name, .connecting, .paired: true
        default: false
        }
    }
}

/// Three computers and their Agents, as the prototype drew them.
nonisolated enum DemoWorld {
    static let macmini = "tcp://CxQ2mZ5kL9pR3wT7vY1bN4dH8fJ0sA6gK2eU5iO9rW@192.168.1.20:7417"
    static let jpdev = "p2p://h7k2x9q4m3v8w1n6c5b0z2r7t4y9u3p8"
    static let studio = "tcp://K8pL2mQ5rT9wX3zB6cF1gH4jN7sV0yA3dE6iU9oR2tW@studio.local:7417"
    static let buildbox = "p2p://k9m2c4x7v1b8n3q6w5e0r2t9y4u7i1o3"
    static let gpu = "p2p://g4p8u1x6c3n9m2v7b5z0q4w8e1r6t3y9"
    static let fingerprint = "q3Zr8wX0bH4nT1sVd9cK2mLp7fYg5jQe6uRa0iWo-Lk"

    static func agent(_ kind: String, _ status: AgentStatus, _ blockedOn: String? = nil)
        -> AgentView
    {
        let labels = ["claude": "Claude", "codex": "Codex", "opencode": "OpenCode"]
        return AgentView(
            kind: kind, label: labels[kind] ?? kind, status: status, blockedOn: blockedOn)
    }

    static func pane(_ id: UInt64, _ title: String, _ agent: AgentView? = nil) -> PaneView {
        PaneView(id: id, title: title, agent: agent, attention: false, exited: false)
    }

    static let devices: [DeviceView] = [
        DeviceView(address: macmini, label: "macmini", status: .connected),
        DeviceView(address: jpdev, label: "jpdev", status: .connected),
        DeviceView(
            address: studio, label: "studio",
            status: .failed(reason: "No answer from studio.local. Is Condr running there?")),
    ]

    static let workspaces: [String: [WorkspaceView]] = [
        macmini: [
            WorkspaceView(
                id: 1, name: "condr", root: "~/Developer/project/condr",
                panes: [
                    pane(
                        1, "Implement statistics page",
                        agent("claude", .blocked, "Bash: cargo test -p condr-client")),
                    pane(2, "Fix sidebar flicker", agent("codex", .working)),
                    pane(3, "condr"),
                ]),
            WorkspaceView(
                id: 2, name: "rime-config", root: "~/rime-config",
                panes: [pane(4, "Double pinyin schema", agent("opencode", .done))]),
        ],
        jpdev: [
            WorkspaceView(
                id: 11, name: "admin-dashboard", root: "~/work/admin-dashboard",
                panes: [
                    pane(
                        11, "Add usage chart",
                        agent("claude", .blocked, "Which chart library should the dashboard use?")),
                    pane(12, "Migrate auth to passkeys", agent("claude", .working)),
                ]),
            WorkspaceView(
                id: 12, name: "condr-mobile", root: "~/work/condr-mobile",
                panes: [pane(13, "Pairing screen copy", agent("codex", .idle))]),
        ],
        studio: [],
    ]

    /// One computer running fifteen Agents beside the usual two, to try the design at scale.
    static let crowd: ([DeviceView], [String: [WorkspaceView]]) = {
        let box = DeviceView(address: buildbox, label: "buildbox", status: .connected)
        let workspaces = [
            WorkspaceView(
                id: 21, name: "api", root: "~/src/api",
                panes: [
                    pane(
                        21, "Rate-limit the public API",
                        agent("claude", .blocked, "Bash: cargo test -p api --features limits")),
                    pane(22, "Paginate the audit log", agent("codex", .working)),
                    pane(23, "Upgrade axum to 0.9", agent("claude", .working)),
                    pane(24, "Trace slow queries", agent("codex", .working)),
                    pane(25, "Retry failed webhooks", agent("codex", .idle)),
                ]),
            WorkspaceView(
                id: 22, name: "web", root: "~/src/web",
                panes: [
                    pane(26, "Dark mode for settings", agent("claude", .working)),
                    pane(27, "Fix the hydration warning", agent("opencode", .done)),
                    pane(
                        28, "Onboarding checklist",
                        agent("claude", .blocked, "Should the checklist be dismissible?")),
                    pane(29, "Storybook for buttons", agent("codex", .working)),
                    pane(30, "Lazy-load the charts", agent("claude", .working)),
                ]),
            WorkspaceView(
                id: 23, name: "infra", root: "~/src/infra",
                panes: [
                    pane(31, "Terraform the staging database", agent("claude", .working)),
                    pane(32, "Rotate CI secrets", agent("codex", .working)),
                    pane(33, "Monthly cost report", agent("codex", .idle)),
                    pane(34, "Pin the base images", agent("codex", .working)),
                    pane(35, "Alert on disk usage", agent("codex", .idle)),
                ]),
        ]
        return (
            [box, devices[0], devices[2]],
            [buildbox: workspaces, macmini: Self.workspaces[macmini] ?? [], studio: []]
        )
    }()

    /// Five computers, one running twenty-four Agents: the score past the screen's width.
    static let many: ([DeviceView], [String: [WorkspaceView]]) = {
        let tasks = [
            "Shard the embeddings job", "Profile the tokenizer", "Resume failed epochs",
            "Cache the eval set",
            "Quantize the 8B model", "Fix NaN in layer norm", "Compare optimizers",
            "Prune dead checkpoints",
        ]
        let mix: [AgentStatus] =
            [.blocked, .done, .done] + Array(repeating: .working, count: 14)
            + Array(repeating: .idle, count: 7)
        let panes = mix.enumerated().map { index, status in
            pane(
                UInt64(100 + index),
                "\(tasks[index % tasks.count])\(index < tasks.count ? "" : " #\(index / tasks.count + 1)")",
                agent(
                    index % 3 == 0 ? "codex" : "claude", status,
                    status == .blocked ? "Bash: python train.py --resume runs/0412" : nil))
        }
        let (crowdDevices, crowdWorkspaces) = crowd
        var workspaces = crowdWorkspaces.merging(Self.workspaces) { crowd, _ in crowd }
        workspaces[gpu] = [
            WorkspaceView(
                id: 31, name: "training", root: "~/ml/training", panes: Array(panes[..<12])),
            WorkspaceView(id: 32, name: "evals", root: "~/ml/evals", panes: Array(panes[12...])),
        ]
        return (
            [
                crowdDevices[0], DeviceView(address: gpu, label: "gpu-01", status: .connected),
                devices[0], devices[1], devices[2],
            ],
            workspaces
        )
    }()

    /// The world a scene opens on.
    static func world(for scene: DemoScene) -> (
        devices: [DeviceView], workspaces: [String: [WorkspaceView]]
    ) {
        if scene.unpaired { return ([], [:]) }
        var (devices, workspaces) =
            switch scene {
            case .crowd, .crowdDevice: crowd
            case .many: many
            default: (Self.devices, Self.workspaces)
            }
        if scene == .empty {
            workspaces = workspaces.mapValues { list in
                list.map { workspace in
                    var workspace = workspace
                    workspace.panes = workspace.panes.map { pane in
                        var pane = pane
                        if pane.agent?.status == .blocked {
                            pane.agent?.status = .working
                            pane.agent?.blockedOn = nil
                        }
                        return pane
                    }
                    return workspace
                }
            }
        }
        if scene == .banner, var list = workspaces[jpdev] {
            list[0].panes[1].agent = agent(
                "claude", .blocked, "Bash: pnpm prisma migrate dev --name passkeys")
            workspaces[jpdev] = list
        }
        _ = devices
        return (devices, workspaces)
    }
}

/// The screens the demo Panes show: a few lines each, so the terminal page has something
/// to draw and the key row something to answer.
nonisolated enum DemoScreens {
    static let columns: UInt16 = 44
    static let rows: UInt16 = 18
    static let foreground: UInt32 = 0xc9d1d9
    static let background: UInt32 = 0x0d1117
    static let green: UInt32 = 0x3fb950
    static let yellow: UInt32 = 0xd29922
    static let blue: UInt32 = 0x58a6ff
    static let gray: UInt32 = 0x6e7681
    static let white: UInt32 = 0xffffff

    typealias Span = (text: String, color: UInt32, bold: Bool)

    static func span(_ text: String, _ color: UInt32 = foreground, bold: Bool = false) -> Span {
        (text, color, bold)
    }

    static func screen(_ pane: UInt64) -> TerminalScreen {
        let lines: [[Span]] =
            switch pane {
            case 1:
                [
                    [
                        span("● ", green), span("Bash", white, bold: true),
                        span("(cargo test -p condr-client)", gray),
                    ],
                    [span("  ⎿  ", gray), span("Running 46 tests")],
                    [],
                    [span("Allow Bash to run this command?", white, bold: true)],
                    [],
                    [span("❯ 1. Yes", blue, bold: true)],
                    [span("  2. Yes, and don't ask again this session")],
                    [span("  3. No, and tell Claude what to do differently")],
                    [],
                    [span("Esc to cancel · Enter to choose", gray)],
                ]
            case 11:
                [
                    [
                        span("● ", green),
                        span("Which chart library should the dashboard use?", white, bold: true),
                    ],
                    [],
                    [
                        span("❯ 1. Recharts", blue, bold: true),
                        span("  already in package.json", gray),
                    ],
                    [span("  2. Chart.js")],
                    [span("  3. Something else")],
                    [],
                    [span("Enter to choose", gray)],
                ]
            case 2:
                [
                    [span("codex", white, bold: true), span(" · gpt-5-codex", gray)],
                    [span("⠋ ", yellow), span("Reading crates/condr-gui/src/app/sidebar.rs")],
                    [span("  ", gray), span("sidebar.rs:214 repaints every Pane on hover", gray)],
                    [],
                    [span("Working…", yellow)],
                ]
            case 12:
                [
                    [
                        span("● ", yellow), span("Edit", white, bold: true),
                        span("(src/auth/passkeys.ts)", gray),
                    ],
                    [span("  ⎿  ", gray), span("Added 42 lines")],
                    [
                        span("● ", yellow), span("Bash", white, bold: true),
                        span("(pnpm test auth)", gray),
                    ],
                    [span("  ⎿  ", gray), span("12 passed", green)],
                    [],
                    [span("✻ Thinking…", yellow)],
                ]
            case 4:
                [
                    [span("opencode", white, bold: true)],
                    [span("● ", green), span("Rewrote double_pinyin.schema.yaml")],
                    [span("● ", green), span("rime --deploy: ok", green)],
                    [],
                    [span("Done.", green, bold: true)],
                ]
            case 13:
                [
                    [span("codex", white, bold: true), span(" · gpt-5-codex", gray)],
                    [span("› ", blue), span("Tighten the pairing screen copy")],
                    [span("  ", gray), span("Rewrote 6 strings in Onboarding.swift", gray)],
                    [],
                    [span("› ", blue), span("", foreground)],
                ]
            default:
                [
                    [
                        span("rocky@macmini ", green), span("~/Developer/project/condr", blue),
                        span(" %"),
                    ],
                    [span("$ cargo build")],
                    [span("   Compiling condr-core v0.1.10", green)],
                    [span("    Finished dev profile in 12.4s", green)],
                    [
                        span("rocky@macmini ", green), span("~/Developer/project/condr", blue),
                        span(" % "),
                    ],
                ]
            }
        var cells: [ScreenCell] = []
        cells.reserveCapacity(Int(rows) * Int(columns))
        var cursor = ScreenCursor(row: UInt16(lines.count - 1), column: 0, color: 0xf0f6fc)
        for row in 0..<Int(rows) {
            var column = 0
            for spanned in (row < lines.count ? lines[row] : []) {
                for character in spanned.text where column < Int(columns) {
                    cells.append(
                        ScreenCell(
                            text: String(character), foreground: spanned.color,
                            background: background,
                            bold: spanned.bold, italic: false, underline: false, link: nil))
                    column += 1
                }
            }
            if row == lines.count - 1 { cursor.column = UInt16(column) }
            while column < Int(columns) {
                cells.append(
                    ScreenCell(
                        text: " ", foreground: foreground, background: background, bold: false,
                        italic: false, underline: false, link: nil))
                column += 1
            }
        }
        return TerminalScreen(
            revision: 1, rows: rows, columns: columns, scrolledLines: 0, cells: cells,
            cursor: cursor,
            background: background, mouseTracking: false, hasSelection: false)
    }
}

/// The library's shape over the demo world: nothing connects, nothing moves on its own.
nonisolated final class DemoClient: CompanionProtocol {
    private struct State {
        var devices: [DeviceView]
        var workspaces: [String: [WorkspaceView]]
        /// The demo Pane's selection, as cell indices, both included.
        var selection: (pane: UInt64, cells: ClosedRange<Int>)?
        var changes: [Change] = []
        var events: [Event] = []
    }

    /// The default scheme's selection, as the library colours it.
    private static let selectionBackground: UInt32 = 0xe6edf3
    private static let selectionText: UInt32 = 0x0d1117

    private let state: Mutex<State>

    init(scene: DemoScene) {
        let world = DemoWorld.world(for: scene)
        state = Mutex(State(devices: world.devices, workspaces: world.workspaces))
    }

    func close() {}
    func resume() {}
    func suspend() {}
    func setActive(active: Bool) {}
    func networkChanged() {}

    func takeChanges() -> [Change] {
        state.withLock { state in
            defer { state.changes = [] }
            return state.changes
        }
    }

    func takeEvents() -> [Event] {
        state.withLock { state in
            defer { state.events = [] }
            return state.events
        }
    }

    func devices() -> [DeviceView] {
        state.withLock { $0.devices }
    }

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

    func setName(name: String) {}

    func pair(link: String, label: String?) throws -> SavedDevice {
        guard link.hasPrefix("p2p://") || link.hasPrefix("tcp://"), link.contains(".") else {
            throw MobileError.Invalid(
                message: "That isn't a pairing link. It starts with p2p:// or tcp://.")
        }
        return state.withLock { state in
            let known = Set(state.devices.map(\.address))
            let next =
                DemoWorld.devices.first { !known.contains($0.address) && $0.status == .connected }
                ?? DemoWorld.devices[0]
            if !known.contains(next.address) {
                state.devices.append(next)
                state.workspaces[next.address] = DemoWorld.workspaces[next.address] ?? []
            }
            return SavedDevice(address: next.address, label: next.label)
        }
    }

    func forget(address: String) {
        state.withLock { state in
            state.devices.removeAll { $0.address == address }
            state.workspaces[address] = nil
        }
    }

    /// Like the library, whatever screen the Pane has, asked before or after it is shown,
    /// its selection in the selection's colours.
    func terminal(address: String, pane: UInt64) -> TerminalScreen? {
        var screen = DemoScreens.screen(pane)
        guard let selection = state.withLock({ $0.selection }), selection.pane == pane else {
            return screen
        }
        for index in selection.cells where screen.cells.indices.contains(index) {
            screen.cells[index].foreground = Self.selectionText
            screen.cells[index].background = Self.selectionBackground
        }
        screen.hasSelection = true
        return screen
    }

    func showPane(address: String?, pane: UInt64, rows: UInt16, columns: UInt16) {}

    func rename(address: String, label: String) {}
    func sendText(address: String, pane: UInt64, text: String) {}
    func sendKey(address: String, pane: UInt64, key: Key, control: Bool) {}
    func paste(address: String, pane: UInt64, text: String) {}
    func submit(address: String, pane: UInt64, text: String) {}
    func wheel(address: String, pane: UInt64, row: UInt16, column: UInt16, lines: Int32) {}
    func click(address: String, pane: UInt64, row: UInt16, column: UInt16) {}
    func scrollToBottom(address: String, pane: UInt64) {}

    /// The run of non-blank cells around the one pressed, as the Server picks a word.
    func selectWord(address: String, pane: UInt64, row: UInt16, column: UInt16) {
        let screen = DemoScreens.screen(pane)
        let width = Int(screen.columns)
        let start = Int(row) * width
        let pressed = start + Int(column)
        guard screen.cells.indices.contains(pressed), screen.cells[pressed].text != " " else {
            return
        }
        var first = pressed
        var last = pressed
        while first > start, screen.cells[first - 1].text != " " { first -= 1 }
        while last + 1 < start + width, screen.cells[last + 1].text != " " { last += 1 }
        select(pane: pane, address: address, cells: first...last)
    }

    func selectRange(
        address: String, pane: UInt64, fromRow: UInt16, fromColumn: UInt16, toRow: UInt16,
        toColumn: UInt16
    ) {
        let width = Int(DemoScreens.columns)
        let from = Int(fromRow) * width + Int(fromColumn)
        let to = Int(toRow) * width + Int(toColumn)
        select(pane: pane, address: address, cells: min(from, to)...max(from, to))
    }

    func clearSelection(address: String, pane: UInt64) {
        state.withLock { state in
            state.selection = nil
            state.changes.append(.terminal(address: address, pane: pane))
        }
    }

    func copySelection(address: String, pane: UInt64) {
        let screen = DemoScreens.screen(pane)
        state.withLock { state in
            guard let selection = state.selection, selection.pane == pane else { return }
            let text = selection.cells.map { screen.cells[$0].text }.joined()
            state.events.append(.copied(text: text.trimmingCharacters(in: .whitespaces)))
        }
    }

    private func select(pane: UInt64, address: String, cells: ClosedRange<Int>) {
        state.withLock { state in
            state.selection = (pane, cells)
            state.changes.append(.terminal(address: address, pane: pane))
        }
    }
}
