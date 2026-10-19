import CloudKit
import Foundation
import IsctCore
import UIKit

/// `NativeCloud` over the CloudKit private database. Core names the record
/// (`cloudkitRecord`), so the Mac app reads and writes the same one, and Core
/// parses, merges and keeps history; this only moves the record's bytes and
/// change tag. Called on Core worker threads.
final class CloudKitStore: NativeCloud, @unchecked Sendable {
    private let location = cloudkitRecord()
    private let name: String
    private let database: CKDatabase
    private let container: CKContainer

    @MainActor
    init() {
        name = UIDevice.current.name
        container = CKContainer(identifier: location.container)
        database = container.privateCloudDatabase
    }

    private var recordID: CKRecord.ID { CKRecord.ID(recordName: location.recordName) }

    func deviceName() -> String? { name }

    /// iCloud has no token to forget; the account belongs to the device.
    func disconnect(provider: CloudProvider) throws {}

    func read(provider: CloudProvider) async throws -> CloudObject? {
        try await translated {
            try await requireAccount()
            guard let record = try await fetch() else { return nil }
            guard let asset = record[location.assetKey] as? CKAsset, let url = asset.fileURL else {
                throw SdkError.Network(detail: "iCloud backup record has no payload.")
            }
            return CloudObject(bytes: try Data(contentsOf: url), tag: record.recordChangeTag ?? "")
        }
    }

    func write(provider: CloudProvider, bytes: Data, expected: String?) async throws {
        try await translated {
            try await requireAccount()
            let current = try await fetch()
            // `expected` is nil only when no record existed at the read.
            guard current?.recordChangeTag == expected else {
                throw SdkError.Conflict(detail: "iCloud backup changed since it was read.")
            }
            let record = current ?? CKRecord(recordType: location.recordType, recordID: recordID)
            let file = FileManager.default.temporaryDirectory
                .appendingPathComponent("isct-companion-\(UUID().uuidString).json")
            try bytes.write(to: file, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
            defer { try? FileManager.default.removeItem(at: file) }
            record[location.assetKey] = CKAsset(fileURL: file)
            // `save` keeps CloudKit's if-unchanged policy: a writer that slipped
            // in after the fetch makes it fail with `serverRecordChanged`.
            _ = try await database.save(record)
        }
    }

    private func fetch() async throws -> CKRecord? {
        do {
            return try await database.record(for: recordID)
        } catch let error as CKError where error.code == .unknownItem {
            return nil
        }
    }

    private func requireAccount() async throws {
        switch try await container.accountStatus() {
        case .available:
            return
        case .noAccount:
            throw SdkError.Unavailable(detail: "Sign in to iCloud to use iCloud sync.")
        case .restricted:
            throw SdkError.Unavailable(detail: "iCloud access is restricted on this device.")
        case .temporarilyUnavailable:
            throw SdkError.Unavailable(detail: "iCloud is temporarily unavailable.")
        default:
            throw SdkError.Unavailable(detail: "iCloud account status could not be determined.")
        }
    }

    /// Core only understands `SdkError`; a lost conditional save is a conflict
    /// it retries after a re-read, everything else a transport failure.
    private func translated<T>(_ body: () async throws -> T) async throws -> T {
        do {
            return try await body()
        } catch let error as SdkError {
            throw error
        } catch let error as CKError where error.code == .serverRecordChanged {
            throw SdkError.Conflict(detail: error.localizedDescription)
        } catch {
            throw SdkError.Network(detail: error.localizedDescription)
        }
    }
}
