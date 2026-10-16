import CondrKit
import CoreImage.CIFilterBuiltins
import SwiftUI

/// Pairing with a computer (ADR 0040): the computer shows an Invite as a code, the phone
/// scans it or takes it pasted, names itself the first time, and connects. On first launch
/// a welcome comes first.
struct PairingFlow: View {
    enum Step: String, Hashable { case code, scan, name, connecting }

    var firstLaunch: Bool
    /// The flow ended: paired, or cancelled when it was a sheet.
    let finished: () -> Void
    @Environment(Session.self) private var session
    @Environment(Preferences.self) private var preferences
    @State private var path: [Step] = []
    @State private var link = ""
    @State private var name = ""
    /// The handshake under way, or done: one per visit to the last step.
    @State private var pairing: Pairing?

    init(firstLaunch: Bool, finished: @escaping () -> Void) {
        self.firstLaunch = firstLaunch
        self.finished = finished
        // A demo scene opens the flow on one of its steps.
        let order: [Step] = [.code, .scan, .name, .connecting]
        let start: Int? =
            switch DemoScene.launched {
            case .pair: 0
            case .scan: 1
            case .name: 2
            case .connecting, .paired: 3
            default: nil
            }
        _path = State(initialValue: start.map { Array(order[(firstLaunch ? 0 : 1)...$0]) } ?? [])
        _link = State(initialValue: start == nil ? "" : DemoWorld.jpdev + ".f3a91c0d")
    }

    var body: some View {
        NavigationStack(path: $path) {
            Group {
                if firstLaunch {
                    Welcome { path.append(.code) }
                } else {
                    inviteStep
                        .toolbar {
                            ToolbarItem(placement: .cancellationAction) {
                                Button("Cancel", systemImage: "xmark") { finished() }
                            }
                        }
                }
            }
            .navigationDestination(for: Step.self) { step in
                switch step {
                case .code: inviteStep
                case .scan: ScanStep(found: take)
                case .name: NameStep(name: $name, next: connect)
                case .connecting:
                    // Through the binding: this closure can run with the flow's state from
                    // before `connect()`, and `pairing` read here would still be nil.
                    ConnectingDestination(pairing: $pairing, firstLaunch: firstLaunch) {
                        finished()
                    } back: {
                        pairing = nil
                        path.removeAll { $0 == .connecting }
                    }
                }
            }
        }
        .onAppear {
            if name.isEmpty { name = preferences.phoneName }
            // A demo scene opens on the last step; it pairs as the person would have.
            if path.last == .connecting, pairing == nil { connect() }
        }
    }

    /// The last step: the handshake starts once, as the person moves on to it.
    private func connect() {
        if pairing == nil {
            pairing = Pairing(session: session, preferences: preferences, link: link, name: name)
        }
        if path.last != .connecting { path.append(.connecting) }
    }

    private var inviteStep: some View {
        InviteStep(offersDemo: firstLaunch, scan: { path.append(.scan) }, pasted: take)
    }

    /// An Invite link, scanned or pasted, moves the flow on to naming the phone, or
    /// straight to pairing once it has a name.
    private func take(_ link: String) {
        self.link = link
        if preferences.named {
            connect()
        } else if path.last != .name {
            path.append(.name)
        }
    }
}

// MARK: - Welcome

