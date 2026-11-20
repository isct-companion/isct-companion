package app.isctcompanion.core

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/** Main-thread publication state. Core owns the dependency versions. A forced
 * read invalidates even an older answer at the same storage version. */
internal class ScheduleReads {
    var generation = 0L
        private set
    private var publishedGeneration = -1L
    private var reads: List<WorkspaceChange> = emptyList()
    private var publishedSeq = 0uL
    private var job: Job? = null

    fun invalidate() { generation++ }
    fun current(session: Session) = publishedGeneration == generation && session.readVersion(reads) <= publishedSeq

    fun accept(snapshot: ScheduleSnapshot, started: Long, session: Session): Boolean {
        if (started != generation || snapshot.seq < publishedSeq || session.readVersion(snapshot.reads) > snapshot.seq) return false
        publishedGeneration = generation
        publishedSeq = snapshot.seq
        reads = snapshot.reads
        return true
    }

    fun refresh(scope: CoroutineScope, session: Session, read: suspend (Long) -> Unit, failed: (Exception) -> Unit): Job {
        job?.let { return it }
        val next = scope.launch(start = CoroutineStart.LAZY) {
            try {
                while (!current(session)) read(generation)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                failed(e)
            } finally {
                job = null
            }
        }
        job = next
        next.start()
        return next
    }
}
