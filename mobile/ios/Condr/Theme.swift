import SwiftUI

// The thin Condr layer over the platform: what the desktop already decided, drawn natively.

extension Color {
    init(hex: UInt32, opacity: Double = 1) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xff) / 255,
            green: Double((hex >> 8) & 0xff) / 255,
            blue: Double(hex & 0xff) / 255,
            opacity: opacity
        )
    }

    /// The brand's ink and paper (`assets/brand/condr.svg`).
    static let ink = Color(hex: 0x1D1D1F)
    static let paper = Color(hex: 0xF5F5F7)
}

/// The terminal's default scheme, condr-core's `DEFAULT_*` colours.
enum Palette {
    static let background: UInt32 = 0x0d1117
    static let foreground: UInt32 = 0xc9d1d9
    static let cursor: UInt32 = 0xf0f6fc
    static let black: UInt32 = 0x484f58
    static let red: UInt32 = 0xff7b72
    static let green: UInt32 = 0x3fb950
    static let yellow: UInt32 = 0xd29922
    static let blue: UInt32 = 0x58a6ff
    static let magenta: UInt32 = 0xbc8cff
    static let cyan: UInt32 = 0x39c5cf
    static let white: UInt32 = 0xb1bac4
    static let gray: UInt32 = 0x6e7681
    static let brightGreen: UInt32 = 0x56d364
    static let brightYellow: UInt32 = 0xe3b341
    static let brightBlue: UInt32 = 0x79c0ff
    static let brightWhite: UInt32 = 0xffffff
    static let claude: UInt32 = 0xD97757
}

// MARK: - Agent status

extension AgentStatus {
    /// The desktop sidebar's tones (`agent_sidebar_status`), as the platform's semantic colours.
    var tint: Color {
        switch self {
        case .working: .orange
        case .blocked: .red
        case .done: .green
        case .idle, .unknown: .secondary
        }
    }

    var title: String {
        switch self {
        case .working: "Working"
        case .blocked: "Needs you"
        case .done: "Done"
        case .idle: "Idle"
        case .unknown: "Not reporting"
        }
    }
}

/// The status glyph the desktop tucks into an Agent mark's corner: a hollow circle for
/// Idle, a filled one for Working, Blocked and Done, an info mark for Unknown.
struct StatusGlyph: View {
    let status: AgentStatus
    var size: CGFloat = 9
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var animating = false

    var body: some View {
        ZStack {
            switch status {
            case .idle:
                Circle().strokeBorder(.secondary, lineWidth: max(1.2, size / 6))
            case .unknown:
                Image(systemName: "info.circle.fill")
                    .resizable()
                    .foregroundStyle(.secondary)
            default:
                Circle().fill(status.tint)
            }
        }
        .frame(width: size, height: size)
        .background {
            if !reduceMotion, status == .blocked {
                Circle()
                    .stroke(status.tint, lineWidth: 1.5)
                    .scaleEffect(animating ? 2.4 : 1)
                    .opacity(animating ? 0 : 0.8)
            }
        }
        .opacity(!reduceMotion && status == .working && animating ? 0.45 : 1)
        .onAppear { animate() }
        .onChange(of: status) { animating = false; animate() }
        .accessibilityLabel(status.title)
    }

    private func animate() {
        guard !reduceMotion else { return }
        switch status {
        case .working:
            withAnimation(.easeInOut(duration: 0.9).repeatForever(autoreverses: true)) { animating = true }
        case .blocked:
            withAnimation(.easeOut(duration: 1.6).repeatForever(autoreverses: false)) { animating = true }
        default:
            break
        }
    }
}

extension Array where Element == AgentStatus {
    /// Most urgent first, the order a score draws them in, so each colour reads as a run.
    func byUrgency() -> [AgentStatus] {
        let rank: [AgentStatus: Int] = [.blocked: 0, .done: 1, .working: 2, .unknown: 3, .idle: 4]
        return sorted { rank[$0]! < rank[$1]! }
    }
}

