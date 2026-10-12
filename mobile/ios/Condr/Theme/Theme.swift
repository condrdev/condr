import CondrKit
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

/// The terminal's default scheme, condr-core's `DEFAULT_*` colours, for what the app draws
/// around the terminal: the Settings preview and the page behind the grid.
enum Palette {
    static let background: UInt32 = 0x0d1117
    static let foreground: UInt32 = 0xc9d1d9
    static let green: UInt32 = 0x3fb950
    static let gray: UInt32 = 0x6e7681
    static let brightWhite: UInt32 = 0xffffff
}

/// The status glyph the desktop tucks into an Agent mark's corner: a hollow circle for
/// Idle, a filled one for Working, Blocked and Done, an info mark for Unknown.
struct StatusGlyph: View {
    let status: AgentStatus
    var size: CGFloat = 9
    /// A Blocked glyph ripples only where one wait is the subject: a terminal's title, a banner.
    var ripple = false
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
            if !reduceMotion, ripple, status == .blocked {
                Circle()
                    .stroke(status.tint, lineWidth: 1.5)
                    .scaleEffect(animating ? 2.4 : 1)
                    .opacity(animating ? 0 : 0.8)
            }
        }
        .onAppear { animate() }
        .onChange(of: status) {
            animating = false; animate()
        }
        .accessibilityLabel(status.title)
    }

    private func animate() {
        guard !reduceMotion else { return }
        switch status {
        case .blocked:
            withAnimation(.easeOut(duration: 1.6).repeatForever(autoreverses: false)) {
                animating = true
            }
        default:
            break
        }
    }
}

/// Agents as the logo's bars, a height band per state so shape tells them apart as well as
/// colour: a blocked one full height, a finished one three quarters, a working one
/// breathing between a quarter and a half, a resting one low and grey. Every bar keeps the same
/// clock and `first` places this run in a longer row, so working bars rise in one wave
/// travelling left to right across every computer's column.
struct Bars: View {
    let statuses: [AgentStatus]
    var tall: CGFloat
    var widest: CGFloat = 14
    var spacing: CGFloat = 6
    var animated = true
    var first = 0
    /// While someone scrubs the score: the bar under their finger here, or none of these.
    var highlight: Int?? = nil
    /// How far the header has folded as the screen scrolls, 1 open to 0.3 folded: heights
    /// shrink, corners stay round.
    var fold: CGFloat = 1
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityDifferentiateWithoutColor) private var withoutColor

    /// The bar width and gap that fit `count` bars in `width`, no wider than `widest`.
    // ponytail: past 2 pt bars they overflow; draw one bar per state when a Device runs that many.
    static func fit(count: Int, width: CGFloat, widest: CGFloat, spacing: CGFloat) -> (
        width: CGFloat, spacing: CGFloat
    ) {
        guard count > 0 else { return (widest, spacing) }
        let natural = (width - spacing * CGFloat(count - 1)) / CGFloat(count)
        if natural >= 4 { return (min(widest, natural), spacing) }
        return (max(2, (width - 2 * CGFloat(count - 1)) / CGFloat(count)), 2)
    }

    var body: some View {
        TimelineView(
            .animation(
                minimumInterval: 1 / 30,
                paused: reduceMotion || !animated || !statuses.contains(.working))
        ) { timeline in
            let time = timeline.date.timeIntervalSinceReferenceDate
            HStack(alignment: .bottom, spacing: spacing) {
                ForEach(Array(statuses.enumerated()), id: \.offset) { index, status in
                    let shape = RoundedRectangle(
                        cornerRadius: min(4, widest * 0.26), style: .continuous)
                    Group {
                        if withoutColor, status == .done, widest >= 6 {
                            shape.strokeBorder(status.tint, lineWidth: 2)
                        } else {
                            shape.fill(
                                status == .idle || status == .unknown
                                    ? AnyShapeStyle(.tertiary) : AnyShapeStyle(status.tint.gradient)
                            )
                        }
                    }
                    .frame(
                        width: widest,
                        height: max(
                            widest * 0.8,
                            height(status, time: time - Double(first + index) * 0.32) * fold)
                    )
                    .opacity(highlight.map { $0 == index } == false ? 0.28 : 1)
                    .overlay(alignment: .bottom) {
                        if highlight.map({ $0 == index }) == true {
                            Capsule()
                                .fill(.primary)
                                .frame(width: max(8, widest + 4), height: 3)
                                .offset(y: 7)
                                .transition(.opacity)
                        }
                    }
                    .animation(.snappy(duration: 0.15), value: highlight.map { $0 == index })
                }
            }
            .frame(height: tall, alignment: .bottom)
        }
        .accessibilityHidden(true)
    }

    private func height(_ status: AgentStatus, time: TimeInterval) -> CGFloat {
        switch status {
        case .idle, .unknown: min(tall * 0.2, max(3, tall * 0.12, widest * 2))
        default: Self.height(status, tall: tall, wave: animated ? sin(time * 2.4) : 0)
        }
    }

    /// The band each state's bar stands in, `wave` in -1...1 breathing a working one.
    static func height(_ status: AgentStatus, tall: CGFloat, wave: Double) -> CGFloat {
        switch status {
        case .blocked: tall
        case .done: tall * 0.75
        case .working: tall * (0.35 + 0.10 * wave)
        case .idle, .unknown: tall * 0.12
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
        return Text(statuses.isEmpty ? "No agents" : "\(statuses.count) idle")
    }
    let need = Text("\(blocked) \(blocked == 1 ? "needs" : "need") you").fontWeight(.semibold)
        .foregroundStyle(AgentStatus.blocked.tint)
    return rest.isEmpty ? need : Text("\(need) · \(rest)")
}

