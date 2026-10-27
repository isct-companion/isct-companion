import Foundation
import IsctCore

final class MemoryVault: NativeVault, @unchecked Sendable {
    private let lock = NSCondition()
    private var values: [String: String] = [:]
    private var failStore = false
    private var blockStore = false
    private var storeBlocked = false
    func failNextStore() { lock.withLock { failStore = true } }
    func blockNextStore() { lock.withLock { blockStore = true } }
    func waitUntilStoreBlocked() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        let deadline = Date(timeIntervalSinceNow: 5)
        while !storeBlocked {
            if !lock.wait(until: deadline) { return false }
        }
        return true
    }
    func releaseBlockedStore() {
        lock.withLock {
            blockStore = false
            lock.broadcast()
        }
    }
    func load(account: String) throws -> String? { lock.withLock { values[account] } }
    func store(account: String, secret: String) throws {
        lock.lock()
        defer { lock.unlock() }
        if blockStore {
            storeBlocked = true
            lock.broadcast()
            while blockStore { lock.wait() }
            storeBlocked = false
        }
        if failStore {
            failStore = false
            throw NSError(domain: "native-vault-fixture", code: 1)
        } else {
            values[account] = secret
        }
    }
    func delete(account: String) throws { _ = lock.withLock { values.removeValue(forKey: account) } }
}
final class WaitingOperation: IsctCore.Operation, @unchecked Sendable {
    private let lock = NSCondition()
    private var waiting = false
    private var detached = false
    private var continuation: CheckedContinuation<OperationResult, any Error>?
    init() { super.init(noHandle: .init()) }
    required init(unsafeFromHandle handle: UInt64) { fatalError("unused test initializer") }
    override func awaitResult() async throws -> OperationResult {
        try await withCheckedThrowingContinuation { continuation in
            lock.withLock {
                waiting = true
                self.continuation = continuation
                lock.broadcast()
            }
        }
    }
    override func detach() throws {
        let continuation = lock.withLock {
            detached = true
            let continuation = self.continuation
            self.continuation = nil
            return continuation
        }
        continuation?.resume(throwing: SdkError.Cancelled)
    }
    func waitUntilWaiting() async -> Bool {
        await Task.detached { [self] in waitUntilWaitingBlocking() }.value
    }
    private func waitUntilWaitingBlocking() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        let deadline = Date(timeIntervalSinceNow: 5)
        while !waiting {
            if !lock.wait(until: deadline) { return false }
        }
        return true
    }
    func wasDetached() -> Bool { lock.withLock { detached } }
}
func require(_ condition: Bool, _ detail: String) throws {
    if !condition { throw NSError(domain: "SdkSmoke", code: 1, userInfo: [NSLocalizedDescriptionKey: detail]) }
}
@main struct Smoke {
    static func main() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("isct-swift-\(UUID().uuidString)")
        let config = SessionConfig(dataDirectory: directory.appendingPathComponent("data").path, filesDirectory: directory.appendingPathComponent("files").path, initialNetwork: .offline, cloudProviders: [], authorizedCloudProviders: [])
        let vault = MemoryVault()
        let session = try await Session.open(config: config, vault: vault, browser: nil, cloud: nil, notifications: nil, downloads: nil, webAuthentication: nil, driveTokens: nil)
        let logs = try exportApplicationLogs(about: "smoke")
        try require(logs.hasPrefix("smoke\n") && logs.contains("Application started"), "Opening a session must install Core's logger")
        let waitingOperation = WaitingOperation()
        let cancelledWait = Task { try await waitingOperation.value() }
        try require(await waitingOperation.waitUntilWaiting(), "Swift value() did not enter its native wait")
        cancelledWait.cancel()
        do {
            _ = try await cancelledWait.value
            throw NSError(domain: "expected-swift-cancellation", code: 1)
        } catch is CancellationError { }
        try require(waitingOperation.wasDetached(), "Swift cancellation did not detach the active waiter")

