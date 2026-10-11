import SwiftUI

@main
struct CondrApp: App {
    @State private var store = Store()

    var body: some Scene {
        WindowGroup {
            Root()
                .environment(store)
        }
    }
}

struct Root: View {
    @Environment(Store.self) private var store
    @Environment(\.scenePhase) private var phase
    @State private var leftAt: Date?

    var body: some View {
        @Bindable var store = store
        ZStack {
            if store.paired {
                MainTabs()
                    .transition(.opacity.combined(with: .scale(scale: 1.04)))
            } else {
                PairingFlow(firstLaunch: true)
                    .transition(.opacity)
            }
        }
        .animation(.smooth(duration: 0.6), value: store.paired)
        .overlay(alignment: .top) { BannerView() }
        .overlay(alignment: .bottom) { ErrorToast() }
        .overlay {
            if store.locked {
                LockView().transition(.opacity)
            }
        }
        .animation(.smooth, value: store.locked)
        .sheet(isPresented: $store.pairingAnother) {
            PairingFlow(firstLaunch: false)
        }
        .sensoryFeedback(trigger: store.banner) { _, banner in
            switch banner?.status {
            case .blocked?: .warning
            case .done?: .success
            default: nil
            }
        }
        .onChange(of: phase) { _, phase in
            // The app lock (ADR 0041): on start, and after five minutes away.
            switch phase {
            case .background: leftAt = .now
            case .active:
                if let leftAt, Date.now.timeIntervalSince(leftAt) > 300, store.paired { store.locked = true }
                leftAt = nil
            default: break
            }
        }
    }
}

struct MainTabs: View {
    @Environment(Store.self) private var store

    var body: some View {
        @Bindable var store = store
        TabView(selection: $store.tab) {
            Tab("Agents", image: "tab-agents", value: Store.Tab.agents) {
                AgentsView()
            }
            .badge(store.agents(.blocked).count)
            Tab("Devices", systemImage: "desktopcomputer", value: Store.Tab.devices) {
                DevicesView()
            }
            Tab("Settings", systemImage: "gearshape", value: Store.Tab.settings) {
                SettingsView()
            }
        }
        .tabBarMinimizeBehavior(.onScrollDown)
    }
}

/// An Agent elsewhere started to need you or finished, while you look at something else.
struct BannerView: View {
    @Environment(Store.self) private var store

    var body: some View {
        Group {
            if let banner = store.banner, store.showing != banner.ref, !store.locked {
                Button { store.open(banner.ref) } label: {
                    HStack(spacing: 12) {
                        AgentAvatar(kind: banner.kind, status: banner.status, side: 38, surface: Color(.systemBackground))
                        VStack(alignment: .leading, spacing: 1) {
                            Text(banner.title)
                                .font(.subheadline.weight(.semibold))
                            Text(banner.detail)
                                .font(.footnote)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                        }
                        Spacer(minLength: 0)
                        Text("Open")
                            .font(.footnote.weight(.semibold))
                            .foregroundStyle(banner.status.tint)
                    }
                    .padding(.leading, 12)
                    .padding(.trailing, 18)
                    .padding(.vertical, 11)
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .glassEffect(.regular.interactive(), in: .rect(cornerRadius: 28, style: .continuous))
                .padding(.horizontal, 10)
                .gesture(DragGesture(minimumDistance: 8).onEnded { value in
                    if value.translation.height < 0 { store.banner = nil }
                })
                .transition(.move(edge: .top).combined(with: .opacity))
                .task(id: banner.id) {
                    try? await Task.sleep(for: .seconds(6))
                    if store.banner?.id == banner.id { store.banner = nil }
                }
                .accessibilityHint("Opens its terminal")
            }
        }
        .animation(.spring(duration: 0.5, bounce: 0.25), value: store.banner)
    }
}

/// A failed operation as one message to read, copy and dismiss (ADR 0020).
struct ErrorToast: View {
    @Environment(Store.self) private var store

    var body: some View {
        Group {
            if let message = store.error {
                HStack(alignment: .firstTextBaseline, spacing: 12) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                    Text(message)
                        .font(.subheadline)
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                    Button("Copy") { UIPasteboard.general.string = message }
                        .font(.subheadline.weight(.semibold))
                    Button("Dismiss", systemImage: "xmark") { store.error = nil }
                        .labelStyle(.iconOnly)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.secondary)
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 14)
                .glassEffect(.regular, in: .rect(cornerRadius: 24, style: .continuous))
                .padding(.horizontal, 12)
                .padding(.bottom, 86)
                .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(.spring(duration: 0.45, bounce: 0.2), value: store.error)
    }
}

/// Face ID before anything shows (ADR 0041). The prototype stands in for LocalAuthentication.
struct LockView: View {
    @Environment(Store.self) private var store
    @State private var scanning = false

    var body: some View {
        ZStack {
            Rectangle().fill(.regularMaterial).ignoresSafeArea()
            VStack(spacing: 0) {
                Spacer()
                AppMark(side: 80)
                    .shadow(color: .black.opacity(0.15), radius: 16, y: 8)
                Text("Condr is locked")
                    .font(.title2.bold())
                    .padding(.top, 24)
                Text("Unlock to reach your computers.")
                    .foregroundStyle(.secondary)
                    .padding(.top, 6)
                Image(systemName: "faceid")
                    .font(.system(size: 44, weight: .light))
                    .foregroundStyle(scanning ? Color.accentColor : Color.secondary)
                    .symbolEffect(.pulse, isActive: scanning)
                    .padding(.top, 44)
                Spacer()
                PrimaryButton("Unlock with Face ID", action: unlock)
                .padding(.horizontal, 28)
                .padding(.bottom, 12)
            }
        }
        .task { if store.live { unlock() } }
    }

    private func unlock() {
        scanning = true
        Task {
            try? await Task.sleep(for: .seconds(0.9))
            scanning = false
            store.locked = false
        }
    }
}