/// A computer it can't reach, everywhere the same: three bars of falling height toppled
/// flat along the baseline.
struct Fallen: View {
    var width: CGFloat = 40

    var body: some View {
        let gap = max(2, width * 0.06)
        let lengths: [CGFloat] = [1, 0.72, 0.44]
        let unit = (width - gap * 2) / lengths.reduce(0, +)
        HStack(spacing: gap) {
            ForEach(lengths, id: \.self) { length in
                RoundedRectangle(cornerRadius: 1.5, style: .continuous)
                    .fill(.tertiary)
                    .frame(width: unit * length, height: max(3, width * 0.05))
            }
        }
        .frame(width: width, alignment: .leading)
        .accessibilityHidden(true)
    }
}

// MARK: - Marks and avatars

/// An Agent's mark from `assets/agents`, in the text colour: on the phone colour means
/// state alone, so Claude's terracotta would read as Working (the desktop keeps it). A shell
/// gets the terminal symbol.
struct AgentMark: View {
    /// The Agent's kind id; none for a shell.
    let kind: String?

    var body: some View {
        if let mark = AgentMarks.resource(for: kind) {
            Image(mark)
                .resizable()
                .scaledToFit()
                .foregroundStyle(Color.primary)
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
    let kind: String?
    var status: AgentStatus?
    var side: CGFloat = 40
    var surface: Color = Color(.systemBackground)
    var ripple = false

    var body: some View {
        RoundedRectangle(cornerRadius: side * 0.3, style: .continuous)
            .fill(.fill.tertiary)
            .overlay { AgentMark(kind: kind).padding(side * 0.24) }
            .frame(width: side, height: side)
            .overlay(alignment: .bottomTrailing) {
                if let status {
                    StatusGlyph(status: status, size: side * 0.27, ripple: ripple)
                        .padding(side * 0.06)
                        .background(Circle().fill(surface))
                        .offset(x: side * 0.12, y: side * 0.12)
                }
            }
    }
}

/// The desktop's Workspace avatar, the first letter, on a neutral tile where the desktop
/// gives it an identity colour: on the phone colour means state alone.
struct WorkspaceAvatar: View {
    let name: String
    var side: CGFloat = 22

    var body: some View {
        RoundedRectangle(cornerRadius: side * 0.26, style: .continuous)
            .fill(.fill.secondary)
            .overlay {
                Text(name.prefix(1).uppercased())
                    .font(.system(size: side * 0.56, weight: .semibold, design: .rounded))
                    .foregroundStyle(.primary)
            }
            .frame(width: side, height: side)
            .accessibilityHidden(true)
    }
}

// MARK: - Logo

/// The three bars of `assets/brand/condr.svg`, in its 128-unit square.
nonisolated struct LogoBar: Shape {
    enum Part { case top, spine, bottom }
    let part: Part

    func path(in rect: CGRect) -> Path {
        let s = min(rect.width, rect.height) / 128
        let r: CGRect =
            switch part {
            case .top: CGRect(x: 48, y: 24, width: 42, height: 12)
            case .spine: CGRect(x: 35.5, y: 36, width: 12, height: 56)
            case .bottom: CGRect(x: 48, y: 92, width: 42, height: 12)
            }
        let scaled = CGRect(
            x: rect.minX + r.minX * s, y: rect.minY + r.minY * s, width: r.width * s,
            height: r.height * s)
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
