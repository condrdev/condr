import Foundation
import LocalAuthentication
import Observation

/// Who confirms it is the owner: Face ID, a fingerprint or the device passcode.
protocol Authenticator: Sendable {
    /// Whether the device can ask at all; one without a passcode cannot.
    var available: Bool { get }
    /// Whether the lock asks as soon as it shows, or waits for the button.
    var promptsOnAppear: Bool { get }
    func authenticate() async -> Bool
}

struct DeviceAuthenticator: Authenticator {
    var available: Bool {
        LAContext().canEvaluatePolicy(.deviceOwnerAuthentication, error: nil)
    }

    var promptsOnAppear: Bool { true }

    func authenticate() async -> Bool {
        let context = LAContext()
        context.localizedReason = String(localized: "Unlock to reach your computers.")
        do {
            return try await context.evaluatePolicy(
                .deviceOwnerAuthentication, localizedReason: context.localizedReason)
        } catch {
            return false
        }
    }
}

/// The app lock (ADR 0041): the owner's face, finger or passcode when the app starts and
/// when it comes back after more than five minutes away. Before anything is paired there
/// is nothing to protect.
@Observable
@MainActor
final class AppLock {
    static let grace: TimeInterval = 300

    private(set) var locked: Bool
    private(set) var authenticating = false
    private var leftAt: Date?
    private let authenticator: any Authenticator
    private let now: () -> Date

    init(authenticator: any Authenticator, paired: Bool, now: @escaping () -> Date = { .now }) {
        self.authenticator = authenticator
        self.now = now
        locked = paired && authenticator.available
    }

    var available: Bool { authenticator.available }
    var promptsOnAppear: Bool { authenticator.promptsOnAppear }

    func lock() {
        guard authenticator.available else { return }
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
        authenticating = true
        defer { authenticating = false }
        if await authenticator.authenticate() {
            locked = false
        }
    }
}