/// The brand's one loud moment: ink, the mark building itself, and the score playing out
/// what the app is for before a word of it is read.
struct Welcome: View {
    let next: () -> Void
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @ScaledMetric(relativeTo: .largeTitle) private var headlineSize: CGFloat = 46
    @State private var stage = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 9) {
                Logo(assembled: stage >= 1, color: .paper)
                    .frame(width: 30, height: 30)
                Text("Condr")
                    .font(.title3.weight(.semibold))
                    .opacity(stage >= 1 ? 1 : 0)
            }
            .padding(.top, 8)
            Spacer(minLength: 8)
            WelcomeStage(playing: stage >= 2)
                .frame(height: 250)
                .padding(.horizontal, -28)
                .opacity(stage >= 2 ? 1 : 0)
            Spacer(minLength: 8)
            VStack(alignment: .leading, spacing: 14) {
                Text("Every agent,\nwithin reach.")
                    .font(.system(size: headlineSize, weight: .bold))
                    .minimumScaleFactor(0.7)
                    .tracking(-1.4)
                    .lineSpacing(-2)
                Text(
                    "See which agent waits for you, read what it did, and answer it. From anywhere."
                )
                .font(.title3)
                .foregroundStyle(Color.paper.opacity(0.6))
                .fixedSize(horizontal: false, vertical: true)
            }
            .opacity(stage >= 3 ? 1 : 0)
            .offset(y: stage >= 3 ? 0 : 14)
            Spacer(minLength: 0)
        }
        .foregroundStyle(Color.paper)
        .padding(.horizontal, 28)
        .safeAreaBar(edge: .bottom) {
            // The steps' footer, so the button sits where theirs do.
            StepFooter {
                Button(action: next) {
                    Text("Pair with Your Computer")
                        .font(.headline)
                        .foregroundStyle(Color.ink)
                        .frame(maxWidth: .infinity, minHeight: 34)
                }
                .buttonStyle(.glassProminent)
                .tint(.paper)
                .controlSize(.large)
            } secondary: {
            }
            .opacity(stage >= 3 ? 1 : 0)
        }
        // Black like the steps after it in dark mode, so the cover hands over without a seam.
        .background(Color.black.ignoresSafeArea())
        .environment(\.colorScheme, .dark)
        .toolbarVisibility(.hidden, for: .navigationBar)
        .task {
            if reduceMotion {
                stage = 3
                return
            }
            try? await Task.sleep(for: .milliseconds(300))
            withAnimation(.spring(duration: 0.9, bounce: 0.35)) { stage = 1 }
            try? await Task.sleep(for: .milliseconds(500))
            withAnimation(.smooth(duration: 0.9)) { stage = 2 }
            try? await Task.sleep(for: .milliseconds(400))
            withAnimation(.smooth(duration: 0.8)) { stage = 3 }
        }
    }
}

/// Three computers' Agents as the first screen's score draws them, a column each, idling
/// and working, while one after another rises to need you or settles as done, its mark
/// and words floating over it: the home screen's language, taught before it is seen.
struct WelcomeStage: View {
    var playing: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var cue: Int?

    private struct Story {
        let column: Int
        let bar: Int
        let status: AgentStatus
        let kind: String
        let words: LocalizedStringResource
    }
    private let stories = [
        Story(column: 0, bar: 2, status: .blocked, kind: "claude", words: "Claude needs you"),
        Story(column: 1, bar: 4, status: .done, kind: "codex", words: "Codex is done"),
        Story(column: 2, bar: 1, status: .blocked, kind: "claude", words: "Claude asks a question"),
        Story(column: 1, bar: 0, status: .done, kind: "opencode", words: "OpenCode is done"),
    ]
    private let computers = ["macmini", "buildbox", "thinkbook"]
    /// Each bar's own state when no story uses it.
    private let base: [[AgentStatus]] = [
        [.working, .idle, .working, .idle, .working, .idle],
        [.working, .idle, .working, .working, .idle, .working, .idle],
        [.idle, .working, .idle, .working, .idle],
    ]

