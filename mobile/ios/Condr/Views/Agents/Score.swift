import CondrKit
import SwiftUI

/// The head of the first screen, in place of a title: each computer as a column of the
/// logo's bars, one per Agent, most urgent first, so a colour reads as a run however many
/// there are. A tap on a column opens its computer. Press and slide to scrub the bars: each
/// Agent passed under the finger ticks and names itself, and letting go opens its terminal.
struct Score: View {
    let zoom: Namespace.ID
    /// Whether a scrub is under way, for the page to hold still meanwhile.
    @Binding var scrubbing: Bool
    @Environment(Session.self) private var session
    @Environment(Router.self) private var router
    /// The first screen's scroll space, where the score measures how far it has been lifted.
    nonisolated static let space = "agents"
    /// The score row's own space, which scrubbing reads the finger and the columns in.
    private nonisolated static let row = "score"

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
    private var tall: CGFloat { session.devices.count <= 3 ? 88 : 56 }

    var body: some View {
        ScrollView(.horizontal) {
            HStack(alignment: .bottom, spacing: 24) {
                ForEach(Array(session.devices.enumerated()), id: \.element.id) { index, device in
                    // A tap is the column's button; pressing and sliding scrubs on top of it.
                    Button {
                        if !scrubbed { router.agentsPath.append(.device(device.address)) }
                    } label: {
                        column(
                            device,
                            first: session.devices[..<index].reduce(0) {
                                $0 + session.agents(of: $1).count
                            })
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
            [
                geometry.contentOffset.x > 1,
                geometry.contentOffset.x + geometry.containerSize.width < geometry.contentSize.width
                    - 1,
            ]
        } action: { _, edges in
            withAnimation(.smooth) { self.edges = edges }
        }
        .onScrollGeometryChange(for: CGFloat.self) {
            $0.contentOffset.x
        } action: { _, x in
            scrolled = x
        }
        .onGeometryChange(for: CGFloat.self) {
            $0.frame(in: .named(Score.space)).minY
        } action: { minY in
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
            if !now { Task { @MainActor in scrubbed = false } }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Agents")
        .accessibilityAddTraits(.isHeader)
        .onAppear(perform: stage)
    }

    private func column(_ device: DeviceView, first: Int) -> some View {
        let items = session.agents(of: device).byUrgency()
        let statuses = items.map(\.agent.status)
        let fit = fitting(statuses.count)
        let connected = device.connected
        // Not scrubbing: every bar as usual. Scrubbing: the bar under the finger, none here.
        let highlight: Int??
        if !active {
            highlight = nil
        } else if scrub?.address == device.address {
            highlight = .some(items.firstIndex { $0.ref == scrub })
        } else {
            highlight = .some(nil)
        }
        return VStack(alignment: .leading, spacing: 10) {
            Group {
                if connected, !statuses.isEmpty {
                    Bars(
                        statuses: statuses, tall: tall, widest: fit.width, spacing: fit.spacing,
                        first: first,
                        highlight: highlight, fold: fold)
                } else {
                    Fallen(width: 40)
                }
            }
            .frame(height: tall, alignment: .bottom)
            .onGeometryChange(for: CGRect.self) {
                $0.frame(in: .named(Score.row))
            } action: {
                columns[device.address] = $0
            }
            HStack(spacing: 4) {
                if !connected {
                    Image(
                        systemName: device.status == .revoked ? "key.fill" : "bolt.horizontal.fill"
                    )
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
                if case .second(true, let drag?) = value, pane(at: drag.location) != nil {
                    scrubbed = true
                }
            }
            .onEnded { value in
                if case .second(true, let drag?) = value, let ref = pane(at: drag.location) {
                    router.agentsPath = [.pane(ref)]
                }
            }
    }

    /// The Agent whose bar is nearest the finger, in whichever column it is over; none once
    /// the finger has slid well above or below the bars, which is how a scrub is cancelled.
    private func pane(at point: CGPoint) -> PaneRef? {
        for device in session.devices where device.connected {
            guard let frame = columns[device.address], point.x >= frame.minX - 12,
                point.x <= frame.maxX + 12,
                point.y >= frame.minY - 44, point.y <= frame.maxY + 64
            else { continue }
            let items = session.agents(of: device).byUrgency()
            guard !items.isEmpty else { return nil }
            let fit = fitting(items.count)
            let index = Int((point.x - frame.minX) / (fit.width + fit.spacing))
            return items[min(max(index, 0), items.count - 1)].ref
        }
        return nil
    }

    /// Where the scrubbed bar's centre is on screen.
    private func screenX(of ref: PaneRef) -> CGFloat? {
        guard let device = session.device(ref.address), let frame = columns[ref.address] else {
            return nil
        }
        let items = session.agents(of: device).byUrgency()
        guard let index = items.firstIndex(where: { $0.ref == ref }) else { return nil }
        let fit = fitting(items.count)
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
                if let scrub, let pane = session.pane(scrub), let agent = pane.agent,
                    let device = session.device(scrub.address)
                {
                    AgentMark(kind: agent.kind)
                        .frame(width: 16, height: 16)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(pane.name)
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
        guard parts.count == 2, session.devices.indices.contains(parts[0]) else { return }
        let items = session.agents(of: session.devices[parts[0]]).byUrgency()
        guard items.indices.contains(parts[1]) else { return }
        staged = items[parts[1]].ref
    }

    private func spoken(_ device: DeviceView, _ statuses: [AgentStatus]) -> String {
        switch device.status {
        case .connected:
            let parts = [
                (AgentStatus.blocked, "need you"), (.error, "failed"), (.working, "working"),
                (.done, "done"), (.idle, "idle"),
            ]
            .map { status, words in (statuses.filter { $0 == status }.count, words) }
            .filter { $0.0 > 0 }
            .map { "\($0.0) \($0.1)" }
            return parts.isEmpty ? "no agents" : parts.joined(separator: ", ")
        case .connecting, .suspended: return "connecting"
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
