import SwiftUI

/// A visit to the sample world, over the pairing page. Its preferences and Devices live
/// separately from the real app, and leaving returns to the same step of pairing.
struct DemoExperience: View {
    @Environment(\.dismiss) private var dismiss
    @State private var suite = "dev.condr.companion.preview.\(UUID().uuidString)"
    @State private var stores: Stores?

    var body: some View {
        VStack(spacing: 0) {
            demoBar
            if let stores {
                MainTabs()
                    .overlay(alignment: .top) { BannerView() }
                    .overlay(alignment: .bottom) {
                        if stores.session.shown == nil {
                            VStack(spacing: 8) {
                                NoticeToast()
                                ErrorToast()
                            }
                            .padding(.bottom, 86)
                        }
                    }
                    .environment(stores.session)
                    .environment(stores.router)
                    .environment(stores.preferences)
                    .environment(stores.lock)
                    .environment(stores.alerts)
                    // Pairing always happens in the real app, never against sample Devices.
                    .onChange(of: stores.router.pairingAnother) { _, pairing in
                        if pairing { dismiss() }
                    }
                    .onChange(of: stores.session.paired) { _, paired in
                        if !paired { dismiss() }
                    }
            } else {
                ProgressView()
            }
        }
        .onAppear {
            if stores == nil { stores = Stores.demo(.agents, suite: suite) }
        }
        .onDisappear {
            UserDefaults.standard.removePersistentDomain(forName: suite)
        }
    }

    private var demoBar: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Demo Mode").font(.subheadline.weight(.semibold))
                Text("Sample data · No live connection")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
            Button("Exit Demo") { dismiss() }
                .font(.subheadline.weight(.semibold))
                .frame(minHeight: 44)
                .accessibilityIdentifier("exitDemo")
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 4)
        .background(.bar)
    }
}
