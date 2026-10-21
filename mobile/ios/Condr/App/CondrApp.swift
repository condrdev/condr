import CondrKit
import Network
import SwiftUI
import Synchronization
import UserNotifications

@main
struct CondrApp: App {
    @UIApplicationDelegateAdaptor private var delegate: AppDelegate

    var body: some Scene {
        WindowGroup {
            switch delegate.stores {
            case .success(let stores):
                Root()
                    .environment(stores.session)
                    .environment(stores.router)
                    .environment(stores.preferences)
                    .environment(stores.lock)
                    .environment(stores.alerts)
                    .environment(stores.push)
            case .failure(let error):
                StartupFailure(message: error.localizedDescription)
            }
        }
    }
}

/// Builds the app's objects as iOS launches it, so a notification tapped to launch it
/// finds them, and hands them what APNs and the notification center say (ADR 0047).
final class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    let stores = Result { try Stores.make() }

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions options: [UIApplication.LaunchOptionsKey: Any]?
    ) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        return true
    }

    func application(
        _ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken token: Data
    ) {
        guard let push = try? stores.get().push else { return }
        Task { await push.received(token: token) }
    }

    func application(
        _ application: UIApplication,
        didFailToRegisterForRemoteNotificationsWithError error: any Error
    ) {
        guard let push = try? stores.get().push else { return }
        push.failed()
    }

    /// In the foreground a notification shows only while its Device is not connected here.
    /// The completion handler is called on the main thread: the async form of these
    /// methods finishes on a background thread, and UIKit aborts the app for it.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification,
        withCompletionHandler completionHandler:
            @escaping @Sendable (UNNotificationPresentationOptions) -> Void
    ) {
        let pushed = PushedPane(notification.request.content.userInfo)
        Task { @MainActor in
            completionHandler(presents(pushed) ? [.banner, .list, .sound] : [])
        }
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping @Sendable () -> Void
    ) {
        let pushed = PushedPane(response.notification.request.content.userInfo)
        Task { @MainActor in
            if let pushed { open(pushed) }
            completionHandler()
        }
    }

    private func presents(_ pushed: PushedPane?) -> Bool {
        guard let pushed, let session = try? stores.get().session else { return true }
        return session.presents(pushed)
    }

    /// The Pane a tapped notification names (ADR 0041), over its Device's Workspace list.
    private func open(_ pushed: PushedPane) {
        guard let stores = try? stores.get(), let ref = stores.session.ref(pushed) else { return }
        stores.router.openNotified(ref)
    }
}

/// The app's objects, built once at launch: over the library, or over a demo scene.
@MainActor
struct Stores {
    let session: Session
    let router: Router
    let preferences: Preferences
    let lock: AppLock
    let alerts: Alerts
    /// None in a demo scene, which registers nowhere.
    let push: Push?

    static func make() throws -> Stores {
        if let scene = DemoScene.launched { return demo(scene) }
        let preferences = Preferences()
        let router = Router()
        let alerts = Alerts()
        let saved = SavedDeviceStore.shared()
        let seed = try DeviceKey.load()
        let listener = SessionListener()
        let client = try Companion(
            seed: seed, name: preferences.phoneName, devices: saved.devices, listener: listener)
        let phone = try fingerprint(seed: seed)
        let session = Session(
            client: client, fingerprint: phone, saved: saved, alerts: alerts, router: router)
        let push = Push(client: client)
        listener.bind {
            Task { @MainActor [weak session] in session?.refresh() }
        }
        let lock = AppLock(authenticator: DeviceAuthenticator(), paired: session.paired)
        return Stores(
            session: session, router: router, preferences: preferences, lock: lock, alerts: alerts,
            push: push)
    }

