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
    private let reportFailure: @MainActor (Error) -> Void

    init(
        session: @escaping @MainActor () -> Session?,
        refreshWorkspace: @escaping @MainActor () async -> Void,
        reportFailure: @escaping @MainActor (Error) -> Void
    ) {
        self.session = session
        self.refreshWorkspace = refreshWorkspace
        self.reportFailure = reportFailure
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
    func restore(_ id: String) async {
        await cloudOperation { try $0.restoreCloud(id: id) }
        await refreshWorkspace()
    }

    /// The current workspace as a backup file (the desktop's Export…).
    func export() async -> String? {
        guard let session = session() else { return nil }
        do {
            guard case let .text(value) = try await session.exportBackup().value() else { return nil }
            return value
        } catch {
            reportFailure(error)
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
        do {
            if case let .cloud(snapshot) = try await start(session).value() {
                state = snapshot
            }
        } catch {
            reportFailure(error)
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
