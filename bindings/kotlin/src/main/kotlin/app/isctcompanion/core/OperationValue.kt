package app.isctcompanion.core

import kotlinx.coroutines.CancellationException

/** Coroutine cancellation detaches the waiter; Core retains admitted work. */
suspend fun Operation.value(): OperationResult = try {
    awaitResult()
} catch (error: CancellationException) {
    detach()
    throw error
}