    /// A scene for review or a UI test: the demo world, a store that forgets between
    /// launches, and a lock that opens on request.
    static func demo(_ scene: DemoScene, suite: String = "dev.condr.companion.demo") -> Stores {
        guard let defaults = UserDefaults(suiteName: suite) else {
            preconditionFailure("Could not open the demo preferences")
        }
        defaults.removePersistentDomain(forName: suite)
        let preferences = Preferences(defaults: defaults)
        let router = Router()
        // The demo world holds still, its banner and notices too, however slowly a test
        // finds them.
        let alerts = Alerts(bannerLifetime: nil, noticeLifetime: nil)
        let saved = SavedDeviceStore(defaults: defaults)
        let session = Session(
            client: DemoClient(scene: scene), fingerprint: DemoWorld.fingerprint, saved: saved,
            alerts: alerts, router: router)
        let lock = AppLock(
            authenticator: DemoAuthenticator(), paired: scene == .lock, defaults: defaults)
        switch scene {
        case .devices: router.tab = .devices
        case .device:
            router.tab = .devices
            router.devicesPath = [.device(DemoWorld.macmini)]
        case .offline:
            router.tab = .devices
            router.devicesPath = [.device(DemoWorld.studio)]
        case .settings: router.tab = .settings
        case .error:
            alerts.report("Couldn't reach macmini: the connection closed while opening the Pane.")
        case .terminal: router.agentsPath = [.pane(PaneRef(address: DemoWorld.macmini, pane: 1))]
        case .terminalError:
            router.agentsPath = [.pane(PaneRef(address: DemoWorld.macmini, pane: 1))]
            alerts.report(
                "Couldn't send to macmini: the connection closed. Your reply is still in the field."
            )
        case .question: router.agentsPath = [.pane(PaneRef(address: DemoWorld.jpdev, pane: 11))]
        case .codex: router.agentsPath = [.pane(PaneRef(address: DemoWorld.macmini, pane: 2))]
        case .computer: router.agentsPath = [.device(DemoWorld.macmini)]
        case .crowdDevice: router.agentsPath = [.device(DemoWorld.buildbox)]
        case .shell:
            router.tab = .devices
            router.devicesPath = [
                .device(DemoWorld.macmini), .pane(PaneRef(address: DemoWorld.macmini, pane: 3)),
            ]
        case .banner:
            router.tab = .devices
            alerts.raise(
                Banner(
                    ref: PaneRef(address: DemoWorld.jpdev, pane: 12), status: .blocked,
                    kind: "claude",
                    title: "Claude needs you", detail: "Migrate auth to passkeys · jpdev"))
        default: break
        }
        return Stores(
            session: session, router: router, preferences: preferences, lock: lock, alerts: alerts,
            push: nil)
    }
}

/// A demo scene's lock opens on request, without the device's owner.
struct DemoAuthenticator: Authenticator {
    var available: Bool { true }
    var method: String { "Face ID" }
    var promptsOnAppear: Bool { false }
    func authenticate(reason: String) async -> Bool { true }
}

/// The Keychain would not keep the Device key: nothing can be paired without it.
struct StartupFailure: View {
    let message: String

    var body: some View {
        ContentUnavailableView {
            Label("Condr can't start", systemImage: "key.slash")
        } description: {
            Text(message)
        } actions: {
            Button("Copy Error") { UIPasteboard.general.string = message }
        }
    }
}

struct Root: View {
    @Environment(Session.self) private var session
    @Environment(Router.self) private var router
    @Environment(AppLock.self) private var lock
    @Environment(Alerts.self) private var alerts
    @Environment(Push.self) private var push: Push?
    @Environment(\.scenePhase) private var phase
    @State private var curtain = Curtain()
    @State private var network = NetworkMonitor()
    /// The welcome and its pairing, until they are done; again once every Device is forgotten.
    @State private var onboarding: Bool?

