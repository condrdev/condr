import CondrKit
import SwiftUI

/// Every paired Device and, inside one, its Workspaces and Panes (ADR 0041). Nothing here
/// changes Session structure: the phone opens Panes, it never creates or closes them.
struct DevicesView: View {
    @Environment(Session.self) private var session
    @Environment(Router.self) private var router

    var body: some View {
        @Bindable var router = router
        NavigationStack(path: $router.devicesPath) {
            List {
                Section {
                    ForEach(session.devices) { device in
                        NavigationLink(value: DeviceRoute.device(device.address)) {
                            DeviceRow(device: device)
                        }
                    }
                }
                Section {
                    Button {
                        router.pairingAnother = true
                    } label: {
                        Label("Pair a Computer", systemImage: "qrcode.viewfinder")
                    }
                }
            }
            .navigationTitle("Devices")
            .navigationDestination(for: DeviceRoute.self) { route in
                switch route {
                case .device(let address): DeviceScreen(address: address)
                case .pane(let ref): TerminalPage(ref: ref)
                }
            }
        }
    }
}

/// A computer drawn as its own score: its Agents' bars on a tile, so the Devices list
/// speaks the first screen's language and green or red never stand for a connection. A
/// computer it can't reach is the score's dashed placeholder.
struct DeviceTile: View {
    let device: DeviceView
    var side: CGFloat = 44
    @Environment(Session.self) private var session

