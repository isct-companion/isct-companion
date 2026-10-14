import Foundation

enum ForeignFailure: Error { case injected }
final class Listener: ProbeListener, @unchecked Sendable {
    let fails: Bool
    private let lock = NSLock()
    private var requests: [UInt64] = []
    init(fails: Bool = false) { self.fails = fails }
    func completed(request: UInt64) throws {
        if fails { throw ForeignFailure.injected }
        lock.lock(); defer { lock.unlock() }
        requests.append(request)
    }
    func snapshot() -> [UInt64] { lock.lock(); defer { lock.unlock() }; return requests }
}
final class WeakListener: @unchecked Sendable {
    weak var value: Listener?
    init(_ value: Listener) { self.value = value }
}
@main struct Smoke {
    static func cancelRun(_ probe: Probe) async throws -> WeakListener {
        let listener = Listener()
        let released = WeakListener(listener)
        let pending = Task { try await probe.run(request: 9, delayMs: 1000, listener: listener) }
        try await Task.sleep(for: .milliseconds(20))
        pending.cancel() // Swift Task cancellation alone does not cancel this binding.
        let cancelled = try probe.cancel(request: 9)
        precondition(cancelled)
        do { _ = try await pending.value; fatalError("cancel did not propagate") }
        catch ProbeError.Cancelled { }
        return released
    }
    static func main() async throws {
        let probe = try Probe()
        let listener = Listener()
        let value = try await probe.run(request: 7, delayMs: 5, listener: listener)
        precondition(value == 7 && listener.snapshot() == [7])
        do {
            _ = try await probe.run(request: 8, delayMs: 1, listener: Listener(fails: true))
            fatalError("unexpected callback error was swallowed")
        } catch ProbeError.Callback { }
        let released = try await cancelRun(probe)
        try await Task.sleep(for: .milliseconds(50))
        precondition(released.value == nil, "cancelled task retained foreign listener")
        let count = try probe.pending()
        precondition(count == 0)
        let closing = Task { try await probe.run(request: 12, delayMs: 1000, listener: listener) }
        try await Task.sleep(for: .milliseconds(20))
        try probe.shutdown()
        do { _ = try await closing.value; fatalError("shutdown left a task running") }
        catch ProbeError.Cancelled { }
        precondition(listener.snapshot() == [7])
        try probe.shutdown()
        do {
            _ = try await probe.run(request: 10, delayMs: 1, listener: listener)
            fatalError("run after shutdown succeeded")
        } catch ProbeError.Closed { }
        do { try injectedPanic(); fatalError("panic did not escape as an error") }
        catch { precondition(String(describing: error).contains("FFI smoke-test panic")) }
        print("SWIFT PASS: runtime, correlation, foreign error, cancellation, shutdown, panic")
    }
}
