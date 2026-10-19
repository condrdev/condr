import CondrKit
import SwiftUI

// The library's records as the lists show them.

nonisolated extension DeviceView: @retroactive Identifiable {
    public var id: String { address }

    var connected: Bool { status == .connected }
}

nonisolated extension WorkspaceView: @retroactive Identifiable {}

nonisolated extension PaneView: @retroactive Identifiable {
    /// The title its program set, else what stands in: its Agent's name or "Shell".
    func name(fallback: String) -> String {
        title ?? fallback
    }

    var name: String { name(fallback: agent?.label ?? "Shell") }
}

nonisolated extension NeedsYouItem: @retroactive Identifiable {
    public var id: String { "\(address)#\(pane)" }

    var ref: PaneRef { PaneRef(address: address, pane: pane) }
}

extension AgentView {
    /// The mark from `assets/agents` for the Agent's kind, or none for a kind this build
    /// has no mark for.
    var mark: ImageResource? { AgentMarks.resource(for: kind) }
}

enum AgentMarks {
    private static let marks: [String: ImageResource] = [
        "antigravity": .agentAntigravity, "claude": .agentClaude, "codex": .agentCodex,
        "copilot": .agentCopilot, "cursor": .agentCursor, "grok": .agentGrok,
        "kimi": .agentKimi, "omp": .agentOmp, "opencode": .agentOpencode, "pi": .agentPi,
    ]

    static func resource(for kind: String?) -> ImageResource? {
        kind.flatMap { marks[$0] }
    }
}

extension AgentStatus {
    /// The desktop sidebar's tones (`agent_sidebar_status`), as the platform's semantic colours.
    var tint: Color {
        switch self {
        case .working: .orange
        case .blocked, .error: .red
        case .done: .green
        case .idle, .unknown: .secondary
        }
    }

    var title: String {
        switch self {
        case .working: String(localized: "Working")
        case .blocked: String(localized: "Needs you")
        case .error: String(localized: "Failed")
        case .done: String(localized: "Done")
        case .idle: String(localized: "Idle")
        case .unknown: String(localized: "Not reporting")
        }
    }

    /// Waiting for an answer or halted on a failure: the person has to come (ADR 0044).
    var needsYou: Bool { self == .blocked || self == .error }

    /// Most urgent first, the order a score draws them in, so each colour reads as a run.
    var urgency: Int {
        switch self {
        case .blocked: 0
        case .error: 1
        case .done: 2
        case .working: 3
        case .unknown: 4
        case .idle: 5
        }
    }
}

extension Array where Element == AgentStatus {
    func byUrgency() -> [AgentStatus] {
        sorted { $0.urgency < $1.urgency }
    }
}

extension Array where Element == AgentItem {
    /// In the order a score draws them: most urgent first, stable within a state.
    func byUrgency() -> [AgentItem] {
        enumerated()
            .sorted {
                ($0.element.agent.status.urgency, $0.offset) < (
                    $1.element.agent.status.urgency, $1.offset
                )
            }
            .map(\.element)
    }
}

/// "condr · macmini"
func place(_ workspace: String, _ device: String) -> String { "\(workspace) · \(device)" }

/// A Blocked Agent's `detail` as the agent wrote it (ADR 0024): "Tool: command" for a
/// permission, else a question or the notification's text.
struct BlockedOn {
    let tool: String?
    let text: String

    init(_ raw: String) {
        if let colon = raw.range(of: ": "),
            raw[..<colon.lowerBound].allSatisfy({ $0.isLetter || $0.isNumber || $0 == "_" })
        {
            tool = String(raw[..<colon.lowerBound])
            text = String(raw[colon.upperBound...])
        } else {
            tool = nil
            text = raw
        }
    }
}

nonisolated extension MobileError {
    /// The library's own words for what went wrong (ADR 0020). uniffi's own
    /// `errorDescription` is the enum's debug dump, so a failure shows this instead.
    var message: String {
        switch self {
        case .Invalid(let message), .Connection(let message): message
        }
    }
}
