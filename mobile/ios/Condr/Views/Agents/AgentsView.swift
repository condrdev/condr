import CondrKit
import SwiftUI

/// The first screen (ADR 0041): every computer's Agents at a glance, then "Needs you",
/// then every Agent by what it is doing, across every paired Device.
struct AgentsView: View {
    @Environment(Session.self) private var session
    @Environment(Router.self) private var router
    @Namespace private var zoom
    @State private var scrubbing = false
    /// `-offset 80` opens the screen already scrolled, to review how the score folds.
    @State private var position = ScrollPosition(
        point: CGPoint(x: 0, y: UserDefaults.standard.double(forKey: "offset")))

    var body: some View {
        @Bindable var router = router
        NavigationStack(path: $router.agentsPath) {
            ScrollView {
                VStack(alignment: .leading, spacing: 30) {
                    Score(zoom: zoom, scrubbing: $scrubbing)
                    needsYou
                    group("Done", session.agents(.done))
                    group("Working", session.agents(.working))
                    group("Idle", session.agents(.idle, .unknown))
                }
                .padding(.horizontal, 16)
                .padding(.top, 14)
                .padding(.bottom, 24)
                .animation(.smooth, value: session.agents.map(\.agent.status))
            }
            .scrollPosition($position)
            .scrollDisabled(scrubbing)
            .coordinateSpace(.named(Score.space))
            .background(Color(.systemGroupedBackground))
            .overlay(alignment: .top) { StatusBarFade() }
            // The tab names the screen and the score heads it; the title stays for the
            // back menu and VoiceOver.
            .navigationTitle("Agents")
            .toolbarVisibility(.hidden, for: .navigationBar)
            .navigationDestination(for: DeviceRoute.self) { route in
                switch route {
                case .device(let address):
                    DeviceScreen(address: address)
                        .navigationTransition(.zoom(sourceID: address, in: zoom))
                case .pane(let ref):
                    TerminalPage(ref: ref)
                        .navigationTransition(.zoom(sourceID: ref, in: zoom))
                }
            }
        }
    }

    private var needsYou: some View {
        let items = session.agents(.blocked)
        return VStack(alignment: .leading, spacing: 10) {
            SectionTitle("Needs you", count: items.count, tint: AgentStatus.blocked.tint)
            if items.isEmpty {
                AllClear(working: session.agents(.working).count)
            } else {
                ForEach(items) { item in
                    NavigationLink(value: DeviceRoute.pane(item.ref)) {
                        NeedsYouCard(item: item)
                    }
                    .buttonStyle(CardPress())
                    .matchedTransitionSource(id: item.ref, in: zoom)
                    .transition(
                        .asymmetric(
                            insertion: .scale(scale: 0.96).combined(with: .opacity),
                            removal: .opacity))
                }
            }
        }
    }

    @ViewBuilder
    private func group(_ title: LocalizedStringResource, _ items: [AgentItem]) -> some View {
        if !items.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                SectionTitle(title, count: items.count, tint: .secondary)
                Card {
                    ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                        NavigationLink(value: DeviceRoute.pane(item.ref)) {
                            AgentRow(item: item)
                        }
                        .buttonStyle(RowPress())
                        .matchedTransitionSource(id: item.ref, in: zoom)
                        if index < items.count - 1 {
                            Divider().padding(.leading, 66)
                        }
                    }
                }
            }
        }
    }
}

/// The grouped surface the first screen's rows sit on.
struct Card<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        VStack(spacing: 0) { content }
            .background(
                Color(.secondarySystemGroupedBackground),
                in: .rect(cornerRadius: 26, style: .continuous))
    }
}

struct SectionTitle: View {
    let title: LocalizedStringResource
    let count: Int
    let tint: Color

    init(_ title: LocalizedStringResource, count: Int, tint: Color) {
        self.title = title
        self.count = count
        self.tint = tint
    }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 7) {
            Text(title)
                .font(.title3.weight(.semibold))
            if count > 0 {
                Text("\(count)")
                    .font(.subheadline.weight(.semibold).monospacedDigit())
                    .foregroundStyle(tint == .secondary ? Color.secondary : tint)
                    .contentTransition(.numericText())
            }
        }
        .padding(.horizontal, 4)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}

