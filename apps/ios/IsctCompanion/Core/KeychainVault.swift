import Foundation
import IsctCore
import Security

/// `NativeVault` over the iOS Keychain. Core names the item (`vaultItem`), so a
/// school login synchronized through iCloud Keychain from the Mac app is the
/// same item here. Called on Core worker threads, never the main thread.
final class KeychainVault: NativeVault, @unchecked Sendable {
    func load(account: String) throws -> String? {
        var query = try baseQuery(account)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        switch status {
        case errSecSuccess:
            guard let data = result as? Data, let text = String(data: data, encoding: .utf8) else {
                throw SdkError.Unavailable(detail: "Keychain item is not text")
            }
            return text
        case errSecItemNotFound:
            return nil
        default:
            throw SdkError.Unavailable(detail: "Keychain read failed (OSStatus \(status))")
        }
    }

    func store(account: String, secret: String) throws {
        let query = try baseQuery(account)
        let data = Data(secret.utf8)
        let update = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if update == errSecSuccess { return }
        guard update == errSecItemNotFound else {
            throw SdkError.Unavailable(detail: "Keychain write failed (OSStatus \(update))")
        }
        var insert = query
        insert[kSecValueData as String] = data
        insert[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        let status = SecItemAdd(insert as CFDictionary, nil)
        guard status == errSecSuccess else {
            throw SdkError.Unavailable(detail: "Keychain write failed (OSStatus \(status))")
        }
    }

    func delete(account: String) throws {
        let status = SecItemDelete(try baseQuery(account) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw SdkError.Unavailable(detail: "Keychain delete failed (OSStatus \(status))")
        }
    }

    private func baseQuery(_ account: String) throws -> [String: Any] {
        let item = try vaultItem(account: account)
        return [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: item.service,
            kSecAttrAccount as String: item.key,
            kSecAttrSynchronizable as String: item.synchronized ? kCFBooleanTrue! : kCFBooleanFalse!,
        ]
    }
}
