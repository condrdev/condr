import CondrKit
import SwiftUI

struct SettingsView: View {
    @Environment(Session.self) private var session
    @Environment(Router.self) private var router
    @Environment(Preferences.self) private var preferences
    @Environment(AppLock.self) private var lock
    @State private var screenWidth: CGFloat = 402

    var body: some View {
        @Bindable var router = router
        @Bindable var preferences = preferences
        NavigationStack(path: $router.settingsPath) {
            Form {
                Section {
                    NavigationLink(value: SettingsRoute.phoneName) {
                        HStack(spacing: 14) {
                            RoundedRectangle(cornerRadius: 14, style: .continuous)
                                .fill(.fill.tertiary)
                                .overlay {
                                    Image(systemName: "iphone")
                                        .font(.system(size: 26, weight: .light))
                                        .symbolRenderingMode(.hierarchical)
                                }
                                .frame(width: 52, height: 52)
                                .accessibilityHidden(true)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(preferences.phoneName)
                                    .font(.headline)
                                Text(
                                    "Paired with ^[\(session.devices.count) computer](inflect: true)"
                                )
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                            }
                        }
                        .padding(.vertical, 4)
                    }
                }

                Section("Terminal") {
                    VStack(alignment: .leading, spacing: 12) {
                        HStack {
                            Text("Text Size")
                            Spacer()
                            Text("\(preferences.fontSize, specifier: "%.1f") pt")
                                .monospacedDigit()
                                .foregroundStyle(.secondary)
                        }
                        Slider(
                            value: $preferences.fontSize,
                            in: Preferences.smallestFont...Preferences.largestFont,
                            step: 0.5
                        ) {
                            Text("Text Size")
                        } minimumValueLabel: {
                            Text("A").font(.caption2)
                        } maximumValueLabel: {
                            Text("A").font(.title2)
                        }
                        preview
                    }
                    .padding(.vertical, 4)
                }

                Section {
                    LabeledContent("App Lock", value: lock.available ? "On" : "Off")
                    LabeledContent("Key") {
                        // The whole key, as the computer's Paired devices shows it, cut in the middle to fit.
                        Text(session.fingerprint)
                            .font(.callout.monospaced())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    .contextMenu {
                        Button("Copy Key", systemImage: "doc.on.doc") {
                            UIPasteboard.general.string = session.fingerprint
                        }
                    }
                    NavigationLink("If You Lose This iPhone", value: SettingsRoute.lostPhone)
                } header: {
                    Text("Security")
                } footer: {
                    Text(
                        lock.available
                            ? "Condr asks for Face ID, Touch ID or your passcode when it opens and after five minutes away."
                            : "Set a passcode on this iPhone and Condr locks when it opens and after five minutes away."
                    )
                }

                Section {
                    LabeledContent("Version", value: Bundle.main.versionText)
                    if let site = URL(string: "https://condr.dev") {
                        Link(destination: site) {
                            HStack {
                                Text("condr.dev")
                                Spacer()
                                Image(systemName: "arrow.up.right")
                                    .font(.footnote.weight(.semibold))
                                    .foregroundStyle(.tertiary)
                            }
                        }
                    }
                }
            }
            .navigationTitle("Settings")
            .navigationDestination(for: SettingsRoute.self) { route in
                switch route {
                case .phoneName: PhoneName()
                case .lostPhone: LostPhone()
                }
            }
            .onGeometryChange(for: CGFloat.self) {
                $0.size.width
            } action: {
                screenWidth = $0
            }
        }
    }

    /// Two lines of a terminal at the chosen size, and about how many columns the phone
    /// fits in portrait at it.
    private var preview: some View {
        let font = UIFont.monospacedSystemFont(ofSize: preferences.fontSize, weight: .regular)
        let cellWidth = ("0" as NSString).size(withAttributes: [.font: font]).width
        let columns = Int((screenWidth - 2 * TerminalPage.gutter) / cellWidth)
        return VStack(alignment: .leading, spacing: 6) {
            VStack(alignment: .leading, spacing: 2) {
                let dot = Text("● ").foregroundStyle(Color(hex: Palette.green))
                let tool = Text("Bash").bold().foregroundStyle(Color(hex: Palette.brightWhite))
                let elbow = Text("  ⎿  ").foregroundStyle(Color(hex: Palette.gray))
                let ok = Text("ok").bold().foregroundStyle(Color(hex: Palette.green))
                Text("\(dot)\(tool)(cargo test)")
                Text("\(elbow)test result: \(ok). 46 passed")
            }
            .font(.system(size: preferences.fontSize, design: .monospaced))
            .foregroundStyle(Color(hex: Palette.foreground))
            .lineLimit(1)
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                Color(hex: Palette.background), in: .rect(cornerRadius: 12, style: .continuous))
            Text("About \(columns) columns in portrait")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }
}

extension Bundle {
    /// "0.1 (1)"
    var versionText: String {
        let version = infoDictionary?["CFBundleShortVersionString"] as? String ?? "?"
        let build = infoDictionary?["CFBundleVersion"] as? String ?? "?"
        return "\(version) (\(build))"
    }
}

/// This phone's name, as each computer's Paired devices lists it (ADR 0040).
struct PhoneName: View {
    @Environment(Preferences.self) private var preferences
    @Environment(Session.self) private var session
    @Environment(\.dismiss) private var dismiss
    @State private var draft = ""
    @FocusState private var focused: Bool

    var body: some View {
        Form {
            HStack {
                TextField("Name", text: $draft)
                    .focused($focused)
                    .submitLabel(.done)
                    .onSubmit { dismiss() }
                if !draft.isEmpty {
                    Button("Clear", systemImage: "xmark.circle.fill") { draft = "" }
                        .labelStyle(.iconOnly)
                        .foregroundStyle(.tertiary)
                        .buttonStyle(.plain)
                }
            }
        }
        .navigationTitle("Name")
        .navigationBarTitleDisplayMode(.inline)
        .onAppear {
            draft = preferences.phoneName
            focused = true
        }
        // As the system's own name field does, leaving keeps what was typed.
        .onDisappear(perform: save)
    }

    private func save() {
        let name = draft.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty, name != preferences.phoneName else { return }
        preferences.phoneName = name
        Task { await session.rename(name) }
    }
}

/// What ADR 0040 asks the help to say: revocation is per computer.
struct LostPhone: View {
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Image(systemName: "iphone.slash")
                    .font(.system(size: 40, weight: .light))
                    .foregroundStyle(.secondary)
                Text("If you lose this iPhone")
                    .font(.title2.bold())
                step(
                    1,
                    "On each computer this iPhone is paired with, open Condr's Settings › Paired devices and remove it. Each computer keeps its own list, so removing it on one doesn't remove it on the others."
                )
                step(
                    2,
                    "Then look over each list for a device you don't recognize, and remove that too."
                )
                step(3, "A new iPhone is a new device: pair it with an invite as you did this one.")
            }
            .padding(24)
        }
        .navigationBarTitleDisplayMode(.inline)
    }

    private func step(_ number: Int, _ text: LocalizedStringResource) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text("\(number)")
                .font(.subheadline.weight(.bold).monospacedDigit())
                .frame(width: 24, height: 24)
                .background(.fill.tertiary, in: .circle)
            Text(text)
                .font(.body)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}
