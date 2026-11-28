package app.isctcompanion.core

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.setValue
import app.isctcompanion.design.Format
import java.time.LocalDate
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch

/** Main-thread publication state. Core owns the dependency versions. A forced
 * read invalidates even an older answer at the same storage version. */
internal class ScheduleReads {
    var generation = 0L
        private set
    private var publishedGeneration = -1L
    private var reads: List<WorkspaceChange> = emptyList()
    private var publishedSeq = 0uL
    var publishedVersion = 0uL
        private set
    private var job: Job? = null

    fun invalidate() { generation++ }
    fun current(session: Session) = publishedGeneration == generation && session.readVersion(reads) <= publishedSeq

    fun accept(snapshot: ScheduleSnapshot, started: Long, session: Session): Boolean {
        if (started != generation || snapshot.seq < publishedSeq) return false
        val version = session.readVersion(snapshot.reads)
        if (version > snapshot.seq) return false
        publishedGeneration = generation
        publishedSeq = snapshot.seq
        publishedVersion = version
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

/** The pager keeps its last answer for each Monday while Core recomputes it. */
internal class TimetableWeeks {
    private val reads = ScheduleReads()
    private val shown = mutableStateMapOf<LocalDate, TimetablePage>()
    private val fresh = mutableSetOf<LocalDate>()
    var revision by mutableLongStateOf(0L)
        private set

    operator fun get(monday: LocalDate): TimetablePage? = shown[monday]

    /** A week on screen reads again when a commit touched what it read, or
     * when its last read did not finish. */
    fun invalidateIfChanged(session: Session) {
        if (shown.isNotEmpty() && (fresh.isEmpty() || !reads.current(session))) invalidate()
    }

    fun invalidate(discard: Boolean = false) {
        reads.invalidate()
        fresh.clear()
        revision++
        if (discard) shown.clear()
    }

    suspend fun week(
        monday: LocalDate,
        session: Session,
        read: suspend (WorkspaceQuery) -> ScheduleSnapshot?,
    ): TimetablePage? {
        if (monday in fresh && reads.current(session)) return shown[monday]

        val generation = reads.generation
        // A cold read fills the swipe window. Refreshes cover the pages visible while swiping.
        val reach: UByte = if (shown[monday] == null) 4u else 1u
        val query = WorkspaceQuery.TimetableWeeks(Format.civil(monday), reach, TimetableMode.CLASSES)

        while (generation == reads.generation) {
            val snapshot = read(query) ?: return null
            currentCoroutineContext().ensureActive()
            val weeks = (snapshot.values.single() as WorkspaceQueryResult.TimetableWeeks).v1
            val previousVersion = reads.publishedVersion
            if (!reads.accept(snapshot, generation, session)) continue

            // A commit may already be visible to Core before its event arrives.
            if (reads.publishedVersion != previousVersion) fresh.clear()
            val loaded = weeks.associateBy { LocalDate.parse(it.monday) }
            shown.putAll(loaded)
            fresh.addAll(loaded.keys)
            return shown[monday]
        }
        return null
    }
}