/// Agents as the logo's bars: a blocked one full height, a finished one two thirds, a
/// working one breathing around half, a resting one low and grey.
struct Bars: View {
    let statuses: [AgentStatus]
    var tall: CGFloat
    var widest: CGFloat = 14
    var spacing: CGFloat = 6
    var animated = true
    var phase: Double = 0
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// The bar width and gap that fit `count` bars in `width`, no wider than `widest`.
    // ponytail: past 2 pt bars they overflow; draw one bar per state when a Device runs that many.
    static func fit(count: Int, width: CGFloat, widest: CGFloat, spacing: CGFloat) -> (width: CGFloat, spacing: CGFloat) {
        guard count > 0 else { return (widest, spacing) }
        let natural = (width - spacing * CGFloat(count - 1)) / CGFloat(count)
        if natural >= 4 { return (min(widest, natural), spacing) }
        return (max(2, (width - 2 * CGFloat(count - 1)) / CGFloat(count)), 2)
    }

    var body: some View {
        TimelineView(.animation(paused: reduceMotion || !animated || !statuses.contains(.working))) { timeline in
            let time = timeline.date.timeIntervalSinceReferenceDate + phase
            HStack(alignment: .bottom, spacing: spacing) {
                ForEach(Array(statuses.enumerated()), id: \.offset) { index, status in
                    RoundedRectangle(cornerRadius: min(4, widest * 0.26), style: .continuous)
                        .fill(status == .idle || status == .unknown ? AnyShapeStyle(.quaternary) : AnyShapeStyle(status.tint.gradient))
                        .frame(width: widest, height: height(status, time: time + Double(index) * 1.1))
                }
            }
            .frame(height: tall, alignment: .bottom)
        }
        .accessibilityHidden(true)
    }

    private func height(_ status: AgentStatus, time: TimeInterval) -> CGFloat {
        switch status {
        case .blocked: tall
        case .done: tall * 0.64
        case .working: tall * (0.48 + (animated ? 0.18 * sin(time * 2.2) : 0))
        case .idle, .unknown: max(3, tall * 0.21)
        }
    }
}

/// "1 needs you · 9 working · 1 done", the need in red; idle only when nothing else is.
func agentSummary(_ statuses: [AgentStatus]) -> Text {
    let count = { (status: AgentStatus) in statuses.filter { $0 == status }.count }
    let blocked = count(.blocked)
    let rest = [(count(.working), "working"), (count(.done), "done")]
        .filter { $0.0 > 0 }
        .map { "\($0.0) \($0.1)" }
        .joined(separator: " · ")
    guard blocked > 0 else {
        if !rest.isEmpty { return Text(rest) }
        return Text(statuses.isEmpty ? "No Agents" : "\(statuses.count) idle")
    }
    let need = Text("\(blocked) \(blocked == 1 ? "needs" : "need") you").foregroundStyle(.red)
    return rest.isEmpty ? need : Text("\(need) · \(rest)")
}

// MARK: - Marks and avatars

/// An Agent's mark from `assets/agents`, in its brand colour where it has one; a shell
/// gets the terminal symbol.
struct AgentMark: View {
    let kind: AgentKind?

    var body: some View {
        if let kind {
            Image(kind.mark)
                .resizable()
                .scaledToFit()
                .foregroundStyle(kind.brand.map { Color(hex: $0) } ?? Color.primary)
        } else {
            Image(systemName: "terminal")
                .resizable()
                .scaledToFit()
                .fontWeight(.medium)
                .foregroundStyle(.secondary)
        }
    }
}

/// A mark on a soft tile with its status glyph on a disc of the surface's colour, so a
/// row reads by Agent first and by state second, as the desktop sidebar does.
struct AgentAvatar: View {
    let kind: AgentKind?
    var status: AgentStatus?
    var side: CGFloat = 40
    var surface: Color = Color(.systemBackground)

    var body: some View {
        RoundedRectangle(cornerRadius: side * 0.3, style: .continuous)
            .fill(.fill.tertiary)
            .overlay { AgentMark(kind: kind).padding(side * 0.24) }
            .frame(width: side, height: side)
            .overlay(alignment: .bottomTrailing) {
                if let status {
                    StatusGlyph(status: status, size: side * 0.27)
                        .padding(side * 0.06)
                        .background(Circle().fill(surface))
                        .offset(x: side * 0.12, y: side * 0.12)
                }
            }
    }
}

/// The desktop's Workspace avatar: the first letter on its identity colour.
struct WorkspaceAvatar: View {
    let name: String
    var side: CGFloat = 22

    var body: some View {
        RoundedRectangle(cornerRadius: side * 0.26, style: .continuous)
            .fill(identityColor(name))
            .overlay {
                Text(name.prefix(1).uppercased())
                    .font(.system(size: side * 0.56, weight: .semibold, design: .rounded))
                    .foregroundStyle(.white)
            }
            .frame(width: side, height: side)
            .accessibilityHidden(true)
    }
}