    var body: some View {
        GeometryReader { proxy in
            let margin: CGFloat = 26
            let gap: CGFloat = 30
            let spacing: CGFloat = 7
            let count = base.reduce(0) { $0 + $1.count }
            let width =
                (proxy.size.width - margin * 2 - gap * CGFloat(base.count - 1) - spacing
                    * CGFloat(count - base.count))
                / CGFloat(count)
            let baseline = proxy.size.height - 30
            let tall = baseline - 58
            let story = cue.map { stories[$0] }
            let origin = { (column: Int) -> CGFloat in
                let before = base[..<column].reduce(0) { $0 + $1.count }
                return margin + CGFloat(before) * width + CGFloat(before - column) * spacing
                    + CGFloat(column) * gap
            }
            let status = { (column: Int, bar: Int) -> AgentStatus in
                if let story, story.column == column, story.bar == bar { return story.status }
                return base[column][bar]
            }
            TimelineView(.animation(minimumInterval: 1 / 30, paused: reduceMotion || !playing)) {
                timeline in
                let time = timeline.date.timeIntervalSinceReferenceDate
                ZStack(alignment: .topLeading) {
                    ForEach(base.indices, id: \.self) { column in
                        ForEach(base[column].indices, id: \.self) { bar in
                            let index = base[..<column].reduce(0) { $0 + $1.count } + bar
                            let current = status(column, bar)
                            let h = Bars.height(
                                current, tall: tall, wave: sin(time * 2.4 - Double(index) * 0.32))
                            RoundedRectangle(cornerRadius: min(4, width * 0.28), style: .continuous)
                                .fill(
                                    current == .idle
                                        ? AnyShapeStyle(Color.paper.opacity(0.14))
                                        : AnyShapeStyle(current.tint.gradient)
                                )
                                .frame(width: width, height: h)
                                .position(
                                    x: origin(column) + CGFloat(bar) * (width + spacing) + width
                                        / 2, y: baseline - h / 2)
                        }
                        let columnWidth = CGFloat(base[column].count) * (width + spacing) - spacing
                        Text(computers[column])
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(Color.paper.opacity(0.42))
                            .frame(width: columnWidth, alignment: .leading)
                            .position(x: origin(column) + columnWidth / 2, y: baseline + 17)
                    }
                }
            }
            if let story {
                let h = Bars.height(story.status, tall: tall, wave: 0)
                let x = origin(story.column) + CGFloat(story.bar) * (width + spacing) + width / 2
                HStack(spacing: 8) {
                    AgentMark(kind: story.kind)
                        .frame(width: 15, height: 15)
                    Text(story.words)
                        .font(.footnote.weight(.semibold))
                }
                .padding(.horizontal, 13)
                .padding(.vertical, 9)
                .glassEffect(.regular, in: .capsule)
                .fixedSize()
                .position(x: min(max(x, 104), proxy.size.width - 104), y: baseline - h - 28)
                .transition(.scale(scale: 0.6, anchor: .bottom).combined(with: .opacity))
                .id(cue)
            }
        }
        .accessibilityHidden(true)
        .task(id: playing) {
            guard playing else { return }
            var next = 0
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(0.7))
                withAnimation(.spring(duration: 0.7, bounce: 0.3)) { cue = next }
                try? await Task.sleep(for: .seconds(2.8))
                withAnimation(.smooth(duration: 0.6)) { cue = nil }
                next = (next + 1) % stories.count
            }
        }
    }
}

// MARK: - Showing the code

struct InviteStep: View {
    let offersDemo: Bool
    let scan: () -> Void
    let pasted: (String) -> Void
    @State private var pasting = false
    @State private var showingDemo = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 28) {
                StepTitle("Show the pairing code on your computer")
                InviteIllustration()
                    .padding(.horizontal, 4)
                VStack(alignment: .leading, spacing: 16) {
                    numbered(1, Text("Open **Condr** on your computer."))
                    numbered(2, Text("Go to **Settings › Paired devices**."))
                    numbered(3, Text("Choose **Show pairing code**."))
                }
            }
            .padding(.horizontal, 24)
            .padding(.top, 8)
        }
        .scrollBounceBehavior(.basedOnSize)
        .safeAreaBar(edge: .bottom) {
            VStack(spacing: 0) {
                StepFooter {
                    PrimaryButton("Scan Code", systemImage: "qrcode.viewfinder", action: scan)
                } secondary: {
                    Button("Paste a Link Instead") { pasting = true }
                        .font(.subheadline.weight(.medium))
                }
                if offersDemo {
                    Button {
                        showingDemo = true
                    } label: {
                        Text(
                            "No computer to pair? \(Text("Try Demo").foregroundStyle(Color.accentColor))"
                        )
                        .foregroundStyle(.secondary)
                        .font(.footnote)
                        .frame(maxWidth: .infinity, minHeight: 44)
                        .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                    .padding(.horizontal, 24)
                    .accessibilityIdentifier("tryDemo")
                }
            }
        }
        .sheet(isPresented: $pasting) {
            PasteLink { link in
                pasting = false
                pasted(link)
            }
        }
        .fullScreenCover(isPresented: $showingDemo) { DemoExperience() }
        // Pairing starts here: iOS's Local Network prompt now, not on the pairing dial.
        .onAppear {
            if DemoScene.launched == nil { LocalNetwork.ask() }
        }
    }

    private func numbered(_ number: Int, _ text: Text) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text("\(number)")
                .font(.footnote.weight(.bold).monospacedDigit())
                .frame(width: 24, height: 24)
                .background(.fill.secondary, in: .circle)
            text.font(.body)
        }
    }
}

