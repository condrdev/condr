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

/// A computer drawn as its own score: its Agents' bars on a tile, so the Devices list
/// speaks the first screen's language and green or red never stand for a connection. A
/// computer it can't reach is the score's dashed placeholder.
struct DeviceTile: View {
    let device: Device
    var side: CGFloat = 44

    var body: some View {
        let statuses = device.agents.compactMap { $0.agent?.status }.byUrgency()
        RoundedRectangle(cornerRadius: side * 0.28, style: .continuous)
            .fill(.fill.tertiary)
            .overlay {
                switch device.status {
                case .connected where !statuses.isEmpty:
                    let fit = Bars.fit(count: statuses.count, width: side * 0.66, widest: side * 0.13, spacing: side * 0.06)
                    Bars(statuses: statuses, tall: side * 0.5, widest: fit.width, spacing: fit.spacing, animated: false)
                case .connected:
                    Image(systemName: "desktopcomputer")
                        .font(.system(size: side * 0.4, weight: .medium))
                        .foregroundStyle(.secondary)
                case .connecting:
                    ProgressView().controlSize(.small)
                case .failed, .revoked:
                    Fallen(width: side * 0.6)
                        .frame(height: side * 0.5, alignment: .bottom)
                }
            }
            .frame(width: side, height: side)
            .accessibilityHidden(true)
    }
}

/// A computer: its score as the tile, its name, and its Agents in words.
struct DeviceRow: View {
    let device: Device

    var body: some View {
        HStack(spacing: 14) {
            DeviceTile(device: device)
            VStack(alignment: .leading, spacing: 2) {
                Text(device.label)
                    .font(.headline)
                    .lineLimit(1)
                status
                    .font(.subheadline)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
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
        case .failed: Text("Can't connect").foregroundStyle(.secondary)
        case .revoked: Text("Needs pairing again").foregroundStyle(.secondary)
        }
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
            Gone(title: "Can't reach \(device.label)")
        } description: {
            Text(reason)
        } actions: {
            Button { store.retry(device.address) } label: {
                Text("Try Again").foregroundStyle(Color(.systemBackground))
            }
                .buttonStyle(.glassProminent)
                .controlSize(.large)
            Button("Copy Error") { UIPasteboard.general.string = reason }
                .buttonStyle(.borderless)
        }
    }

    private func revoked(_ device: Device) -> some View {
        ContentUnavailableView {
            Gone(title: "\(device.label) doesn't know this iPhone")
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

/// The head of a computer it can't reach: the score's fallen bars over what went wrong.
struct Gone: View {
    let title: String

    var body: some View {
        VStack(spacing: 22) {
            Fallen(width: 96)
            Text(title)
                .font(.title2.bold())
                .multilineTextAlignment(.center)
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
                    // The glyph carries the colour; only a wait is also said in red, in bold.
                    Group {
                        if agent.status == .blocked {
                            Text("\(Text("Needs you").fontWeight(.semibold).foregroundStyle(.red))\(agent.blockedOn.map { " · \($0)" } ?? "")")
                        } else {
                            Text(agent.status.title)
                        }
                    }
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
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
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Rang the bell")
            }
        }
        .padding(.vertical, 2)
    }
}