/// The desktop sidebar's `IDENTITY_COLORS`, picked by a stable hash of `key`.
func identityColor(_ key: String) -> Color {
    let colors: [UInt32] = [0x7a6aa8, 0x3d7ea6, 0x388068, 0xa4673a, 0xb05c80, 0x6a70b8, 0x368080, 0xb06260, 0x8f7838, 0x5179b0]
    var hash: UInt32 = 2_166_136_261
    for byte in key.utf8 { hash = (hash ^ UInt32(byte)) &* 16_777_619 }
    return Color(hex: colors[Int(hash % UInt32(colors.count))])
}

// MARK: - Logo

/// The three bars of `assets/brand/condr.svg`, in its 128-unit square.
struct LogoBar: Shape {
    enum Part { case top, spine, bottom }
    let part: Part

    func path(in rect: CGRect) -> Path {
        let s = min(rect.width, rect.height) / 128
        let r: CGRect = switch part {
        case .top: CGRect(x: 48, y: 24, width: 42, height: 12)
        case .spine: CGRect(x: 35.5, y: 36, width: 12, height: 56)
        case .bottom: CGRect(x: 48, y: 92, width: 42, height: 12)
        }
        let scaled = CGRect(x: rect.minX + r.minX * s, y: rect.minY + r.minY * s, width: r.width * s, height: r.height * s)
        return Path(roundedRect: scaled, cornerRadius: 2 * s, style: .continuous)
    }
}

/// The logo drawn from its bars; `assembled` false gathers them to the spine, so the mark
/// can build itself on first launch.
struct Logo: View {
    var assembled = true
    var color: Color = .primary

    var body: some View {
        ZStack {
            LogoBar(part: .spine)
                .scaleEffect(x: 1, y: assembled ? 1 : 0.2, anchor: .center)
            LogoBar(part: .top)
                .scaleEffect(x: assembled ? 1 : 0, y: 1, anchor: UnitPoint(x: 0.37, y: 0.5))
                .offset(y: assembled ? 0 : 20)
            LogoBar(part: .bottom)
                .scaleEffect(x: assembled ? 1 : 0, y: 1, anchor: UnitPoint(x: 0.37, y: 0.5))
                .offset(y: assembled ? 0 : -20)
        }
        .foregroundStyle(color)
        .aspectRatio(1, contentMode: .fit)
        .accessibilityLabel("Condr")
    }
}

/// The brand's square: the logo on ink, as the app icon draws it.
struct AppMark: View {
    var side: CGFloat = 64
    var assembled = true

    var body: some View {
        RoundedRectangle(cornerRadius: side * 0.225, style: .continuous)
            .fill(Color.ink)
            .overlay { Logo(assembled: assembled, color: .paper) }
            .overlay {
                RoundedRectangle(cornerRadius: side * 0.225, style: .continuous)
                    .strokeBorder(.white.opacity(0.18), lineWidth: 0.75)
            }
            .frame(width: side, height: side)
    }
}

// MARK: - Text

/// `blocked_on` as the hook wrote it (ADR 0024): "Tool: command" for a permission, else
/// a question or the notification's text.
struct BlockedOn {
    let tool: String?
    let text: String

    init(_ raw: String) {
        if let colon = raw.range(of: ": "),
           raw[..<colon.lowerBound].allSatisfy({ $0.isLetter || $0.isNumber || $0 == "_" }) {
            tool = String(raw[..<colon.lowerBound])
            text = String(raw[colon.upperBound...])
        } else {
            tool = nil
            text = raw
        }
    }
}

/// "condr · macmini"
func place(_ workspace: String, _ device: String) -> String { "\(workspace) · \(device)" }

// MARK: - Actions

/// A screen's one main action: the accent, ink in light and paper in dark, with its label
/// in the background's colour so it reads in both.
struct PrimaryButton: View {
    let title: String
    var systemImage: String?
    let action: () -> Void

    init(_ title: String, systemImage: String? = nil, action: @escaping () -> Void) {
        self.title = title
        self.systemImage = systemImage
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            Group {
                if let systemImage {
                    Label(title, systemImage: systemImage)
                } else {
                    Text(title)
                }
            }
            .font(.headline)
            .foregroundStyle(Color(.systemBackground))
            .frame(maxWidth: .infinity, minHeight: 34)
        }
        .buttonStyle(.glassProminent)
        .controlSize(.large)
    }
}
