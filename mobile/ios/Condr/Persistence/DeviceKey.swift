import CondrKit
import Foundation
import Security

/// This phone's Device key seed, kept by the Keychain under the app's shared access group
/// so the push extension can read it later, available after the first unlock and never
/// leaving this device (ADR 0040). The library generates it once; the app hands it back
/// at every start.
enum DeviceKey {
    struct KeychainError: Error, LocalizedError {
        let status: OSStatus
        var errorDescription: String? {
            "The Keychain refused the device key: \(SecCopyErrorMessageString(status, nil) as String? ?? "\(status)")."
        }
    }

    private static let query: [String: Any] = [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: "dev.condr.companion",
        kSecAttrAccount as String: "device-key",
    ]

    /// The stored seed, or a fresh one stored now.
    static func load() throws -> Data {
        if let seed = try read() { return seed }
        let seed = try generateSeed()
        try store(seed)
        return seed
    }

    private static func read() throws -> Data? {
        var query = query
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        switch status {
        case errSecSuccess: return item as? Data
        case errSecItemNotFound: return nil
        default: throw KeychainError(status: status)
        }
    }

    private static func store(_ seed: Data) throws {
        var attributes = query
        attributes[kSecValueData as String] = seed
        attributes[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let status = SecItemAdd(attributes as CFDictionary, nil)
        guard status == errSecSuccess else { throw KeychainError(status: status) }
    }
}