        let credentials = SchoolCredentials(username: "fixture", password: "not-a-real-password", otpSecret: "", matrix: String(repeating: "A", count: 70))
        vault.blockNextStore()
        let cancelledOperation = try session.connectSchool(profile: .tokyoTech, credentials: credentials)
        try require(vault.waitUntilStoreBlocked(), "Credential save did not reach the vault")
        try cancelledOperation.detach()
        do {
            _ = try await cancelledOperation.awaitResult()
            throw NSError(domain: "expected-detached-operation", code: 1)
        } catch SdkError.Cancelled { }
        guard case .pending = try cancelledOperation.status() else {
            throw NSError(domain: "swift-cancellation-aborted-operation", code: 1)
        }
        vault.releaseBlockedStore()
        let completionClock = ContinuousClock()
        let completionDeadline = completionClock.now.advanced(by: .seconds(5))
        operationCompletion: while true {
            switch try cancelledOperation.status() {
            case .pending:
                try require(completionClock.now < completionDeadline, "Detached credential save did not settle")
                await Task.yield()
            case .completed:
                throw NSError(domain: "cancelled-operation-result", code: 1)
            // No school browser here: connecting keeps the credentials, then cannot sign in.
            case .failed(SdkError.Unavailable):
                guard case let .credentials(status) = try await session.credentialStatus().value() else {
                    throw NSError(domain: "credential-status", code: 1)
                }
                try require(status.tokyoTech.saved, "Detached credential save did not complete")
                break operationCompletion
            case let .failed(error):
                throw error
            }
        }
        if CommandLine.arguments.contains("--require-ocr") {
            try require(try session.capabilities().ocr, "Packaged iOS SDK must expose OCR")
        }
        try require(try !schoolBrowserInitialization().isEmpty, "Missing document-start scripts")
        let updatedCredentials = SchoolCredentials(username: "fixture", password: "changed-password", otpSecret: "", matrix: String(repeating: "A", count: 70))
        vault.failNextStore()
        do {
            _ = try await session.connectSchool(profile: .tokyoTech, credentials: updatedCredentials).value()
            throw NSError(domain: "missing-callback-error", code: 1)
        } catch SdkError.Callback { }
        do {
            _ = try await session.connectSchool(profile: .tokyoTech, credentials: updatedCredentials).value()
            throw NSError(domain: "missing-browser-error", code: 1)
        } catch SdkError.Unavailable { }
        _ = try await session.deleteCredentials(profile: .tokyoTech).value()
        let stream = try session.subscribe()
        guard case let .text(first) = try await session.workspaceRevision().value() else { throw NSError(domain:"revision",code:1) }
        let write = try session.mutateCommit(mutation: WorkspaceMutation(expectedRevision: first, intent: .setAdmissionYear(admissionYear: 2026)))
        try require(try !write.cancel(), "Transactions must not be aborted")
        guard case let .committed(committed) = try await write.value() else { throw NSError(domain:"mutation",code:1) }
        try require(committed.revision != first, "Mutation did not commit")
        do {
            _ = try await session.mutateCommit(mutation: WorkspaceMutation(expectedRevision:first,intent:.setAdmissionYear(admissionYear:2025))).value()
            throw NSError(domain:"missing-conflict",code:1)
        } catch SdkError.Conflict { }
        guard case .event = try await stream.next() else { throw NSError(domain:"missing-event",code:1) }
        let queries: [WorkspaceQuery] = [.glance(now:"2026-10-15T09:00:00+09:00"), .notificationPlan(now:"2026-10-15T09:00:00+09:00",horizon:"2026-10-16T09:00:00+09:00",locale:nil)]
        guard case let .schedule(result) = try await session.scheduleQuery(queries:queries).value() else { throw NSError(domain:"query",code:1) }
        try require(result.seq >= committed.seq && result.values.count == 2, "Query contract differs")
        guard case let .schedule(clockResult) = try await session.scheduleQuery(queries:[.teachingPeriods]).value(),
              case let .some(.teachingPeriods(periods)) = clockResult.values.first else { throw NSError(domain:"teaching-periods",code:1) }
        try require(periods.count == 6 && periods[5].periods == Data([11,12]) && periods[5].startTime == "19:05" && periods[5].endTime == "20:45", "Native teaching clock differs")
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
        do { _ = try session.workspaceRevision(); throw NSError(domain:"expected-closed",code:1) } catch SdkError.Closed { }
        try FileManager.default.removeItem(at:directory)
        print("SWIFT_CORE_SDK_SMOKE_PASSED")
    }
}
