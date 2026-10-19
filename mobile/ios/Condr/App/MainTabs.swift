import SwiftUI

struct MainTabs: View {
    @Environment(Session.self) private var session
    @Environment(Router.self) private var router

    var body: some View {
        @Bindable var router = router
        TabView(selection: $router.tab) {
            Tab(value: AppTab.agents) {
                AgentsView()
            } label: {
                Label("Agents", image: .tabAgents)
            }
            .badge(session.agents(.blocked, .error).count)
            Tab("Devices", systemImage: "desktopcomputer", value: AppTab.devices) {
                DevicesView()
            }
            Tab("Settings", systemImage: "gearshape", value: AppTab.settings) {
                SettingsView()
            }
        }
        .tabBarMinimizeBehavior(.onScrollDown)
    }
}
