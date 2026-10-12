import Foundation
import Observation

/// What the person set and the app keeps: the terminal's text size and this phone's name.
@Observable
@MainActor
final class Preferences {
    static let smallestFont = 8.0
    static let largestFont = 20.0

    var fontSize: Double {
        didSet { defaults.set(fontSize, forKey: Keys.fontSize) }
    }
    /// The name the phone pairs under; a Server lists it by it (ADR 0040).
    var phoneName: String {
        didSet { defaults.set(phoneName, forKey: Keys.phoneName) }
    }
    /// Whether the phone has a name of its own yet: only the first pairing asks for one.
    var named: Bool { defaults.string(forKey: Keys.phoneName) != nil }

    private let defaults: UserDefaults

    private enum Keys {
        static let fontSize = "terminal.fontSize"
        static let phoneName = "phone.name"
    }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        fontSize = defaults.object(forKey: Keys.fontSize) as? Double ?? 12.5
        phoneName = defaults.string(forKey: Keys.phoneName) ?? "iPhone"
    }
}
