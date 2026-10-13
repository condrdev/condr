import Foundation
import LocalAuthentication
import Observation

/// Who confirms it is the owner: Face ID, a fingerprint or the device passcode.
protocol Authenticator: Sendable {
    /// Whether the device can ask at all; one without a passcode cannot.
    var available: Bool { get }
    /// What it asks for, for the setting's name: "Face ID", "Touch ID" or "Passcode".
    var method: String { get }
    /// Whether the lock asks as soon as it shows, or waits for the button.
    var promptsOnAppear: Bool { get }
    func authenticate(reason: String) async -> Bool
}

struct DeviceAuthenticator: Authenticator {
    var available: Bool {
        LAContext().canEvaluatePolicy(.deviceOwnerAuthentication, error: nil)
    }

    var method: String {
        let context = LAContext()
        _ = context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil)
        return switch context.biometryType {
        case .faceID: "Face ID"
        case .touchID: "Touch ID"
        case .opticID: "Optic ID"
        default: String(localized: "Passcode")
        }
    }

    var promptsOnAppear: Bool { true }

    func authenticate(reason: String) async -> Bool {
        do {
            return try await LAContext().evaluatePolicy(
                .deviceOwnerAuthentication, localizedReason: reason)
        } catch {
            return false
        }
    }
}

/// The app lock (ADR 0041): the owner's face, finger or passcode when the app starts and
/// when it comes back after more than five minutes away. It is on unless the owner turned
/// it off, which asks them first; before anything is paired there is nothing to protect.
@Observable
@MainActor
final class AppLock {
    static let grace: TimeInterval = 300
    private static let enabledKey = "lock.enabled"

    private(set) var locked: Bool
    private(set) var authenticating = false
    /// Whether the lock is on; it is until the owner turns it off.
    private(set) var enabled: Bool
    private var leftAt: Date?
    private let authenticator: any Authenticator
    private let defaults: UserDefaults
    private let now: () -> Date

    init(
        authenticator: any Authenticator, paired: Bool, defaults: UserDefaults = .standard,
        now: @escaping () -> Date = { .now }
    ) {
        self.authenticator = authenticator
        self.defaults = defaults
        self.now = now
        let enabled = defaults.object(forKey: Self.enabledKey) as? Bool ?? true
        self.enabled = enabled
        locked = paired && enabled && authenticator.available
    }

    var available: Bool { authenticator.available }
    var method: String { authenticator.method }
    var promptsOnAppear: Bool { authenticator.promptsOnAppear }

    func lock() {
        guard enabled, authenticator.available else { return }
        locked = true
    }

    /// The app left the screen; the clock on the grace period starts.
    func sceneLeft() {
        if leftAt == nil { leftAt = now() }
    }

    /// The app is back: locked if it was away past the grace period.
    func sceneBecameActive(paired: Bool) {
        if let leftAt, paired, now().timeIntervalSince(leftAt) > Self.grace {
            lock()
        }
        leftAt = nil
    }

    func unlock() async {
        guard locked, !authenticating else { return }
        if await ask(String(localized: "Unlock to reach your computers.")) {
            locked = false
        }
    }

    /// Turns the lock on, or off once the owner confirms it is them: whoever holds the
    /// phone unlocked must not be able to leave it open for the next person.
    func setEnabled(_ on: Bool) async {
        guard on != enabled else { return }
        if !on, !(await ask(String(localized: "Confirm it is you to turn off the app lock."))) {
            return
        }
        enabled = on
        defaults.set(on, forKey: Self.enabledKey)
    }

    private func ask(_ reason: String) async -> Bool {
        guard !authenticating else { return false }
        authenticating = true
        defer { authenticating = false }
        return await authenticator.authenticate(reason: reason)
    }
}
