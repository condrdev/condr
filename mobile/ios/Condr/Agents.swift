// xcode: set sdk=iOS

import SwiftUI

/// The first screen (ADR 0041): every computer's Agents at a glance, then "Needs you",
/// then every Agent by what it is doing, across every paired Device.
struct AgentsView: View {
    @Environment(Store.self) private var store
    @Namespace private var zoom
    @State private var scrubbing = false
    /// `-offset 80` opens the screen already scrolled, to review how the score folds.
    @State private var position = ScrollPosition(point: CGPoint(x: 0, y: UserDefaults.standard.double(forKey: "offset")))

    var body: some View {
        @Bindable var store = store
        NavigationStack(path: $store.agentsPath) {
            ScrollView {
                VStack(alignment: .leading, spacing: 30) {
                    Score(zoom: zoom, scrubbing: $scrubbing)
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
/// there are. A tap on a column opens its computer. Press and slide to scrub the bars: each
/// Agent passed under the finger ticks and names itself, and letting go opens its terminal.
struct Score: View {
    let zoom: Namespace.ID
    /// Whether a scrub is under way, for the page to hold still meanwhile.
    @Binding var scrubbing: Bool
    @Environment(Store.self) private var store
    /// The first screen's scroll space, where the score measures how far it has been lifted.
    static let space = "agents"
    /// The score row's own space, which scrubbing reads the finger and the columns in.
    private static let row = "score"

    @State private var columns: [String: CGRect] = [:]
    @GestureState private var held = false
    @GestureState private var touch: CGPoint?
    /// A scrub picked an Agent, so the column's own tap must not open its computer as well.
    @State private var scrubbed = false
    /// `-scrub "<device index>:<bar index>"` holds a scrub for review.
    @State private var staged: PaneRef?

    private var active: Bool { held || staged != nil }
    private var scrub: PaneRef? { touch.flatMap(pane(at:)) ?? (held ? nil : staged) }
    @State private var edges = [false, false]
    @State private var scrolled: CGFloat = 0
    /// 1 at rest, down to 0.3 as the screen scrolls up: the header folds toward its baseline.
    @State private var fold: CGFloat = 1

    /// Tall while few computers share the row, so the motif heads the screen; lower once
    /// there are more than fit.
    private var tall: CGFloat { store.devices.count <= 3 ? 88 : 56 }

    var body: some View {
        ScrollView(.horizontal) {
            HStack(alignment: .bottom, spacing: 24) {
                ForEach(Array(store.devices.enumerated()), id: \.element.id) { index, device in
                    // A tap is the column's button; pressing and sliding scrubs on top of it.
                    Button {
                        if !scrubbed { store.agentsPath.append(.device(device.address)) }
                    } label: {
                        column(device, first: store.devices[..<index].reduce(0) { $0 + $1.agents.count })
                    }
                    .buttonStyle(ColumnPress(pressable: !active))
                    .simultaneousGesture(scrubGesture)
                    .matchedTransitionSource(id: device.address, in: zoom)
                }
            }
            .padding(.horizontal, 20)
            .coordinateSpace(.named(Score.row))
        }
        .scrollIndicators(.hidden)
        .scrollDisabled(active)
        .onScrollGeometryChange(for: [Bool].self) { geometry in
            [geometry.contentOffset.x > 1,
             geometry.contentOffset.x + geometry.containerSize.width < geometry.contentSize.width - 1]
        } action: { _, edges in
            withAnimation(.smooth) { self.edges = edges }
        }
        .onScrollGeometryChange(for: CGFloat.self) { $0.contentOffset.x } action: { _, x in scrolled = x }
        .onGeometryChange(for: CGFloat.self) { $0.frame(in: .named(Score.space)).minY } action: { minY in
            fold = max(0.3, 1 - max(0, 14 - minY) / 150)
        }
        // Columns past either screen edge fade out, so a computer off screen reads as more
        // to swipe to; with everything in view nothing fades.
        .mask {
            HStack(spacing: 0) {
                LinearGradient(colors: [.clear, .black], startPoint: .leading, endPoint: .trailing)
                    .frame(width: edges[0] ? 36 : 0)
                Rectangle()
                LinearGradient(colors: [.black, .clear], startPoint: .leading, endPoint: .trailing)
                    .frame(width: edges[1] ? 56 : 0)
            }
        }
        .padding(.horizontal, -16)
        .overlay { GeometryReader { proxy in caption(in: proxy.size.width) } }
        .sensoryFeedback(.selection, trigger: scrub)
        .sensoryFeedback(.impact(weight: .medium), trigger: held) { _, now in now }
        .animation(.snappy, value: active)
        .onChange(of: held) { _, now in
            scrubbing = now
            // Once the touch has ended or been cancelled, and the column's button has seen it.
            if !now { DispatchQueue.main.async { scrubbed = false } }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Agents")
        .accessibilityAddTraits(.isHeader)
        .onAppear(perform: stage)
    }

    private func column(_ device: Device, first: Int) -> some View {
        let panes = urgent(device)
        let statuses = panes.compactMap { $0.agent?.status }
        let fit = fitting(statuses.count)
        let connected = device.status == .connected
        // Not scrubbing: every bar as usual. Scrubbing: the bar under the finger, none here.
        let highlight: Int??
        if !active {
            highlight = nil
        } else if scrub?.device == device.address {
            highlight = .some(panes.firstIndex { $0.id == scrub?.pane })
        } else {
            highlight = .some(nil)
        }
        return VStack(alignment: .leading, spacing: 10) {
            Group {
                if connected, !statuses.isEmpty {
                    Bars(statuses: statuses, tall: tall, widest: fit.width, spacing: fit.spacing, first: first, highlight: highlight, fold: fold)
                } else {
                    Fallen(width: 40)
                }
            }
            .frame(height: tall, alignment: .bottom)
            .onGeometryChange(for: CGRect.self) { $0.frame(in: .named(Score.row)) } action: { columns[device.address] = $0 }
            HStack(spacing: 4) {
                if !connected {
                    Image(systemName: device.status == .revoked ? "key.fill" : "bolt.horizontal.fill")
                        .font(.system(size: 8, weight: .bold))
                }
                Text(device.label)
            }
            .font(.caption.weight(.semibold))
            .foregroundStyle(connected ? .secondary : .tertiary)
            .fixedSize()
            .opacity(active ? 0.4 : 1)
        }
        .frame(minWidth: 44, alignment: .leading)
        .contentShape(.rect.inset(by: -6))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(device.label), \(spoken(device, statuses))")
        .accessibilityHint("Opens the computer")
    }

    /// A device's Agents in the order its bars are drawn.
    private func urgent(_ device: Device) -> [Pane] {
        let rank: [AgentStatus: Int] = [.blocked: 0, .done: 1, .working: 2, .unknown: 3, .idle: 4]
        return device.agents.enumerated()
            .sorted { (rank[$0.element.agent!.status]!, $0.offset) < (rank[$1.element.agent!.status]!, $1.offset) }
            .map(\.element)
    }

    // ponytail: a column stops growing at 160 pt, which keeps bars 4 pt wide to ~26 Agents
    // and 2 pt to ~50; past that they overflow.
    private func fitting(_ count: Int) -> (width: CGFloat, spacing: CGFloat) {
        Bars.fit(count: count, width: 160, widest: 18, spacing: 6)
    }

    private var scrubGesture: some Gesture {
        LongPressGesture(minimumDuration: 0.3)
            .sequenced(before: DragGesture(minimumDistance: 0, coordinateSpace: .named(Score.row)))
            .updating($held) { value, held, _ in
                if case .second(true, _) = value { held = true }
            }
            .updating($touch) { value, touch, _ in
                if case .second(true, let drag?) = value { touch = drag.location }
            }
            .onChanged { value in
                if case .second(true, let drag?) = value, pane(at: drag.location) != nil { scrubbed = true }
            }
            .onEnded { value in
                if case .second(true, let drag?) = value, let ref = pane(at: drag.location) {
                    store.agentsPath = [.pane(ref)]
                }
            }
    }

    /// The Agent whose bar is nearest the finger, in whichever column it is over; none once
    /// the finger has slid well above or below the bars, which is how a scrub is cancelled.
    private func pane(at point: CGPoint) -> PaneRef? {
        for device in store.devices where device.status == .connected {
            guard let frame = columns[device.address], point.x >= frame.minX - 12, point.x <= frame.maxX + 12,
                  point.y >= frame.minY - 44, point.y <= frame.maxY + 64 else { continue }
            let panes = urgent(device)
            guard !panes.isEmpty else { return nil }
            let fit = fitting(panes.count)
            let index = Int((point.x - frame.minX) / (fit.width + fit.spacing))
            return PaneRef(device: device.address, pane: panes[min(max(index, 0), panes.count - 1)].id)
        }
        return nil
    }

    /// Where the scrubbed bar's centre is on screen.
    private func screenX(of ref: PaneRef) -> CGFloat? {
        guard let device = store.device(ref.device), let frame = columns[ref.device],
              let index = urgent(device).firstIndex(where: { $0.id == ref.pane }) else { return nil }
        let fit = fitting(device.agents.count)
        return frame.minX + CGFloat(index) * (fit.width + fit.spacing) + fit.width / 2 - scrolled
    }

    /// What the finger is on while scrubbing: level with the bars and off to the side away
    /// from the finger, so the hand on the bars never covers it.
    @ViewBuilder
    private func caption(in width: CGFloat) -> some View {
        if active {
            let x = scrub.flatMap(screenX) ?? width / 2
            let away = x < width / 2 ? x + 118 : x - 118
            HStack(spacing: 9) {
                if let scrub, let pane = store.pane(scrub), let agent = pane.agent, let device = store.device(scrub.device) {
                    AgentMark(kind: agent.kind)
                        .frame(width: 16, height: 16)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(pane.title)
                            .font(.subheadline.weight(.semibold))
                            .lineLimit(1)
                        HStack(spacing: 5) {
                            StatusGlyph(status: agent.status, size: 6)
                            Text("\(agent.status.title) · \(device.label)")
                        }
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    }
                } else {
                    Text(touch == nil ? "Slide to an agent" : "Let go to cancel")
                        .font(.subheadline.weight(.medium))
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
            .background(Color(.secondarySystemGroupedBackground), in: .capsule)
            .shadow(color: .black.opacity(0.14), radius: 14, y: 6)
            .fixedSize()
            .position(x: min(max(away, 110), width - 110), y: tall * 0.45)
            .animation(.snappy(duration: 0.2), value: scrub)
            .transition(.scale(scale: 0.85).combined(with: .opacity))
            .allowsHitTesting(false)
            .accessibilityHidden(true)
        }
    }

    /// `-scrub "<device index>:<bar index>"` holds a scrub for review.
    private func stage() {
        guard let spec = UserDefaults.standard.string(forKey: "scrub") else { return }
        let parts = spec.split(separator: ":").compactMap { Int($0) }
        guard parts.count == 2, store.devices.indices.contains(parts[0]) else { return }
        let device = store.devices[parts[0]]
        let panes = urgent(device)
        guard panes.indices.contains(parts[1]) else { return }
        staged = PaneRef(device: device.address, pane: panes[parts[1]].id)
    }

    private func spoken(_ device: Device, _ statuses: [AgentStatus]) -> String {
        switch device.status {
        case .connected:
            let parts = [(AgentStatus.blocked, "need you"), (.working, "working"), (.done, "done"), (.idle, "idle")]
                .map { status, words in (statuses.filter { $0 == status }.count, words) }
                .filter { $0.0 > 0 }
                .map { "\($0.0) \($0.1)" }
            return parts.isEmpty ? "no agents" : parts.joined(separator: ", ")
        case .connecting: return "connecting"
        case .failed: return "can't connect"
        case .revoked: return "needs pairing again"
        }
    }
}

struct ColumnPress: ButtonStyle {
    /// Off while a scrub runs, so the column it started on doesn't stay pressed under it.
    var pressable = true

    func makeBody(configuration: Configuration) -> some View {
        let pressed = pressable && configuration.isPressed
        return configuration.label
            .scaleEffect(pressed ? 0.94 : 1, anchor: .bottom)
            .opacity(pressed ? 0.75 : 1)
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
                .foregroundStyle(.secondary)
                .frame(width: 42, height: 42)
                .background(.fill.tertiary, in: .rect(cornerRadius: 13, style: .continuous))
            VStack(alignment: .leading, spacing: 2) {
                Text("Nothing needs you")
                    .font(.headline)
                Text(working > 0 ? "Condr brings an agent here the moment it waits for you." : "Every agent is at rest.")
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
