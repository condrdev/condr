import SwiftUI

/// Every paired Device and, inside one, its Workspaces and Panes (ADR 0041). Nothing here
/// changes Session structure: the phone opens Panes, it never creates or closes them.
struct DevicesView: View {
    @Environment(Store.self) private var store

    var body: some View {
        @Bindable var store = store
        NavigationStack(path: $store.devicesPath) {
            List {
                Section {
                    ForEach(store.devices) { device in
                        NavigationLink(value: DeviceRoute.device(device.address)) {
                            DeviceRow(device: device)
                        }
                    }
                } footer: {
                    Text("This iPhone connects while Condr is open. In the background, your computers keep working.")
                }
                Section {
                    Button {
                        store.pairingAnother = true
                    } label: {
                        Label("Pair a Computer", systemImage: "qrcode.viewfinder")
                    }
                }
            }
            .navigationTitle("Devices")
            .navigationDestination(for: DeviceRoute.self) { route in
                switch route {
                case .device(let address): DeviceScreen(address: address)
                case .pane(let ref): TerminalScreen(ref: ref)
                }
            }
        }
    }
}

/// A computer: the desktop symbol with its connection state in the corner.
struct DeviceGlyph: View {
    let status: DeviceStatus
    var side: CGFloat = 40

    var body: some View {
        RoundedRectangle(cornerRadius: side * 0.28, style: .continuous)
            .fill(.fill.tertiary)
            .overlay {
                Image(systemName: "desktopcomputer")
                    .font(.system(size: side * 0.45, weight: .medium))
                    .foregroundStyle(status == .connected ? .primary : .secondary)
            }
            .frame(width: side, height: side)
            .overlay(alignment: .bottomTrailing) {
                Group {
                    switch status {
                    case .connected: Circle().fill(.green)
                    case .connecting: ProgressView().controlSize(.mini)
                    case .failed: Circle().fill(.red)
                    case .revoked: Circle().fill(.orange)
                    }
                }
                .frame(width: side * 0.26, height: side * 0.26)
                .padding(side * 0.06)
                .background(Circle().fill(Color(.secondarySystemGroupedBackground)))
                .offset(x: side * 0.12, y: side * 0.12)
            }
    }
}

/// A computer: its name with its Agents drawn small beside it, and in words below.
struct DeviceRow: View {
    let device: Device

    var body: some View {
        HStack(spacing: 14) {
            DeviceGlyph(status: device.status)
            VStack(alignment: .leading, spacing: 3) {
                HStack(alignment: .bottom, spacing: 8) {
                    Text(device.label)
                        .font(.headline)
                        .lineLimit(1)
                    Spacer(minLength: 0)
                    if device.status == .connected, !statuses.isEmpty {
                        MiniScore(statuses: statuses)
                    }
                }
                status
                    .font(.subheadline)
                    .lineLimit(1)
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .combine)
    }

    private var statuses: [AgentStatus] { device.agents.compactMap { $0.agent?.status } }

    @ViewBuilder
    private var status: some View {
        switch device.status {
        case .connected: agentSummary(statuses).foregroundStyle(.secondary)
        case .connecting: Text("Connecting…").foregroundStyle(.secondary)
        case .failed: Text("Can't connect").foregroundStyle(.red)
        case .revoked: Text("Needs pairing again").foregroundStyle(.orange)
        }
    }
}

/// A Device's Agents as the score draws them, small enough to sit beside its name.
struct MiniScore: View {
    let statuses: [AgentStatus]

    var body: some View {
        let fit = Bars.fit(count: statuses.count, width: 96, widest: 5, spacing: 3)
        Bars(statuses: statuses.byUrgency(), tall: 20, widest: fit.width, spacing: fit.spacing, animated: false)
    }
}

struct DeviceScreen: View {
    let address: String
    @Environment(Store.self) private var store
    @State private var forgetting = false

