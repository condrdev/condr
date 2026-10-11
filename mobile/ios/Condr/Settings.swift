import SwiftUI

struct SettingsView: View {
    @Environment(Store.self) private var store
    @State private var screenWidth: CGFloat = 402

    var body: some View {
        @Bindable var store = store
        NavigationStack(path: $store.settingsPath) {
            Form {
                Section {
                    HStack(spacing: 14) {
                        AppMark(side: 52)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(store.phoneName)
                                .font(.headline)
                            Text("Paired with \(store.devices.count) computer\(store.devices.count == 1 ? "" : "s")")
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .padding(.vertical, 4)
                    LabeledContent("Key") {
                        Text(store.fingerprint.prefix(6) + "…" + store.fingerprint.suffix(6))
                            .font(.callout.monospaced())
                            .foregroundStyle(.secondary)
                    }
                    .contextMenu {
                        Button("Copy Key", systemImage: "doc.on.doc") { UIPasteboard.general.string = store.fingerprint }
                    }
                } footer: {
                    Text("Your computers list this iPhone under Paired devices by this key. It never leaves this iPhone.")
                }

                Section("Terminal") {
                    VStack(alignment: .leading, spacing: 12) {
                        HStack {
                            Text("Text Size")
                            Spacer()
                            Text("\(store.fontSize, specifier: "%.1f") pt")
                                .monospacedDigit()
                                .foregroundStyle(.secondary)
                        }
                        Slider(value: $store.fontSize, in: 8...20, step: 0.5) {
                            Text("Text Size")
                        } minimumValueLabel: {
                            Image(systemName: "textformat.size.smaller")
                        } maximumValueLabel: {
                            Image(systemName: "textformat.size.larger")
                        }
                        preview
                    }
                    .padding(.vertical, 4)
                }

                Section {
                    LabeledContent("App Lock", value: "Face ID")
                    NavigationLink("If You Lose This iPhone", value: "lost")
                } header: {
                    Text("Security")
                } footer: {
                    Text("Condr asks for Face ID when it opens and after five minutes away.")
                }

                Section {
                    LabeledContent("Version", value: "0.1 (1)")
                    Link(destination: URL(string: "https://condr.dev")!) {
                        HStack {
                            Text("condr.dev")
                            Spacer()
                            Image(systemName: "arrow.up.right")
                                .font(.footnote.weight(.semibold))
                                .foregroundStyle(.tertiary)
                        }
                    }
                }

                Section {
                    Button("Replay First Launch") { store.devices = []; store.paired = false }
                    Button("Lock Now") { store.locked = true }
                    Button("Show an Error") { store.error = "Couldn't send to macmini: the connection closed. Your text was not sent." }
                } header: {
                    Text("Prototype")
                }
            }
            .navigationTitle("Settings")
            .navigationDestination(for: String.self) { _ in LostPhone() }
            .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { screenWidth = $0 }
        }
    }

    private var preview: some View {
        let cell = Cell(size: store.fontSize)
        // The terminal page's own arithmetic: the screen's width less its gutters.
        let columns = Int((screenWidth - 20) / cell.width)
        return VStack(alignment: .leading, spacing: 6) {
            TerminalCanvas(lines: [
                [t("● ", Palette.green), t("Bash", Palette.brightWhite, bold: true), t("(cargo test)")],
                [t("  ⎿  ", 0x8b949e), t("test result: "), t("ok", Palette.green, bold: true), t(". 46 passed")],
            ], cell: cell, columns: 44)
            .padding(10)
            .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
            .clipped()
            .background(Color(hex: Palette.background), in: .rect(cornerRadius: 12, style: .continuous))
            Text("About \(columns) columns in portrait")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
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
                step(1, "On each computer this iPhone is paired with, open Condr's Settings › Paired devices and remove it. Each computer keeps its own list, so removing it on one doesn't remove it on the others.")
                step(2, "Then look over each list for a device you don't recognize, and remove that too.")
                step(3, "A new iPhone is a new device: pair it with an invite as you did this one.")
            }
            .padding(24)
        }
        .navigationBarTitleDisplayMode(.inline)
    }

    private func step(_ number: Int, _ text: String) -> some View {
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