struct StepTitle: View {
    let title: LocalizedStringResource
    let detail: LocalizedStringResource?

    init(_ title: LocalizedStringResource, _ detail: LocalizedStringResource? = nil) {
        self.title = title
        self.detail = detail
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title)
                .font(.largeTitle.bold())
                .tracking(-0.4)
                .fixedSize(horizontal: false, vertical: true)
            if let detail {
                Text(detail)
                    .font(.body)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// The desktop's Settings window as the person will find it, invite code showing.
struct InviteIllustration: View {
    private static let window = Color(hex: 0x16181c)

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                ForEach([0xff5f57, 0xfebc2e, 0x28c840] as [UInt32], id: \.self) { color in
                    Circle().fill(Color(hex: color)).frame(width: 9, height: 9)
                }
                Spacer()
            }
            .overlay {
                Text("Settings").font(.system(size: 11, weight: .semibold)).foregroundStyle(
                    .white.opacity(0.55))
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            HStack(spacing: 0) {
                VStack(alignment: .leading, spacing: 3) {
                    ForEach(
                        ["General", "Appearance", "Terminal", "Paired devices", "About"], id: \.self
                    ) { item in
                        Text(item)
                            .font(
                                .system(
                                    size: 10.5,
                                    weight: item == "Paired devices" ? .semibold : .regular)
                            )
                            .foregroundStyle(.white.opacity(item == "Paired devices" ? 1 : 0.55))
                            .padding(.horizontal, 8)
                            .padding(.vertical, 5)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(
                                item == "Paired devices" ? Color.white.opacity(0.1) : .clear,
                                in: .rect(cornerRadius: 6, style: .continuous))
                    }
                    Spacer()
                }
                .frame(width: 104)
                .padding(.horizontal, 8)
                Rectangle().fill(.white.opacity(0.08)).frame(width: 1)
                VStack(spacing: 10) {
                    QRCode(text: DemoWorld.jpdev + ".f3a91c0d")
                        .frame(width: 92, height: 92)
                        .padding(7)
                        .background(.white, in: .rect(cornerRadius: 8, style: .continuous))
                    VStack(spacing: 3) {
                        Text("Scan with Condr on your phone")
                            .font(.system(size: 10, weight: .medium))
                        Text("Expires in 9:12")
                            .font(.system(size: 9).monospacedDigit())
                            .foregroundStyle(.white.opacity(0.5))
                    }
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 14)
            }
            .frame(height: 170)
        }
        .foregroundStyle(.white)
        .background(Self.window, in: .rect(cornerRadius: 16, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(
                .white.opacity(0.1), lineWidth: 1)
        }
        .shadow(color: .black.opacity(0.22), radius: 28, y: 14)
        .environment(\.colorScheme, .dark)
        .accessibilityElement()
        .accessibilityLabel(
            "Condr's Settings on your computer, with Paired devices open and a pairing code showing"
        )
    }
}

struct QRCode: View {
    let text: String
    private static let context = CIContext()

