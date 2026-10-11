import SwiftUI
import UIKit

/// The cell a monospaced font gives at `size`.
struct Cell {
    let size: CGFloat
    let width: CGFloat
    let height: CGFloat
    let lineHeight: CGFloat

    init(size: CGFloat) {
        self.size = size
        let font = UIFont.monospacedSystemFont(ofSize: size, weight: .regular)
        width = ("0" as NSString).size(withAttributes: [.font: font]).width
        lineHeight = font.lineHeight
        height = (font.lineHeight * 1.18).rounded()
    }
}

/// A grid drawn cell by cell: ASCII runs as one text, everything else in its own cell, so
/// box drawing and symbols from fallback fonts keep the columns.
struct TerminalCanvas: View {
    let lines: [Line]
    let cell: Cell
    let columns: Int

    var body: some View {
        Canvas { context, _ in
            for (row, line) in lines.enumerated() {
                let y = CGFloat(row) * cell.height
                var column = 0
                for span in line {
                    let count = span.text.count
                    if let bg = span.style.bg {
                        let width = CGFloat(min(count, max(0, columns - column))) * cell.width
                        context.fill(Path(CGRect(x: CGFloat(column) * cell.width, y: y, width: width, height: cell.height)),
                                     with: .color(Color(hex: bg)))
                    }
                    var run = ""
                    var runStart = column
                    for character in span.text {
                        if character.isASCII {
                            if run.isEmpty { runStart = column }
                            run.append(character)
                        } else {
                            draw(run, at: runStart, y: y, span.style, in: &context)
                            run = ""
                            draw(String(character), at: column, y: y, span.style, in: &context)
                        }
                        column += 1
                    }
                    draw(run, at: runStart, y: y, span.style, in: &context)
                }
            }
        }
        .frame(width: CGFloat(columns) * cell.width, height: CGFloat(lines.count) * cell.height, alignment: .topLeading)
    }

    private func draw(_ text: String, at column: Int, y: CGFloat, _ style: Style, in context: inout GraphicsContext) {
        guard !text.isEmpty, text.contains(where: { $0 != " " }) else { return }
        let font = Font.system(size: cell.size, weight: style.bold ? .semibold : .regular, design: .monospaced)
        var resolved = Text(text).font(font).foregroundStyle(Color(hex: style.fg, opacity: style.dim ? 0.6 : 1))
        if style.italic { resolved = resolved.italic() }
        context.draw(resolved, at: CGPoint(x: CGFloat(column) * cell.width, y: y + (cell.height - cell.lineHeight) / 2),
                     anchor: .topLeading)
    }
}

struct TerminalScreen: View {
    let ref: PaneRef
    @Environment(Store.self) private var store
    @State private var text = ""
    @State private var control = false
    @State private var baseSize: CGFloat?
    @State private var hud: String?
    @State private var hudTask: Task<Void, Never>?
    @State private var atBottom = true
    @State private var position = ScrollPosition(edge: .bottom)
    @State private var selecting = false
    @State private var keyTick = 0
    @FocusState private var focused: Bool

    private let gutter: CGFloat = 10

    var body: some View {
        if let pane = store.pane(ref), let device = store.device(ref.device), let workspace = store.workspace(of: ref) {
            GeometryReader { proxy in
                let cell = Cell(size: store.fontSize)
                let columns = max(20, Int((proxy.size.width - gutter * 2) / cell.width))
                let rows = max(8, Int(proxy.size.height / cell.height))
                screen(pane, cell: cell, columns: columns)
                    .onChange(of: columns, initial: true) { old, new in
                        if old != new || hud == nil { flash("\(new) × \(rows)") }
                    }
            }
            .background(Color(hex: Palette.background))
            .overlay(alignment: .top) { sizeHUD(device.label) }
            .overlay { veil(device) }
            .safeAreaBar(edge: .bottom) { inputBar(pane) }
            .simultaneousGesture(pinch)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { toolbar(pane, workspace: workspace, device: device) }
            .toolbarVisibility(.hidden, for: .tabBar)
            .environment(\.colorScheme, .dark)
            .sheet(isPresented: $selecting) { SelectionSheet(lines: lines(pane, columns: 48, tick: 0)) }
            .sensoryFeedback(.impact(weight: .light, intensity: 0.6), trigger: keyTick)
            .onAppear {
                store.show(ref)
                if UserDefaults.standard.bool(forKey: "focus") { focused = true }
                replay(UserDefaults.standard.string(forKey: "press"))
            }
            .onDisappear { store.hide(ref) }
        }
    }

