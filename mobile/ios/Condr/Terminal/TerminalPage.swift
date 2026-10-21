import CondrKit
import SwiftUI

/// A Pane's terminal, where the phone is one more window (ADR 0041): it reports focus and
/// sizes the Pane to its own grid, draws what the Server sends, and answers through the
/// key row and the field.
struct TerminalPage: View {
    /// The space either side of the grid.
    static let gutter: CGFloat = 10

    struct Grid: Hashable {
        let rows: Int
        let columns: Int
    }

    let ref: PaneRef
    @Environment(Session.self) private var session
    @Environment(Preferences.self) private var preferences
    @Environment(Alerts.self) private var alerts
    @Environment(Router.self) private var router
    @State private var grid: Grid?
    @State private var text = ""
    @State private var control = false
    @State private var baseSize: Double?
    @State private var keyTick = 0
    @FocusState private var focused: Bool

    var body: some View {
        if let pane = session.pane(ref), let device = session.device(ref.address),
            let workspace = session.workspace(of: ref)
        {
            TerminalHost(
                screen: session.screen, fontSize: preferences.fontSize,
                grid: { rows, columns in grid = Grid(rows: rows, columns: columns) },
                session: session,
                open: { UIApplication.shared.open($0) },
                idleTap: { focused = false }
            )
            .padding(.horizontal, Self.gutter)
            .background(Color(hex: Palette.background))
            .overlay {
                if session.screen == nil {
                    ProgressView().tint(.white)
                }
            }
            .overlay(alignment: .bottomTrailing) { newest }
            .overlay { veil(device) }
            .safeAreaBar(edge: .bottom) { inputBar(pane) }
            .simultaneousGesture(pinch)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { toolbar(pane, workspace: workspace, device: device) }
            .toolbarVisibility(.hidden, for: .tabBar)
            .environment(\.colorScheme, .dark)
            .sensoryFeedback(.impact(weight: .light, intensity: 0.6), trigger: keyTick)
            // The grid settles after the keyboard, a rotation or a pinch; one Resize then,
            // not one per frame of the animation (ADR 0041).
            .task(id: grid) {
                guard let grid else { return }
                try? await Task.sleep(for: .milliseconds(250))
                guard !Task.isCancelled else { return }
                session.show(
                    ref, rows: UInt16(clamping: grid.rows), columns: UInt16(clamping: grid.columns))
            }
            .onAppear {
                session.show(ref, rows: 0, columns: 0)
                if UserDefaults.standard.bool(forKey: "focus") { focused = true }
            }
            .onDisappear { session.hide(ref) }
        } else {
            // Forgotten while this page was up: the stack is about to drop it. A Pane that
            // closed, or that a notification named after it closed, leaves once its Device
            // is reached and has no such Pane (ADR 0047).
            Color(hex: Palette.background).ignoresSafeArea()
                .onChange(of: session.device(ref.address)?.connected, initial: true) {
                    _, connected in
                    if connected == true { router.close(ref) }
                }
        }
    }

    // MARK: Chrome

    @ToolbarContentBuilder
    private func toolbar(_ pane: PaneView, workspace: WorkspaceView, device: DeviceView)
        -> some ToolbarContent
    {
        ToolbarItem(placement: .principal) {
            HStack(spacing: 9) {
                AgentMark(kind: pane.agent?.kind)
                    .frame(width: 17, height: 17)
                VStack(alignment: .leading, spacing: 1) {
                    Text(pane.name)
                        .font(.subheadline.weight(.semibold))
                        .lineLimit(1)
                    HStack(spacing: 5) {
                        if let agent = pane.agent {
                            StatusGlyph(status: agent.status, size: 6, ripple: true)
                            if agent.status.needsYou {
                                Text(agent.status.title).fontWeight(.semibold)
                                    .foregroundStyle(agent.status.tint)
                            } else {
                                Text(agent.status.title)
                            }
                            Text("·").foregroundStyle(.tertiary)
                        }
                        Text(place(workspace.name, device.label))
                    }
                    .foregroundStyle(.secondary)
                    .font(.caption2.weight(.medium))
                    .lineLimit(1)
                }
            }
            .accessibilityElement(children: .combine)
        }
        ToolbarItem(placement: .topBarTrailing) {
            Menu {
                Section("Text Size") {
                    ControlGroup {
                        Button("Smaller", systemImage: "textformat.size.smaller") { resize(by: -1) }
                        Button("Larger", systemImage: "textformat.size.larger") { resize(by: 1) }
                    }
                }
            } label: {
                Image(systemName: "ellipsis")
            }
        }
    }