    var body: some View {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        let image = filter.outputImage.flatMap { Self.context.createCGImage($0, from: $0.extent) }
        return Group {
            if let image {
                Image(decorative: image, scale: 1)
                    .interpolation(.none)
                    .resizable()
            }
        }
        .aspectRatio(1, contentMode: .fit)
    }
}

struct PasteLink: View {
    let done: (String) -> Void
    @State private var link = ""
    @State private var problem: LocalizedStringResource?
    @FocusState private var focused: Bool

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 10) {
                Text("Copy the link under the code on your computer and paste it here.")
                    .foregroundStyle(.secondary)
                TextField("p2p://… or tcp://…", text: $link, axis: .vertical)
                    .font(.body.monospaced())
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .focused($focused)
                    .padding(14)
                    .background(
                        Color(.secondarySystemBackground),
                        in: .rect(cornerRadius: 14, style: .continuous))
                if let problem {
                    Label {
                        Text(problem)
                    } icon: {
                        Image(systemName: "exclamationmark.circle.fill")
                    }
                    .font(.footnote)
                    .foregroundStyle(.red)
                }
                Spacer()
            }
            .padding(20)
            .navigationTitle("Paste a Link")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Continue", systemImage: "checkmark") {
                        // A paste may bring whitespace or invisible marks along; a link is
                        // letters, digits and a few signs.
                        let trimmed = link.filter {
                            $0.isLetter || $0.isNumber || "-_.:@/[]".contains($0)
                        }
                        if CodeScanner.isInvite(trimmed) {
                            done(trimmed)
                        } else {
                            withAnimation {
                                problem =
                                    trimmed.isEmpty
                                    ? "Paste the link first."
                                    : "That isn't a pairing link. It starts with p2p:// or tcp://."
                            }
                        }
                    }
                }
            }
            .onAppear { focused = true }
        }
        .presentationDetents([.medium])
    }
}

// MARK: - Scanning

/// The camera on the code the computer shows; where there is no camera, the link pasted.
struct ScanStep: View {
    let found: (String) -> Void
    @State private var pasting = false

    var body: some View {
        Group {
            if CodeScanner.available {
                CodeScanner(found: found)
                    .ignoresSafeArea()
                    .overlay {
                        Reticle()
                            .stroke(
                                Color.white.opacity(0.85),
                                style: StrokeStyle(lineWidth: 4, lineCap: .round, lineJoin: .round)
                            )
                            .frame(width: 250, height: 250)
                            .shadow(color: .black.opacity(0.3), radius: 8)
                            .allowsHitTesting(false)
                    }
                    .overlay(alignment: .bottom) {
                        Text("Point at the code on your computer")
                            .font(.subheadline.weight(.semibold))
                            .padding(.horizontal, 18)
                            .padding(.vertical, 12)
                            .glassEffect(.regular, in: .capsule)
                            .padding(.bottom, 24)
                    }
                    .environment(\.colorScheme, .dark)
            } else {
                ContentUnavailableView {
                    Label("No camera to scan with", systemImage: "camera")
                } description: {
                    Text("Paste the link under the code on your computer instead.")
                } actions: {
                    PrimaryButton("Paste a Link") { pasting = true }
                }
            }
        }
        .navigationBarTitleDisplayMode(.inline)
        .sheet(isPresented: $pasting) {
            PasteLink { link in
                pasting = false
                found(link)
            }
        }
    }
}

