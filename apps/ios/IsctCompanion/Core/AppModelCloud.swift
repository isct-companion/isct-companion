import Foundation
import IsctCore

// MARK: - Backup

/// The desktop's Backup settings over the same Core journal: iCloud and Drive on or off,
/// automatic or by-hand sync, earlier versions, local recovery files and a
/// backup file. Core decides what uploads, merges and restores.
extension AppModel {
    func refreshCloud() async {
        guard let session else { return }
        if case let .cloud(state)? = try? await session.cloudState().value() {
            cloud = state
        }
    }

    /// Turning Drive on first shows Google's sign-in; Core keeps it off with
    /// the reason if the sign-in fails.
    func setCloudEnabled(_ provider: CloudProvider, _ enabled: Bool) async {
        await cloudOperation { try $0.setCloudEnabled(provider: provider, enabled: enabled) }
    }

    func setCloudAutomatic(_ automatic: Bool) async {
        await cloudOperation { try $0.setCloudAutomatic(enabled: automatic) }
    }

    func syncCloud(_ direction: Direction) async {
        await cloudOperation { try $0.syncCloud(direction: direction) }
        await refresh()
    }

    /// Restores a cloud version or a local recovery file, on every connected device.
    func restoreCloud(_ id: String) async {
        await cloudOperation { try $0.restoreCloud(id: id) }
        await refresh()
    }

    /// The current workspace as a backup file (the desktop's Export…).
    func exportBackup() async -> String? {
        guard let session else { return nil }
        do {
            guard case let .text(value) = try await session.exportBackup().value() else { return nil }
            return value
        } catch {
            problem = Self.describe(error)
            return nil
        }
    }

    /// Imports a backup file from this or another device; Core saves a recovery
    /// file first and publishes the import to iCloud like any restore.
    func importBackup(_ data: Data) async throws {
        guard let session else { return }
        // Core validates the document and rejects anything that is not a backup.
        _ = try await session.applyBackup(document: String(decoding: data, as: UTF8.self)).value()
        await refreshCloud()
        await refresh()
    }

    private func cloudOperation(_ start: (Session) throws -> IsctCore.Operation) async {
        guard let session else { return }
        do {
            if case let .cloud(state) = try await start(session).value() {
                cloud = state
            }
        } catch {
            problem = Self.describe(error)
            await refreshCloud()
        }
    }
}
