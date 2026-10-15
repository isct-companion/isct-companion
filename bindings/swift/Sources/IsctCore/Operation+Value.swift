import Foundation

extension Operation {
    /// Swift task cancellation detaches this waiter. It never aborts a Core commit.
    public func value() async throws -> OperationResult {
        try await withTaskCancellationHandler {
            try await self.awaitResult()
        } onCancel: {
            try? self.detach()
        }
    }
}