nonisolated struct Reticle: Shape {
    func path(in rect: CGRect) -> Path {
        let arm = rect.width * 0.16
        let radius = rect.width * 0.08
        var path = Path()
        let corners: [(CGPoint, CGFloat, CGFloat)] = [
            (CGPoint(x: rect.minX, y: rect.minY), 1, 1),
            (CGPoint(x: rect.maxX, y: rect.minY), -1, 1),
            (CGPoint(x: rect.minX, y: rect.maxY), 1, -1),
            (CGPoint(x: rect.maxX, y: rect.maxY), -1, -1),
        ]
        for (corner, dx, dy) in corners {
            path.move(to: CGPoint(x: corner.x, y: corner.y + dy * arm))
            path.addLine(to: CGPoint(x: corner.x, y: corner.y + dy * radius))
            path.addQuadCurve(to: CGPoint(x: corner.x + dx * radius, y: corner.y), control: corner)
            path.addLine(to: CGPoint(x: corner.x + dx * arm, y: corner.y))
        }
        return path
    }
}

// MARK: - Naming

struct NameStep: View {
    @Binding var name: String
    let next: () -> Void
    @Environment(Session.self) private var session
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 28) {
            StepTitle(
                "Name this iPhone", "Your computer lists it under Paired devices by this name.")
            HStack {
                Image(systemName: "iphone")
                    .font(.title2)
                    .foregroundStyle(.secondary)
                TextField("Name", text: $name)
                    .font(.title3.weight(.medium))
                    .focused($focused)
                    .submitLabel(.continue)
                    .onSubmit { if !trimmed.isEmpty { next() } }
                if !name.isEmpty {
                    Button("Clear", systemImage: "xmark.circle.fill") { name = "" }
                        .labelStyle(.iconOnly)
                        .foregroundStyle(.tertiary)
                }
            }
            .padding(.horizontal, 18)
            .frame(minHeight: 60)
            .background(
                Color(.secondarySystemBackground), in: .rect(cornerRadius: 18, style: .continuous))
            PairedPreview(name: trimmed.isEmpty ? "iPhone" : trimmed, key: session.fingerprint)
            Spacer()
        }
        .padding(.horizontal, 24)
        .padding(.top, 8)
        .safeAreaBar(edge: .bottom) {
            StepFooter {
                PrimaryButton("Continue", action: next)
                    .disabled(trimmed.isEmpty)
            } secondary: {
                EmptyView()
            }
        }
        .onAppear { focused = true }
    }

    private var trimmed: String { name.trimmingCharacters(in: .whitespaces) }
}

/// The row the computer's Settings › Paired devices will show, drawn as the desktop draws
/// it and following the name as it is typed.
struct PairedPreview: View {
    let name: String
    let key: String

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Paired devices on your computer")
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(.white.opacity(0.5))
            HStack(spacing: 12) {
                Image(systemName: "iphone")
                    .font(.system(size: 18, weight: .light))
                    .frame(width: 30, height: 30)
                    .background(
                        .white.opacity(0.08), in: .rect(cornerRadius: 7, style: .continuous))
                VStack(alignment: .leading, spacing: 2) {
                    Text(name)
                        .font(.system(size: 13, weight: .semibold))
                        .contentTransition(.interpolate)
                        .animation(.snappy, value: name)
                    Text(key)
                        .font(.system(size: 10.5).monospaced())
                        .foregroundStyle(.white.opacity(0.5))
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Spacer(minLength: 8)
                Text("Just now")
                    .font(.system(size: 11))
                    .foregroundStyle(.white.opacity(0.5))
            }
        }
        .foregroundStyle(.white)
        .padding(14)
        .background(Color(hex: 0x16181c), in: .rect(cornerRadius: 14, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(
                .white.opacity(0.1), lineWidth: 1)
        }
        .environment(\.colorScheme, .dark)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Your computer will list this iPhone as \(name)")
    }
}

/// A step's footer: its one action, then a slot of fixed height for a second action or a
/// note, so the button sits at the same place on every step.
struct StepFooter<Primary: View, Secondary: View>: View {
    @ViewBuilder let primary: Primary
    @ViewBuilder let secondary: Secondary

    var body: some View {
        VStack(spacing: 6) {
            primary
            secondary
                .frame(maxWidth: .infinity, minHeight: 44, maxHeight: 44)
        }
        .padding(.horizontal, 24)
    }
}

// MARK: - Nearby

