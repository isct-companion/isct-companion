import Foundation
import IsctCore
import Observation

/// The cloud journal and the backup operations that update it.
@MainActor
@Observable
final class CloudBackup {
    private(set) var state: SyncState?
    private let session: @MainActor () -> Session?
    private let refreshWorkspace: @MainActor () async -> Void
    private(set) var failure: String?

    init(
        session: @escaping @MainActor () -> Session?,
        refreshWorkspace: @escaping @MainActor () async -> Void
    ) {
        self.session = session
        self.refreshWorkspace = refreshWorkspace
    }

    func refresh() async {
        guard let session = session() else { return }
        if case let .cloud(snapshot)? = try? await session.cloudState().value() {
            state = snapshot
        }
    }

    /// Turning Drive on first shows Google's sign-in; Core keeps it off with
    /// the reason if the sign-in fails.
    func setEnabled(_ provider: CloudProvider, _ enabled: Bool) async {
        await cloudOperation { try $0.setCloudEnabled(provider: provider, enabled: enabled) }
    }

    func setAutomatic(_ automatic: Bool) async {
        await cloudOperation { try $0.setCloudAutomatic(enabled: automatic) }
    }

    func sync(_ direction: Direction) async {
        await cloudOperation { try $0.syncCloud(direction: direction) }
        await refreshWorkspace()
    }

    /// Restores a cloud version or a local recovery file, on every connected device.
    func restore(_ id: String) async throws {
        guard let session = session() else { throw SdkError.Closed }
        _ = try await session.restoreCloud(id: id).value()
        await refresh()
        await refreshWorkspace()
    }

    /// The current workspace as a backup file (the desktop's Export…).
    func export() async -> String? {
        guard let session = session() else { return nil }
        failure = nil
        do {
            guard case let .text(value) = try await session.exportBackup().value() else { return nil }
            return value
        } catch {
            failure = AppModel.describe(error)
            return nil
        }
    }

    /// Imports a backup file from this or another device; Core saves a recovery
    /// file first and publishes the import to iCloud like any restore.
    func importBackup(_ data: Data) async throws {
        guard let session = session() else { return }
        // Core validates the document and rejects anything that is not a backup.
        _ = try await session.applyBackup(document: String(decoding: data, as: UTF8.self)).value()
        await refresh()
        await refreshWorkspace()
    }

    private func cloudOperation(_ start: (Session) throws -> IsctCore.Operation) async {
        guard let session = session() else { return }
        failure = nil
        do {
            if case let .cloud(snapshot) = try await start(session).value() {
                state = snapshot
            }
        } catch {
            failure = AppModel.describe(error)
            await refresh()
        }
    }

    /// After a full reset, as the desktop's: iCloud on again, then its copy pulled.
    func restoreAfterReset() async throws {
        guard let session = session() else { return }
        _ = try await session.setCloudEnabled(provider: .icloud, enabled: true).value()
        _ = try await session.syncCloud(direction: .pull).value()
        await refresh()
        await refreshWorkspace()
    }
}