    var body: some View {
        if let device = store.device(address) {
            Group {
                switch device.status {
                case .connected: workspaces(device)
                case .connecting: ProgressView("Connecting to \(device.label)…")
                case .failed(let reason): unreachable(device, reason: reason)
                case .revoked: revoked(device)
                }
            }
            .navigationTitle(device.label)
            .navigationSubtitle(subtitle(device))
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        Section(address) {
                            Button("Forget \(device.label)", systemImage: "trash", role: .destructive) { forgetting = true }
                        }
                    } label: {
                        Image(systemName: "ellipsis")
                    }
                }
            }
            .confirmationDialog("Forget \(device.label)?", isPresented: $forgetting, titleVisibility: .visible) {
                Button("Forget \(device.label)", role: .destructive) { store.forget(address) }
            } message: {
                Text("This iPhone stops connecting to it. To revoke this iPhone as well, remove it under Paired devices in Condr's Settings on \(device.label).")
            }
        }
    }

    private func subtitle(_ device: Device) -> String {
        switch device.status {
        case .connected: "Connected"
        case .connecting: "Connecting…"
        case .failed: "Can't connect"
        case .revoked: "Needs pairing again"
        }
    }

    private func workspaces(_ device: Device) -> some View {
        let statuses = device.agents.compactMap { $0.agent?.status }
        return List {
            if !statuses.isEmpty {
                // The home row's small score, grown: this computer at a glance.
                Section {
                    VStack(alignment: .leading, spacing: 12) {
                        GeometryReader { proxy in
                            let fit = Bars.fit(count: statuses.count, width: proxy.size.width, widest: 16, spacing: 7)
                            Bars(statuses: statuses.byUrgency(), tall: proxy.size.height, widest: fit.width, spacing: fit.spacing)
                        }
                        .frame(height: 64)
                        agentSummary(statuses)
                            .font(.subheadline.weight(.medium))
                            .foregroundStyle(.secondary)
                    }
                    .listRowBackground(Color.clear)
                    .listRowInsets(EdgeInsets(top: 4, leading: 4, bottom: 4, trailing: 4))
                    .accessibilityElement(children: .combine)
                }
            }
            ForEach(device.workspaces) { workspace in
                Section {
                    ForEach(workspace.panes) { pane in
                        let ref = PaneRef(device: device.address, pane: pane.id)
                        NavigationLink(value: DeviceRoute.pane(ref)) {
                            PaneRow(pane: pane)
                        }
                    }
                } header: {
                    HStack(spacing: 9) {
                        WorkspaceAvatar(name: workspace.name, side: 22)
                        VStack(alignment: .leading, spacing: 0) {
                            Text(workspace.name)
                                .font(.subheadline.weight(.semibold))
                                .foregroundStyle(.primary)
                            Text(workspace.root)
                                .font(.caption.monospaced())
                                .foregroundStyle(.secondary)
                        }
                    }
                    .textCase(nil)
                    .padding(.bottom, 4)
                }
            }
        }
        .contentMargins(.top, 8, for: .scrollContent)
    }

    private func unreachable(_ device: Device, reason: String) -> some View {
        ContentUnavailableView {
            Label("Can't reach \(device.label)", systemImage: "desktopcomputer.trianglebadge.exclamationmark")
        } description: {
            Text(reason)
        } actions: {
            Button { store.retry(device.address) } label: {
                Text("Try Again").foregroundStyle(Color(.systemBackground))
            }
                .buttonStyle(.glassProminent)
                .controlSize(.large)
            Button("Copy Error") { UIPasteboard.general.string = reason }
                .buttonStyle(.glass)
        }
    }

    private func revoked(_ device: Device) -> some View {
        ContentUnavailableView {
            Label("\(device.label) doesn't know this iPhone", systemImage: "person.badge.key")
        } description: {
            Text("It was removed from Paired devices on \(device.label). Create an invite there and scan it to connect again.")
        } actions: {
            Button { store.pairingAnother = true } label: {
                Text("Pair Again").foregroundStyle(Color(.systemBackground))
            }
                .buttonStyle(.glassProminent)
                .controlSize(.large)
        }
    }
}

struct PaneRow: View {
    let pane: Pane

    var body: some View {
        HStack(spacing: 13) {
            AgentAvatar(kind: pane.agent?.kind, status: pane.agent?.status, side: 32, surface: Color(.secondarySystemGroupedBackground))
            VStack(alignment: .leading, spacing: 1) {
                Text(pane.title)
                    .font(.body)
                    .lineLimit(1)
                if let agent = pane.agent {
                    Text(agent.status == .blocked ? (agent.blockedOn ?? agent.status.title) : agent.status.title)
                        .font(.subheadline)
                        .foregroundStyle(agent.status == .idle || agent.status == .unknown ? .secondary : agent.status.tint)
                        .lineLimit(1)
                } else {
                    Text("Shell")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 0)
            if pane.attention {
                Image(systemName: "bell.fill")
                    .font(.caption)
                    .foregroundStyle(.orange)
                    .accessibilityLabel("Rang the bell")
            }
        }
        .padding(.vertical, 2)
    }
}