    private func lines(_ pane: Pane, columns: Int, tick: Int) -> [Line] {
        pane.script.render(pane, columns: columns, tick: tick, now: .now)
    }

    @ViewBuilder
    private func screen(_ pane: Pane, cell: Cell, columns: Int) -> some View {
        let animating = pane.agent?.status == .working
        ScrollView(.vertical) {
            TimelineView(.periodic(from: .now, by: animating ? 0.1 : 3600)) { timeline in
                let tick = Int(timeline.date.timeIntervalSinceReferenceDate * 10)
                TerminalCanvas(lines: lines(pane, columns: columns, tick: tick), cell: cell, columns: columns)
            }
            .padding(.horizontal, gutter)
            .padding(.top, 6)
            .padding(.bottom, 10)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollPosition($position)
        // A grid fills from the top like a terminal, opens at its newest line and follows it.
        .defaultScrollAnchor(.top, for: .alignment)
        .defaultScrollAnchor(.bottom, for: .initialOffset)
        .defaultScrollAnchor(.bottom, for: .sizeChanges)
        .scrollDismissesKeyboard(.interactively)
        .onScrollGeometryChange(for: Bool.self) { geometry in
            // containerSize excludes the insets, and the offset starts at minus the top one.
            geometry.contentOffset.y + geometry.contentInsets.top + geometry.containerSize.height >= geometry.contentSize.height - 24
        } action: { _, bottom in
            withAnimation(.snappy) { atBottom = bottom }
        }
        .overlay(alignment: .bottomTrailing) {
            if !atBottom {
                Button {
                    withAnimation(.smooth) { position.scrollTo(edge: .bottom) }
                } label: {
                    Image(systemName: "arrow.down")
                        .font(.body.weight(.semibold))
                        .frame(width: 42, height: 42)
                }
                .buttonStyle(.plain)
                .glassEffect(.regular.interactive(), in: .circle)
                .padding(.trailing, 14)
                .padding(.bottom, 8)
                .transition(.scale.combined(with: .opacity))
                .accessibilityLabel("Scroll to bottom")
            }
        }
        .accessibilityLabel("Terminal")
        .accessibilityValue(lines(pane, columns: columns, tick: 0).map { $0.map(\.text).joined() }.joined(separator: "\n"))
    }

    // MARK: Chrome

    @ToolbarContentBuilder
    private func toolbar(_ pane: Pane, workspace: Workspace, device: Device) -> some ToolbarContent {
        ToolbarItem(placement: .principal) {
            HStack(spacing: 9) {
                AgentMark(kind: pane.agent?.kind)
                    .frame(width: 17, height: 17)
                VStack(alignment: .leading, spacing: 1) {
                    Text(pane.title)
                        .font(.subheadline.weight(.semibold))
                        .lineLimit(1)
                    HStack(spacing: 5) {
                        if let agent = pane.agent {
                            StatusGlyph(status: agent.status, size: 6)
                            Text(agent.status.title)
                                .foregroundStyle(agent.status == .idle ? .secondary : agent.status.tint)
                            Text("·").foregroundStyle(.tertiary)
                        }
                        Text(place(workspace.name, device.label))
                            .foregroundStyle(.secondary)
                    }
                    .font(.caption2.weight(.medium))
                    .lineLimit(1)
                }
            }
            .accessibilityElement(children: .combine)
        }
        ToolbarItem(placement: .topBarTrailing) {
            Menu {
                Button("Select Text", systemImage: "selection.pin.in.out") { selecting = true }
                Section("Text Size") {
                    ControlGroup {
                        Button("Smaller", systemImage: "textformat.size.smaller") { resize(by: -1) }
                        Button("Larger", systemImage: "textformat.size.larger") { resize(by: 1) }
                    }
                }
                Section {
                    Label("\(device.label) fits its terminals to this iPhone while you're here", systemImage: "arrow.left.and.right")
                }
            } label: {
                Image(systemName: "ellipsis")
            }
        }
    }

    @ViewBuilder
    private func sizeHUD(_ device: String) -> some View {
        if let hud {
            HStack(spacing: 6) {
                Image(systemName: "arrow.left.and.right")
                    .font(.caption2.weight(.bold))
                Text("Fitted to this iPhone")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
                Text(hud)
                    .font(.caption.monospacedDigit().weight(.semibold))
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 7)
            .glassEffect(.regular.tint(.black.opacity(0.45)), in: .capsule)
            .padding(.top, 8)
            .transition(.move(edge: .top).combined(with: .opacity))
            .accessibilityLabel("\(device)'s terminal fitted to this iPhone, \(hud)")
        }
    }

    @ViewBuilder
    private func veil(_ device: Device) -> some View {
        if device.status != .connected {
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

    private func flash(_ size: String) {
        withAnimation(.smooth) { hud = size }
        hudTask?.cancel()
        hudTask = Task {
            try? await Task.sleep(for: .seconds(1.8))
            guard !Task.isCancelled else { return }
            withAnimation(.smooth) { hud = nil }
        }
    }

    private var pinch: some Gesture {
        MagnifyGesture()
            .onChanged { value in
                let base = baseSize ?? store.fontSize
                if baseSize == nil { baseSize = base }
                store.fontSize = min(20, max(8, (base * value.magnification * 2).rounded() / 2))
            }
            .onEnded { _ in baseSize = nil }
    }

    private func resize(by step: CGFloat) {
        withAnimation(.smooth) { store.fontSize = min(20, max(8, store.fontSize + step)) }
    }

    // MARK: Input

    private func placeholder(_ pane: Pane) -> String {
        guard let agent = pane.agent else { return "Command" }
        if agent.status == .blocked, pane.script.menu != nil { return "Choose 1–\(pane.script.menu!), or ↑ ↓ and return" }
        return "Message \(agent.kind.label)"
    }

    private func inputBar(_ pane: Pane) -> some View {
        VStack(spacing: 8) {
            keyRow
            HStack(spacing: 8) {
                TextField(placeholder(pane), text: $text)
                    .font(.body)
                    .textInputAutocapitalization(pane.agent == nil ? .never : .sentences)
                    .autocorrectionDisabled(pane.agent == nil)
                    .submitLabel(.send)
                    .focused($focused)
                    .onSubmit(submit)
                    .onChange(of: text) { _, new in
                        guard control, let last = new.last else { return }
                        press(.character(String(last)))
                        text = ""
                    }
                    .padding(.horizontal, 16)
                    .frame(minHeight: 44)
                    .glassEffect(.regular.interactive(), in: .capsule)
                if !text.isEmpty {
                    Button(action: submit) {
                        Image(systemName: "arrow.up")
                            .font(.body.weight(.bold))
                            .foregroundStyle(pane.agent?.kind.brand == nil ? Color.ink : .white)
                            .frame(width: 44, height: 44)
                    }
                    .buttonStyle(.glassProminent)
                    .buttonBorderShape(.circle)
                    .tint(pane.agent?.kind.brand.map { Color(hex: $0) } ?? .accentColor)
                    .transition(.scale.combined(with: .opacity))
                    .accessibilityLabel("Send")
                }
            }
            .animation(.snappy, value: text.isEmpty)
        }
        .padding(.horizontal, 12)
        .padding(.top, 6)
        .padding(.bottom, 6)
    }

    private var keyRow: some View {
        GlassEffectContainer(spacing: 6) {
            HStack(spacing: 6) {
                key("esc") { press(.escape) }
                key("tab") { press(.tab) }
                Button {
                    control.toggle()
                    keyTick += 1
                } label: {
                    Text("ctrl")
                        .font(.system(size: 14, weight: .semibold, design: .monospaced))
                        .frame(minWidth: 46, minHeight: 36)
                        .foregroundStyle(control ? Color.black : Color.primary)
                }
                .buttonStyle(.plain)
                .glassEffect(control ? .regular.tint(.orange).interactive() : .regular.interactive(), in: .capsule)
                .accessibilityLabel("Control")
                .accessibilityAddTraits(control ? .isSelected : [])
                Spacer(minLength: 0)
                HStack(spacing: 0) {
                    arrow("arrow.left", .left, "Left")
                    arrow("arrow.up", .up, "Up")
                    arrow("arrow.down", .down, "Down")
                    arrow("arrow.right", .right, "Right")
                }
                .glassEffect(.regular.interactive(), in: .capsule)
                key(symbol: "return", label: "Return") { press(.enter) }
            }
        }
    }

    private func key(_ title: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .font(.system(size: 14, weight: .semibold, design: .monospaced))
                .frame(minWidth: 46, minHeight: 36)
        }
        .buttonStyle(.plain)
        .glassEffect(.regular.interactive(), in: .capsule)
    }

    private func key(symbol: String, label: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 15, weight: .semibold))
                .frame(minWidth: 46, minHeight: 36)
        }
        .buttonStyle(.plain)
        .glassEffect(.regular.interactive(), in: .capsule)
        .accessibilityLabel(label)
    }

    private func arrow(_ symbol: String, _ key: Key, _ label: String) -> some View {
        Button { press(key) } label: {
            Image(systemName: symbol)
                .font(.system(size: 14, weight: .semibold))
                .frame(width: 34, height: 36)
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }

    private func press(_ key: Key) {
        keyTick += 1
        store.send(ref, key: key, control: control)
        control = false
    }

    /// The prototype's stand-in for taps: `-press "down enter t:Ship it"` after opening.
    private func replay(_ script: String?) {
        guard let script else { return }
        let keys: [String: Key] = ["enter": .enter, "esc": .escape, "tab": .tab, "up": .up, "down": .down, "left": .left, "right": .right]
        Task {
            for step in script.split(separator: " ").map(String.init) {
                try? await Task.sleep(for: .seconds(0.8))
                if step.hasPrefix("t:") {
                    store.send(ref, text: String(step.dropFirst(2)).replacingOccurrences(of: "_", with: " "))
                } else if let key = keys[step] {
                    store.send(ref, key: key)
                }
            }
        }
    }

    private func submit() {
        if text.isEmpty {
            press(.enter)
        } else {
            keyTick += 1
            store.send(ref, text: text)
            text = ""
        }
        focused = true
    }
}

/// The screen as text the person can select a range of and copy, the phone's stand-in for
/// a drag selection (ADR 0041).
struct SelectionSheet: View {
    let lines: [Line]
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            SelectableText(text: lines.map { $0.map(\.text).joined() }.joined(separator: "\n"))
                .background(Color(hex: Palette.background))
                .navigationTitle("Select Text")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Done", systemImage: "checkmark") { dismiss() }
                    }
                }
        }
        .environment(\.colorScheme, .dark)
        .presentationDetents([.large])
    }
}

private struct SelectableText: UIViewRepresentable {
    let text: String

    func makeUIView(context: Context) -> UITextView {
        let view = UITextView()
        view.isEditable = false
        view.isSelectable = true
        view.backgroundColor = .clear
        view.textColor = UIColor(Color(hex: Palette.foreground))
        view.font = .monospacedSystemFont(ofSize: 13, weight: .regular)
        view.textContainerInset = UIEdgeInsets(top: 16, left: 12, bottom: 16, right: 12)
        return view
    }

    func updateUIView(_ view: UITextView, context: Context) {
        view.text = text
    }
}
