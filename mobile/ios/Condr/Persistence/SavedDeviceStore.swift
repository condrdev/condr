import CondrKit
import Foundation

extension SavedDevice: @retroactive Codable {
    private enum CodingKeys: String, CodingKey {
        case address, label
    }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.init(
            address: try container.decode(String.self, forKey: .address),
            label: try container.decode(String.self, forKey: .label))
    }

    public func encode(to encoder: any Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(address, forKey: .address)
        try container.encode(label, forKey: .label)
    }
}

/// The paired Devices as the app keeps them, beside the key (ADR 0040): each an address
/// without its invite and the name the Server goes by, in the app group's defaults.
@MainActor
final class SavedDeviceStore {
    static let appGroup = "group.dev.condr.companion"
    private static let key = "devices"

    private(set) var devices: [SavedDevice]
    private let defaults: UserDefaults

    init(defaults: UserDefaults) {
        self.defaults = defaults
        devices =
            defaults.data(forKey: Self.key)
            .flatMap { try? JSONDecoder().decode([SavedDevice].self, from: $0) } ?? []
    }

    /// The app group's store, or the standard one where no group is provisioned.
    static func shared() -> SavedDeviceStore {
        SavedDeviceStore(defaults: UserDefaults(suiteName: appGroup) ?? .standard)
    }

    func add(_ device: SavedDevice) {
        devices.removeAll { $0.address == device.address }
        devices.append(device)
        save()
    }

    func remove(_ address: String) {
        devices.removeAll { $0.address == address }
        save()
    }

    func rename(_ address: String, to label: String) {
        guard let index = devices.firstIndex(where: { $0.address == address }) else { return }
        devices[index].label = label
        save()
    }

    private func save() {
        defaults.set(try? JSONEncoder().encode(devices), forKey: Self.key)
    }
}
