import Observation

enum AppTab: Hashable {
    case agents, devices, settings
}

enum DeviceRoute: Hashable {
    case device(String)
    case pane(PaneRef)
}

enum SettingsRoute: Hashable {
    case lostPhone
}

/// Where the app is: the tab and each tab's stack. Opening a Pane from a banner or the
/// score lands on the Agents tab, and forgetting a Device drops every route into it.
@Observable
@MainActor
final class Router {
    var tab: AppTab = .agents
    var agentsPath: [DeviceRoute] = []
    var devicesPath: [DeviceRoute] = []
    var settingsPath: [SettingsRoute] = []
    var pairingAnother = false

    /// The first screen is showing, which shows a change itself: a banner there is felt, not shown.
    var showsHome: Bool { tab == .agents && agentsPath.isEmpty }

    /// Opens a Pane from anywhere: a banner, the score.
    func open(_ ref: PaneRef) {
        tab = .agents
        agentsPath = [.pane(ref)]
    }

    func forget(_ address: String) {
        let stays = { (route: DeviceRoute) -> Bool in
            switch route {
            case .device(let routed): routed != address
            case .pane(let ref): ref.address != address
            }
        }
        agentsPath.removeAll { !stays($0) }
        devicesPath.removeAll { !stays($0) }
    }
}