/// The phone and the computer on one network: Wi-Fi rings spreading between them.
struct Nearby: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        TimelineView(.animation(paused: reduceMotion)) { timeline in
            let time = timeline.date.timeIntervalSinceReferenceDate
            ZStack {
                ForEach(0..<4) { ring in
                    let progress = (time / 3.2 + Double(ring) / 4).truncatingRemainder(
                        dividingBy: 1)
                    Circle()
                        .stroke(Color.primary.opacity((1 - progress) * 0.28), lineWidth: 1.2)
                        .frame(width: 70 + 220 * progress, height: 70 + 220 * progress)
                }
                Image(systemName: "wifi")
                    .font(.system(size: 26, weight: .semibold))
                    .frame(width: 64, height: 64)
                    .background(Color(.systemBackground), in: .circle)
                    .overlay { Circle().strokeBorder(Color.primary.opacity(0.12)) }
                HStack {
                    device("iphone")
                    Spacer()
                    device("desktopcomputer")
                }
            }
        }
        .frame(height: 230)
        .clipped()
        .accessibilityHidden(true)
    }

    private func device(_ symbol: String) -> some View {
        Image(systemName: symbol)
            .font(.system(size: 42, weight: .light))
            .symbolRenderingMode(.hierarchical)
            .frame(width: 92, height: 92)
            .background(Color(.systemBackground), in: .circle)
    }
}

/// A computer's score arriving: its bars rise one by one over its name.
struct Rising: View {
    let statuses: [AgentStatus]
    let shown: Int
    let label: String

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .bottom, spacing: 9) {
                ForEach(Array(statuses.enumerated()), id: \.offset) { index, status in
                    RoundedRectangle(cornerRadius: 5, style: .continuous)
                        .fill(
                            status == .idle || status == .unknown
                                ? AnyShapeStyle(.tertiary) : AnyShapeStyle(status.tint.gradient)
                        )
                        .frame(
                            width: 24,
                            height: index < shown ? Bars.height(status, tall: 120, wave: 0.5) : 0)
                }
            }
            .frame(height: 120, alignment: .bottom)
            Text(label)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.secondary)
        }
        .accessibilityHidden(true)
    }
}

// MARK: - Connecting

/// One handshake with a computer (ADR 0040): started by the flow, watched by the last step,
/// tried again on request. The phone's name goes to the library first, so the Server records
/// it; a success keeps the name as the phone's.
@Observable
@MainActor
final class Pairing {
    enum Phase: Equatable {
        case pairing
        case paired(SavedDevice)
        case failed(String)
    }

    private(set) var phase = Phase.pairing
    private let session: Session
    private let preferences: Preferences
    private let link: String
    private let name: String
    private var task: Task<Void, Never>?

    init(session: Session, preferences: Preferences, link: String, name: String) {
        self.session = session
        self.preferences = preferences
        self.link = link
        self.name = name
        start()
    }

    var paired: SavedDevice? {
        if case .paired(let device) = phase { return device }
        return nil
    }

    func retry() {
        guard case .failed = phase else { return }
        start()
    }

    private func start() {
        phase = .pairing
        task?.cancel()
        task = Task {
            do {
                let device = try await session.pair(link: link, phoneName: name)
                preferences.phoneName = name
                withAnimation(.spring(duration: 0.6, bounce: 0.25)) { phase = .paired(device) }
            } catch {
                let message = (error as? MobileError)?.message ?? error.localizedDescription
                withAnimation { phase = .failed(message) }
            }
        }
    }
}

/// The last step once its handshake exists.
struct ConnectingDestination: View {
    @Binding var pairing: Pairing?
    let firstLaunch: Bool
    let finish: () -> Void
    let back: () -> Void

    var body: some View {
        if let pairing {
            ConnectingStep(pairing: pairing, firstLaunch: firstLaunch, finish: finish, back: back)
        }
    }
}

