// xcode: set sdk=iOS

import SwiftUI

/// The first screen (ADR 0041): every computer's Agents at a glance, then "Needs you",
/// then every Agent by what it is doing, across every paired Device.
struct AgentsView: View {
    @Environment(Store.self) private var store
    @Namespace private var zoom

    var body: some View {
        @Bindable var store = store
        NavigationStack(path: $store.agentsPath) {
            ScrollView {
                VStack(alignment: .leading, spacing: 30) {
                    Score(zoom: zoom)
                    needsYou
                    group("Done", store.agents(.done))
                    group("Working", store.agents(.working))
                    group("Idle", store.agents(.idle, .unknown))
                }
                .padding(.horizontal, 16)
                .padding(.top, 14)
                .padding(.bottom, 24)
                .animation(.smooth, value: store.agents.map(\.agent.status))
            }
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
                    TerminalScreen(ref: ref)
                        .navigationTransition(.zoom(sourceID: ref, in: zoom))
                }
            }
        }
    }

    private var needsYou: some View {
        let items = store.agents(.blocked)
        return VStack(alignment: .leading, spacing: 10) {
            SectionTitle("Needs you", count: items.count, tint: .red)
            if items.isEmpty {
                AllClear(working: store.agents(.working).count)
            } else {
                ForEach(items) { item in
                    NavigationLink(value: DeviceRoute.pane(item.ref)) {
                        NeedsYouCard(item: item)
                    }
                    .buttonStyle(CardPress())
                    .matchedTransitionSource(id: item.ref, in: zoom)
                    .transition(.asymmetric(insertion: .scale(scale: 0.96).combined(with: .opacity), removal: .opacity))
                }
            }
        }
    }

    @ViewBuilder
    private func group(_ title: String, _ items: [AgentItem]) -> some View {
        if !items.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                SectionTitle(title, count: items.count, tint: items[0].agent.status.tint)
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
            .background(Color(.secondarySystemGroupedBackground), in: .rect(cornerRadius: 26, style: .continuous))
    }
}

struct SectionTitle: View {
    let title: String
    let count: Int
    let tint: Color

    init(_ title: String, count: Int, tint: Color) {
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

// MARK: - The score

/// The head of the first screen, in place of a title: each computer as a column of the
/// logo's bars, one per Agent, most urgent first, so a colour reads as a run however many
/// there are. A column is one target and opens its computer; the Agents themselves are
/// the rows below.
struct Score: View {
    let zoom: Namespace.ID
    @Environment(Store.self) private var store

    var body: some View {
        ScrollView(.horizontal) {
            HStack(alignment: .bottom, spacing: 22) {
                ForEach(Array(store.devices.enumerated()), id: \.element.id) { index, device in
                    NavigationLink(value: DeviceRoute.device(device.address)) {
                        column(device, phase: Double(index) * 0.7)
                    }
                    .buttonStyle(ColumnPress())
                    .matchedTransitionSource(id: device.address, in: zoom)
                }
            }
            .padding(.horizontal, 20)
        }
        .scrollIndicators(.hidden)
        .scrollClipDisabled()
        .padding(.horizontal, -16)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Agents")
        .accessibilityAddTraits(.isHeader)
    }

    private func column(_ device: Device, phase: Double) -> some View {
        let statuses = device.agents.compactMap { $0.agent?.status }.byUrgency()
        // ponytail: a column stops growing at 120 pt; past ~40 Agents the bars hit 2 pt and overflow.
        let fit = Bars.fit(count: statuses.count, width: 120, widest: 14, spacing: 6)
        let connected = device.status == .connected
        return VStack(alignment: .leading, spacing: 10) {
            Group {
                if connected, !statuses.isEmpty {
                    Bars(statuses: statuses, tall: 56, widest: fit.width, spacing: fit.spacing, phase: phase)
                } else {
                    RoundedRectangle(cornerRadius: 3.5, style: .continuous)
                        .strokeBorder(style: StrokeStyle(lineWidth: 1.2, dash: [3, 3]))
                        .foregroundStyle(.tertiary)
                        .frame(width: 14, height: 20)
                }
            }
            .frame(height: 56, alignment: .bottom)
            HStack(spacing: 4) {
                if !connected {
                    Image(systemName: "bolt.horizontal.fill")
                        .font(.system(size: 8, weight: .bold))
                }
                Text(device.label)
            }
            .font(.caption.weight(.semibold))
            .foregroundStyle(connected ? .secondary : .tertiary)
            .fixedSize()
        }
        .frame(minWidth: 44, alignment: .leading)
        .contentShape(.rect.inset(by: -6))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(device.label), \(spoken(device, statuses))")
        .accessibilityHint("Opens the computer")
        .accessibilityAddTraits(.isButton)
    }

    private func spoken(_ device: Device, _ statuses: [AgentStatus]) -> String {
        switch device.status {
        case .connected:
            let parts = [(AgentStatus.blocked, "need you"), (.working, "working"), (.done, "done"), (.idle, "idle")]
                .map { status, words in (statuses.filter { $0 == status }.count, words) }
                .filter { $0.0 > 0 }
                .map { "\($0.0) \($0.1)" }
            return parts.isEmpty ? "no Agents" : parts.joined(separator: ", ")
        case .connecting: return "connecting"
        case .failed: return "can't connect"
        case .revoked: return "needs pairing again"
        }
    }
}

struct ColumnPress: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.92 : 1, anchor: .bottom)
            .opacity(configuration.isPressed ? 0.7 : 1)
            .animation(.snappy(duration: 0.2), value: configuration.isPressed)
    }
}