/// With no navigation bar, content scrolling up fades out under the status bar instead
/// of crossing its time and battery.
struct StatusBarFade: View {
    var body: some View {
        Color.clear
            .frame(height: 10)
            .background {
                LinearGradient(
                    stops: [
                        .init(color: Color(.systemGroupedBackground), location: 0),
                        .init(color: Color(.systemGroupedBackground), location: 0.55),
                        .init(color: Color(.systemGroupedBackground).opacity(0), location: 1),
                    ], startPoint: .top, endPoint: .bottom
                )
                .ignoresSafeArea(edges: .top)
            }
            .allowsHitTesting(false)
            .accessibilityHidden(true)
    }
}

// MARK: - Cards and rows

struct NeedsYouCard: View {
    let item: AgentItem
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            AgentHeading(item: item, status: .blocked, side: 42, titleFont: .headline)
            if let raw = item.agent.blockedOn {
                BlockedOnView(blockedOn: BlockedOn(raw))
            }
        }
        .padding(16)
        .background(
            Color(.secondarySystemGroupedBackground),
            in: .rect(cornerRadius: 26, style: .continuous)
        )
        .contentShape(.rect(cornerRadius: 26))
        .accessibilityElement(children: .combine)
        .accessibilityHint("Opens the terminal")
    }
}

/// An Agent's avatar, its Pane's title, and "kind · workspace · computer" with a chevron:
/// the head of a "Needs you" card and the whole of a row.
struct AgentHeading: View {
    let item: AgentItem
    let status: AgentStatus
    let side: CGFloat
    let titleFont: Font
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        HStack(spacing: side > 40 ? 13 : 14) {
            AgentAvatar(
                kind: item.agent.kind, status: status, side: side,
                surface: Color(.secondarySystemGroupedBackground))
            VStack(alignment: .leading, spacing: 2) {
                Text(item.pane.name)
                    .font(titleFont)
                    .lineLimit(typeSize.isAccessibilitySize ? 3 : 1)
                Text("\(item.agent.label) · \(place(item.workspace, item.device))")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(typeSize.isAccessibilitySize ? 3 : 1)
            }
            Spacer(minLength: 0)
            Image(systemName: "chevron.right")
                .font(.footnote.weight(.semibold))
                .foregroundStyle(.tertiary)
        }
    }
}

/// What a Blocked Agent waits for: a tool and its command, or a question.
struct BlockedOnView: View {
    let blockedOn: BlockedOn

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            Text(blockedOn.tool == nil ? "ASKS" : "WANTS TO RUN")
                .font(.caption2.weight(.semibold))
                .tracking(0.8)
                .foregroundStyle(.secondary)
            if let tool = blockedOn.tool {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(tool)
                        .font(.caption.weight(.semibold))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(.fill.secondary, in: .rect(cornerRadius: 5, style: .continuous))
                        .foregroundStyle(.secondary)
                    Text(blockedOn.text)
                        .font(.system(.callout, design: .monospaced))
                        .lineLimit(3)
                }
            } else {
                Text(blockedOn.text)
                    .font(.body.weight(.medium))
                    .lineLimit(3)
            }
        }
        .padding(.horizontal, 13)
        .padding(.vertical, 11)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            Color(.tertiarySystemGroupedBackground), in: .rect(cornerRadius: 16, style: .continuous)
        )
    }
}

struct AgentRow: View {
    let item: AgentItem

    var body: some View {
        AgentHeading(
            item: item, status: item.agent.status, side: 36, titleFont: .body.weight(.medium)
        )
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .contentShape(.rect)
        .accessibilityElement(children: .combine)
    }
}

struct AllClear: View {
    let working: Int

    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: "checkmark")
                .font(.system(size: 15, weight: .bold))
                .foregroundStyle(.secondary)
                .frame(width: 42, height: 42)
                .background(.fill.tertiary, in: .rect(cornerRadius: 13, style: .continuous))
            VStack(alignment: .leading, spacing: 2) {
                Text("Nothing needs you")
                    .font(.headline)
                Text(
                    working > 0
                        ? "Condr brings an agent here the moment it waits for you."
                        : "Every agent is at rest."
                )
                .font(.subheadline)
                .foregroundStyle(.secondary)
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            Color(.secondarySystemGroupedBackground),
            in: .rect(cornerRadius: 26, style: .continuous))
    }
}

struct CardPress: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.97 : 1)
            .animation(.snappy(duration: 0.2), value: configuration.isPressed)
    }
}

struct RowPress: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .background(configuration.isPressed ? Color(.systemFill) : .clear)
    }
}