    var body: some View {
        let statuses = session.agents(of: device).map(\.agent.status).byUrgency()
        RoundedRectangle(cornerRadius: side * 0.28, style: .continuous)
            .fill(.fill.tertiary)
            .overlay {
                switch device.status {
                case .connected where !statuses.isEmpty:
                    let fit = Bars.fit(
                        count: statuses.count, width: side * 0.66, widest: side * 0.13,
                        spacing: side * 0.06)
                    Bars(
                        statuses: statuses, tall: side * 0.5, widest: fit.width,
                        spacing: fit.spacing, animated: false)
                case .connected:
                    Image(systemName: "desktopcomputer")
                        .font(.system(size: side * 0.4, weight: .medium))
                        .foregroundStyle(.secondary)
                case .connecting, .suspended:
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
    let device: DeviceView
    @Environment(Session.self) private var session

    var body: some View {
        HStack(spacing: 14) {
            DeviceTile(device: device)
            VStack(alignment: .leading, spacing: 2) {
                Text(device.label)
                    .font(.headline)
                    .lineLimit(1)
                status
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder
    private var status: some View {
        switch device.status {
        case .connected: agentSummary(session.agents(of: device).map(\.agent.status))
        case .connecting, .suspended: Text("Connecting…")
        case .failed: Text("Can't connect")
        case .revoked: Text("Needs pairing again")
        }
    }
}

struct DeviceScreen: View {
    let address: String
    @Environment(Session.self) private var session
    @Environment(Router.self) private var router
    @State private var forgetting = false
    @State private var renaming = false
    @State private var newLabel = ""

    var body: some View {
        if let device = session.device(address) {
            Group {
                switch device.status {
                case .connected: workspaces(device)
                case .connecting, .suspended: ProgressView("Connecting to \(device.label)…")
                case .failed(let reason): unreachable(device, reason: reason)
                case .revoked: revoked(device)
                }
            }
            .navigationTitle(device.label)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        Button("Rename…", systemImage: "pencil") {
                            newLabel = device.label
                            renaming = true
                        }
                        Button("Forget \(device.label)", systemImage: "trash", role: .destructive) {
                            forgetting = true
                        }
                    } label: {
                        Image(systemName: "ellipsis")
                    }
                    .alert("Rename \(device.label)", isPresented: $renaming) {
                        TextField("Name", text: $newLabel)
                            .textInputAutocapitalization(.words)
                        Button("Rename") { session.rename(address, to: newLabel) }
                            .disabled(
                                newLabel.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        Button("Cancel", role: .cancel) {}
                    }
                    // iOS 26 grows the dialog out of the view it hangs on: the button that asked.
                    .confirmationDialog(
                        "Forget \(device.label)?", isPresented: $forgetting,
                        titleVisibility: .visible
                    ) {
                        Button("Forget \(device.label)", role: .destructive) {
                            session.forget(address)
                        }
                    } message: {
                        Text(
                            "This iPhone stops connecting to it. To revoke this iPhone as well, remove it under Paired devices in Condr's Settings on \(device.label)."
                        )
                    }
                }
            }
        } else {
            // Forgotten while this screen was up: the stack is about to drop it.
            Color.clear
        }
    }

    private func workspaces(_ device: DeviceView) -> some View {
        let statuses = session.agents(of: device).map(\.agent.status)
        return List {
            if !statuses.isEmpty {
                // The home row's small score, grown: this computer at a glance.
                Section {
                    VStack(alignment: .leading, spacing: 12) {
                        GeometryReader { proxy in
                            let fit = Bars.fit(
                                count: statuses.count, width: proxy.size.width, widest: 16,
                                spacing: 7)
                            Bars(
                                statuses: statuses.byUrgency(), tall: proxy.size.height,
                                widest: fit.width,
                                spacing: fit.spacing)
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
            ForEach(session.workspaces[device.address] ?? []) { workspace in
                Section {
                    ForEach(workspace.panes) { pane in
                        NavigationLink(
                            value: DeviceRoute.pane(PaneRef(address: device.address, pane: pane.id))
                        ) {
                            PaneRow(pane: pane)
                        }
                    }
                } header: {
                    HStack(spacing: 9) {
                        WorkspaceAvatar(name: workspace.name, side: 22)
                        Text(workspace.name)
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(.primary)
                    }
                    .textCase(nil)
                    .padding(.bottom, 4)
                }
            }
        }
        .contentMargins(.top, 8, for: .scrollContent)
    }

    private func unreachable(_ device: DeviceView, reason: String) -> some View {
        ContentUnavailableView {
            Gone(title: "Can't reach \(device.label)")
        } description: {
            Text(reason)
        } actions: {
            PrimaryButton("Try Again") { session.resume() }
            Button("Copy Error") { UIPasteboard.general.string = reason }
                .buttonStyle(.borderless)
        }
    }

    private func revoked(_ device: DeviceView) -> some View {
        ContentUnavailableView {
            Gone(title: "\(device.label) doesn't know this iPhone")
        } description: {
            Text(
                "It was removed from Paired devices on \(device.label). Create an invite there and scan it to connect again."
            )
        } actions: {
            PrimaryButton("Pair Again") { router.pairingAnother = true }
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
    let pane: PaneView

    var body: some View {
        HStack(spacing: 13) {
            AgentAvatar(
                kind: pane.agent?.kind, status: pane.agent?.status, side: 32,
                surface: Color(.secondarySystemGroupedBackground))
            VStack(alignment: .leading, spacing: 1) {
                Text(pane.name)
                    .font(.body)
                    .lineLimit(1)
                Group {
                    if let agent = pane.agent {
                        agentStatusLine(agent)
                    } else if pane.exited {
                        Text("Exited")
                    } else {
                        Text("Shell")
                    }
                }
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .lineLimit(1)
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

/// An Agent's state in words. The glyph carries the colour; only a wait is also said in
/// red, in bold, with what it waits for.
func agentStatusLine(_ agent: AgentView, kind: Bool = false) -> Text {
    let prefix = kind ? Text("\(agent.label) · ") : Text("")
    if agent.status == .blocked {
        let need = Text(AgentStatus.blocked.title).fontWeight(.semibold).foregroundStyle(
            AgentStatus.blocked.tint)
        let on = agent.blockedOn.map { " · \($0)" } ?? ""
        return Text("\(prefix)\(need)\(on)")
    }
    return Text("\(prefix)\(agent.status.title)")
}
