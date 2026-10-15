import Foundation
import IsctCore

final class MemoryVault: NativeVault, @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String: String] = [:]
    private var failStore = false
    func failNextStore() { lock.withLock { failStore = true } }
    func load(account: String) throws -> String? { lock.withLock { values[account] } }
    func store(account: String, secret: String) throws {
        try lock.withLock {
            if failStore { failStore = false; throw NSError(domain: "native-vault-fixture", code: 1) }
            values[account] = secret
        }
    }
    func delete(account: String) throws { _ = lock.withLock { values.removeValue(forKey: account) } }
}
func require(_ condition: Bool, _ detail: String) throws {
    if !condition { throw NSError(domain: "SdkSmoke", code: 1, userInfo: [NSLocalizedDescriptionKey: detail]) }
}
@main struct Smoke {
    static func main() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("isct-swift-\(UUID().uuidString)")
        let config = SessionConfig(dataDirectory: directory.appendingPathComponent("data").path, filesDirectory: directory.appendingPathComponent("files").path, initialNetwork: .offline, cloudProviders: [], authorizedCloudProviders: [])
        let vault = MemoryVault()
        let session = try await Session.open(config: config, vault: vault, browser: nil, cloud: nil, notifications: nil, downloads: nil)
        let cancelledOperation = try session.snapshot()
        let cancelledWait = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            return try await cancelledOperation.value()
        }
        do {
            _ = try await cancelledWait.value
            throw NSError(domain: "expected-swift-cancellation", code: 1)
        } catch is CancellationError { }
        if CommandLine.arguments.contains("--require-ocr") {
            try require(try session.capabilities().ocr, "Packaged iOS SDK must expose OCR")
        }
        try require(try !schoolBrowserInitialization().isEmpty, "Missing document-start scripts")
        let credentials = SchoolCredentials(username: "fixture", password: "not-a-real-password", otpSecret: "", matrix: String(repeating: "A", count: 70))
        vault.failNextStore()
        do {
            _ = try await session.saveCredentials(profile: .tokyoTech, credentials: credentials).value()
            throw NSError(domain: "missing-callback-error", code: 1)
        } catch SdkError.Callback { }
        _ = try await session.saveCredentials(profile: .tokyoTech, credentials: credentials).value()
        _ = try await session.deleteCredentials(profile: .tokyoTech).value()
        let stream = try session.subscribe()
        guard case let .workspace(first) = try await session.snapshot().value() else { throw NSError(domain:"snapshot",code:1) }
        let write = try session.mutate(mutation: WorkspaceMutation(expectedRevision: first.revision, intent: .setAdmissionYear(admissionYear: 2026)))
        try require(try !write.cancel(), "Transactions must not be aborted")
        guard case let .workspace(committed) = try await write.value() else { throw NSError(domain:"mutation",code:1) }
        try require(committed.revision != first.revision, "Mutation did not commit")
        do {
            _ = try await session.mutate(mutation: WorkspaceMutation(expectedRevision:first.revision,intent:.setAdmissionYear(admissionYear:2025))).value()
            throw NSError(domain:"missing-conflict",code:1)
        } catch SdkError.Conflict { }
        guard case .event = try await stream.next() else { throw NSError(domain:"missing-event",code:1) }
        let queries: [WorkspaceQuery] = [.glance(now:"2026-10-15T09:00:00+09:00"), .notificationPlan(now:"2026-10-15T09:00:00+09:00",horizon:"2026-10-16T09:00:00+09:00",locale:nil)]
        guard case let .schedule(result) = try await session.scheduleQuery(queries:queries).value() else { throw NSError(domain:"query",code:1) }
        try require(result.revision == committed.contentRevision && result.values.count == 2, "Snapshot contract differs")
        _ = try await session.parseLmsResource(html:"<html></html>",responseUrl:"https://lms.s.isct.ac.jp/2025/mod/resource/view.php?id=1",module:"resource").value()
        _ = try await session.exportBackup().value()
        if try session.capabilities().ocr {
            _ = try await session.prepareScanner().value()
            do { _ = try await session.scanMatrix(image:Data([1,2,3])).value(); throw NSError(domain:"expected-image-error",code:1) }
            catch SdkError.InvalidInput { }
        } else {
            do { _ = try await session.scanMatrix(image:Data()).value(); throw NSError(domain:"expected-unavailable",code:1) }
            catch SdkError.Unavailable { }
        }
        _ = try await session.suspend().value()
        _ = try await session.resume().value()
        try stream.unsubscribe()
        do { _ = try await stream.next(); throw NSError(domain:"expected-closed-stream",code:1) } catch SdkError.Closed { }
        try await session.shutdown()
        try await session.shutdown()
        do { _ = try session.snapshot(); throw NSError(domain:"expected-closed",code:1) } catch SdkError.Closed { }
        try FileManager.default.removeItem(at:directory)
        print("SWIFT_CORE_SDK_SMOKE_PASSED")
    }
}