    var body: some View {
        @Bindable var router = router
        let onboarding = onboarding ?? !session.paired
        ZStack {
            if onboarding {
                PairingFlow(firstLaunch: true) { self.onboarding = false }
                    .transition(.opacity)
            } else {
                MainTabs()
                    .transition(.opacity.combined(with: .scale(scale: 1.04)))
            }
        }
        .animation(.smooth(duration: 0.6), value: onboarding)
        .onChange(of: session.paired, initial: true) { _, paired in
            // Settled at the first look, so pairing from the welcome does not swap the
            // welcome for the tabs before its last step is shown.
            if self.onboarding == nil || !paired { self.onboarding = !paired }
        }
        .onChange(of: session.devices.map(\.address)) { Task { await registerPush() } }
        .overlay(alignment: .top) { BannerView() }
        .overlay(alignment: .bottom) {
            // Inside a Pane the toast sits over the input bar, from the terminal itself.
            if session.shown == nil {
                VStack(spacing: 8) {
                    NoticeToast()
                    ErrorToast()
                }
                .padding(.bottom, 86)
            }
        }
        // The lock and the shield are drawn in a window of their own, over any sheet or menu.
        .onChange(of: lock.locked, initial: true) { _, _ in raiseCurtain() }
        .sheet(isPresented: $router.pairingAnother) {
            PairingFlow(firstLaunch: false) { router.pairingAnother = false }
        }
        .sensoryFeedback(trigger: alerts.banner) { _, banner in
            switch banner?.status {
            case .blocked?: .warning
            case .error?: .error
            case .done?: .success
            default: nil
            }
        }
        .onChange(of: phase, initial: true) { _, phase in
            switch phase {
            case .active:
                // The app lock (ADR 0041): on start, and after five minutes away.
                lock.sceneBecameActive(paired: session.paired)
                raiseCurtain()
                session.resume()
                session.setActive(true)
                network.start { session.networkChanged() }
                Task { await registerPush() }
            case .inactive:
                session.setActive(false)
                // Leaving the screen shields it at once, so the app switcher's snapshot
                // shows only the mark. Before pairing there is nothing to hide, and a lock
                // already hides it all: swapping it for the shield would rebuild it, and
                // Face ID with it, every time its own prompt makes the app inactive.
                if session.paired, !lock.locked { curtain.show(.shield) { AnyView(Shield()) } }
            case .background:
                lock.sceneLeft()
                if session.paired, !lock.locked { curtain.show(.shield) { AnyView(Shield()) } }
                network.stop()
                suspendInBackground()
            @unknown default:
                break
            }
        }
    }

    /// Asks for notifications once a Device is paired, with the system's prompt only and
    /// never again once answered, then registers with Condr Cloud (ADR 0047).
    private func registerPush() async {
        guard let push else { return }
        let center = UNUserNotificationCenter.current()
        var status = await center.notificationSettings().authorizationStatus
        if status == .notDetermined, session.paired {
            _ = try? await center.requestAuthorization(options: [.alert, .sound])
            status = await center.notificationSettings().authorizationStatus
        }
        if status == .authorized { UIApplication.shared.registerForRemoteNotifications() }
        await push.update(status: status, servers: session.devices.compactMap(\.deviceId))
    }

    private func raiseCurtain() {
        if lock.locked {
            curtain.show(.lock) { AnyView(LockView().environment(lock)) }
        } else {
            curtain.hide()
        }
    }

    /// Closing the connections is covered by a background task, so iOS lets it finish
    /// (ADR 0040).
    private func suspendInBackground() {
        let task = UIApplication.shared.beginBackgroundTask(withName: "condr.suspend")
        Task {
            await session.suspend()
            UIApplication.shared.endBackgroundTask(task)
        }
    }
}

/// The platform's network monitor, feeding the library (ADR 0040). The first report
/// describes the path as it is, not a change, and is skipped.
@MainActor
final class NetworkMonitor {
    private var monitor: NWPathMonitor?

    func start(_ changed: @escaping @MainActor () -> Void) {
        guard monitor == nil else { return }
        let monitor = NWPathMonitor()
        let first = Mutex(true)
        monitor.pathUpdateHandler = { _ in
            if first.withLock({ was in
                defer { was = false }
                return was
            }) {
                return
            }
            Task { @MainActor in changed() }
        }
        monitor.start(queue: .global(qos: .utility))
        self.monitor = monitor
    }

    func stop() {
        monitor?.cancel()
        monitor = nil
    }
}