/// The handshake with the computer, then its score and Agents arriving.
struct ConnectingStep: View {
    let pairing: Pairing
    let firstLaunch: Bool
    let finish: () -> Void
    let back: () -> Void
    @Environment(Session.self) private var session
    @State private var shown = 0
    @State private var rose = 0

    private var paired: SavedDevice? { pairing.paired }

    /// The new computer's Agents, as they arrive with its Bootstrap.
    private var found: [AgentItem] {
        paired.flatMap { session.device($0.address) }.map { session.agents(of: $0).byUrgency() }
            ?? []
    }

    var body: some View {
        VStack(spacing: 0) {
            Spacer()
            ZStack {
                if let paired {
                    Rising(statuses: found.map(\.agent.status), shown: rose, label: paired.label)
                        .transition(.opacity)
                } else {
                    Nearby()
                        .transition(.opacity)
                }
            }
            .frame(height: 230)
            VStack(spacing: 10) {
                Text(title)
                    .font(.largeTitle.bold())
                    .multilineTextAlignment(.center)
                    .contentTransition(.opacity)
                Text(detail)
                    .font(.body)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.top, 18)
            if paired != nil, !found.isEmpty {
                VStack(spacing: 0) {
                    ForEach(Array(found.enumerated()), id: \.element.id) { index, item in
                        HStack(spacing: 13) {
                            AgentAvatar(
                                kind: item.agent.kind, status: item.agent.status, side: 34,
                                surface: Color(.secondarySystemBackground))
                            VStack(alignment: .leading, spacing: 1) {
                                Text(item.pane.name)
                                    .font(.subheadline.weight(.semibold))
                                agentStatusLine(item.agent, kind: true)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer(minLength: 0)
                        }
                        .padding(.horizontal, 14)
                        .padding(.vertical, 11)
                        .opacity(shown > index ? 1 : 0)
                        .offset(y: shown > index ? 0 : 12)
                        if index < found.count - 1 {
                            Divider().padding(.leading, 61)
                        }
                    }
                }
                .background(
                    Color(.secondarySystemBackground),
                    in: .rect(cornerRadius: 22, style: .continuous)
                )
                .padding(.top, 32)
            }
            Spacer()
            Spacer()
        }
        .padding(.horizontal, 28)
        .frame(maxWidth: .infinity)
        .navigationBarBackButtonHidden()
        .safeAreaBar(edge: .bottom) {
            switch pairing.phase {
            case .paired:
                StepFooter {
                    PrimaryButton(firstLaunch ? "Show My Agents" : "Done", action: finish)
                } secondary: {
                }
                .transition(.move(edge: .bottom).combined(with: .opacity))
            case .failed:
                StepFooter {
                    PrimaryButton("Try Again") { pairing.retry() }
                } secondary: {
                    Button("Back", action: back)
                        .font(.subheadline.weight(.medium))
                }
                .transition(.move(edge: .bottom).combined(with: .opacity))
            case .pairing:
                EmptyView()
            }
        }
        .sensoryFeedback(.success, trigger: paired)
        .task(id: found.count) {
            // Each Agent's bar rises, then its row, as they come.
            guard paired != nil else { return }
            for index in rose..<found.count {
                try? await Task.sleep(for: .milliseconds(index == 0 ? 250 : 110))
                withAnimation(.spring(duration: 0.55, bounce: 0.35)) { rose = index + 1 }
            }
            for index in shown..<found.count {
                try? await Task.sleep(for: .milliseconds(index == 0 ? 300 : 140))
                withAnimation(.spring(duration: 0.5, bounce: 0.3)) { shown = index + 1 }
            }
        }
    }

    private var title: LocalizedStringResource {
        switch pairing.phase {
        case .pairing: "Pairing…"
        case .paired(let device): "Paired with \(device.label)"
        case .failed: "Couldn't pair"
        }
    }

    private var detail: LocalizedStringResource {
        switch pairing.phase {
        case .pairing: "Exchanging keys with your computer."
        case .paired: "Its agents show up here now."
        case .failed(let reason): "\(reason)"
        }
    }
}