/// With no navigation bar, content scrolling up fades out under the status bar instead
/// of crossing its time and battery.
struct StatusBarFade: View {
    var body: some View {
        Color.clear
            .frame(height: 10)
            .background {
                LinearGradient(stops: [
                    .init(color: Color(.systemGroupedBackground), location: 0),
                    .init(color: Color(.systemGroupedBackground), location: 0.55),
                    .init(color: Color(.systemGroupedBackground).opacity(0), location: 1),
                ], startPoint: .top, endPoint: .bottom)
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
            HStack(spacing: 13) {
                AgentAvatar(kind: item.agent.kind, status: .blocked, side: 42, surface: Color(.secondarySystemGroupedBackground))
                VStack(alignment: .leading, spacing: 2) {
                    Text(item.pane.title)
                        .font(.headline)
                        .lineLimit(typeSize.isAccessibilitySize ? 3 : 1)
                    Text("\(item.agent.kind.label) · \(place(item.workspace, item.device))")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(typeSize.isAccessibilitySize ? 3 : 1)
                }
                Spacer(minLength: 0)
                Image(systemName: "chevron.right")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.tertiary)
            }
            if let raw = item.agent.blockedOn {
                BlockedOnView(blockedOn: BlockedOn(raw))
            }
        }
        .padding(16)
        .background(Color(.secondarySystemGroupedBackground), in: .rect(cornerRadius: 26, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 26, style: .continuous)
                .strokeBorder(.red.opacity(0.22), lineWidth: 1)
        }
        .contentShape(.rect(cornerRadius: 26))
        .accessibilityElement(children: .combine)
        .accessibilityHint("Opens the terminal")
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
                .foregroundStyle(.red)
            if let tool = blockedOn.tool {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(tool)
                        .font(.caption.weight(.semibold))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(.red.opacity(0.12), in: .rect(cornerRadius: 5, style: .continuous))
                        .foregroundStyle(.red)
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
        .background(Color(.tertiarySystemGroupedBackground), in: .rect(cornerRadius: 16, style: .continuous))
    }
}

struct AgentRow: View {
    let item: AgentItem
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        HStack(spacing: 14) {
            AgentAvatar(kind: item.agent.kind, status: item.agent.status, side: 36, surface: Color(.secondarySystemGroupedBackground))
            VStack(alignment: .leading, spacing: 2) {
                Text(item.pane.title)
                    .font(.body.weight(.medium))
                    .lineLimit(typeSize.isAccessibilitySize ? 3 : 1)
                Text("\(item.agent.kind.label) · \(place(item.workspace, item.device))")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(typeSize.isAccessibilitySize ? 3 : 1)
            }
            Spacer(minLength: 0)
            if item.agent.status == .done {
                Text("Done")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.green)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .background(.green.opacity(0.12), in: .capsule)
            }
            Image(systemName: "chevron.right")
                .font(.footnote.weight(.semibold))
                .foregroundStyle(.tertiary)
        }
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
                .foregroundStyle(.green)
                .frame(width: 42, height: 42)
                .background(.green.opacity(0.12), in: .rect(cornerRadius: 13, style: .continuous))
            VStack(alignment: .leading, spacing: 2) {
                Text("Nothing needs you")
                    .font(.headline)
                Text(working > 0 ? "Condr brings an Agent here the moment it waits for you." : "Every Agent is at rest.")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(.secondarySystemGroupedBackground), in: .rect(cornerRadius: 26, style: .continuous))
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
