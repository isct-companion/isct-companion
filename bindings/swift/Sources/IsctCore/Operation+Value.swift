import Foundation

extension Operation {
    /// Swift task cancellation detaches this waiter and throws `CancellationError`.
    /// Detaching never aborts work already admitted by Core, including commits.
    public func value() async throws -> OperationResult {
        try Task.checkCancellation()
        return try await withTaskCancellationHandler {
            do {
                let result = try await self.awaitResult()
                try Task.checkCancellation()
                return result
            } catch {
                if Task.isCancelled {
                    throw CancellationError()
                }
                throw error
            }
        } onCancel: {
            try? self.detach()
        }
    }
}