    /// Back to the newest output, once the Pane has been scrolled up (ADR 0036).
    @ViewBuilder
    private var newest: some View {
        if (session.screen?.scrolledLines ?? 0) > 0 {
            Button {
                session.scrollToBottom()
            } label: {
                Image(systemName: "arrow.down")
                    .font(.body.weight(.semibold))
                    .frame(width: 44, height: 44)
            }
            .buttonStyle(.plain)
            .glassEffect(.regular.interactive(), in: .circle)
            .padding(16)
            .transition(.scale.combined(with: .opacity))
            .accessibilityLabel("Jump to the newest output")
        }
    }

    @ViewBuilder
    private func veil(_ device: DeviceView) -> some View {
        if !device.connected {
            ZStack {
                Rectangle().fill(.ultraThinMaterial)
                VStack(spacing: 10) {
                    ProgressView()
                    Text("Reconnecting to \(device.label)…")
                        .font(.subheadline.weight(.medium))
                }
            }
            .transition(.opacity)
        }
    }

    private var pinch: some Gesture {
        MagnifyGesture()
            .onChanged { value in
                let base = baseSize ?? preferences.fontSize
                if baseSize == nil { baseSize = base }
                preferences.fontSize = Self.clampFont(
                    (base * value.magnification * 2).rounded() / 2)
            }
            .onEnded { _ in baseSize = nil }
    }

    private func resize(by step: Double) {
        withAnimation(.smooth) {
            preferences.fontSize = Self.clampFont(preferences.fontSize + step)
        }
    }

    private static func clampFont(_ size: Double) -> Double {
        min(Preferences.largestFont, max(Preferences.smallestFont, size))
    }

    // MARK: Input

    private func placeholder(_ pane: PaneView) -> LocalizedStringResource {
        guard let agent = pane.agent else { return "Command" }
        if agent.status == .blocked { return "Reply, or pick with ↑ ↓ and return" }
        return "Message \(agent.label)"
    }

    private func inputBar(_ pane: PaneView) -> some View {
        VStack(spacing: 8) {
            NoticeToast()
            ErrorToast()
                .padding(.horizontal, -12)
            KeyRow(control: $control, press: press)
            HStack(alignment: .bottom, spacing: 8) {
                // The keyboard moves only when asked, since every appearance resizes the
                // Pane for every viewer (ADR 0041).
                if focused {
                    Button {
                        focused = false
                    } label: {
                        Image(systemName: "keyboard.chevron.compact.down")
                            .font(.body.weight(.semibold))
                            .frame(width: 44, height: 44)
                    }
                    .buttonStyle(.plain)
                    .glassEffect(.regular.interactive(), in: .circle)
                    .transition(.scale.combined(with: .opacity))
                    .accessibilityLabel("Hide Keyboard")
                }
                // A draft of as many lines as the reply needs; Return is a line break, and
                // the terminal's Return is on the key row.
                TextField(placeholder(pane), text: $text, axis: .vertical)
                    .lineLimit(1...5)
                    .font(.body)
                    .textInputAutocapitalization(pane.agent == nil ? .never : .sentences)
                    .autocorrectionDisabled(pane.agent == nil)
                    .focused($focused)
                    .onChange(of: text) { old, new in typedWithControl(old, new) }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
                    .frame(minHeight: 44)
                    .glassEffect(.regular.interactive(), in: .rect(cornerRadius: 22))
                    .accessibilityIdentifier("draft")
                Button(action: submit) {
                    Image(systemName: "arrow.up")
                        .font(.body.weight(.bold))
                        .foregroundStyle(text.isEmpty ? Color.secondary : Color.ink)
                        .frame(width: 44, height: 44)
                }
                .buttonStyle(.plain)
                .glassEffect(
                    text.isEmpty ? .regular.interactive() : .regular.tint(.paper).interactive(),
                    in: .circle
                )
                .disabled(text.isEmpty)
                .accessibilityIdentifier("sendReply")
                .accessibilityLabel("Send")
            }
            .animation(.snappy, value: focused)
        }
        .padding(.horizontal, 12)
        .padding(.top, 6)
        .padding(.bottom, 6)
    }

    private func press(_ key: Key) {
        keyTick += 1
        session.send(key: key, control: control)
        control = false
    }

    /// With Ctrl on, the key just typed goes as Ctrl+key and the draft stays as it was.
    private func typedWithControl(_ old: String, _ new: String) {
        guard control, new.count > old.count else { return }
        let kept = zip(old, new).prefix { $0 == $1 }.count
        if let typed = new.dropFirst(kept).first {
            press(.character(text: String(typed)))
        }
        text = old
    }

    /// Send is the draft as the Server submits an agent prompt: a paste, then Return once
    /// the program has taken it. The keyboard stays where it is (ADR 0041).
    private func submit() {
        guard !text.isEmpty else { return }
        keyTick += 1
        session.submit(text: text)
        text = ""
        control = false
    }
}
