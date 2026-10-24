package app.isctcompanion.core

import app.isctcompanion.design.t
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.os.PowerManager
import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ProcessLifecycleOwner
import app.isctcompanion.design.Format
import app.isctcompanion.design.Localizer
import app.isctcompanion.features.timetable.clockMinutes
import app.isctcompanion.features.timetable.thisWeek
import java.io.File
import java.time.Instant
import java.time.LocalDate
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.json.JSONObject

/** The shell's one owner of the Core session. It renders Core read models,
 * forwards intents and reports lifecycle facts; every decision about what is
 * due, grouped or shown stays in Core. Lives as long as the process, so state
 * survives activity recreation. Every member is used on the main thread. */
class AppModel(private val context: Context, private val scope: CoroutineScope) {
    sealed interface Status {
        data object Opening : Status
        data object Ready : Status
        data class Failed(val message: String) : Status
    }

    var status: Status by mutableStateOf(Status.Opening)
        private set
    var today: TodayView? by mutableStateOf(null)
        private set
    var board: AssignmentBoard? by mutableStateOf(null)
    /** Files waiting in each assignment's `submit/`, by assignment id, for
     * 今日's rows (Core's `waitingHandIns`). */
    var waiting: Map<String, Int> by mutableStateOf(emptyMap())
        private set
        private set
    var periods: List<TeachingPeriod> by mutableStateOf(emptyList())
        private set
    /** Course id → Core's display colour, identical on every surface. */
    var colors: Map<String, String> by mutableStateOf(emptyMap())
        private set
    /** Core has reminders for this device but it may not post them. */
    var notificationsBlocked: Boolean by mutableStateOf(false)
        private set
    private var shownRecords by mutableStateOf(WorkspaceRecords())
    var records: WorkspaceRecords
        get() = shownRecords
        private set(value) {
            shownRecords = value
            Localizer.use(value.settings.optString("locale"))
        }
    var credentials: SchoolCredentialStatus? by mutableStateOf(null)
        private set
    var sessions: SchoolSessionStates? by mutableStateOf(null)
        private set
    /** Core's sync planner: its runs and sources, rendered as they are. */
    var syncSnapshot: SyncSnapshot? by mutableStateOf(null)
        private set

    /** One source in Core's sync snapshot (`lms`, `kyomu`, `references`, `catalog`). */
    fun syncSource(source: String): SourceView? = syncSnapshot?.sources?.firstOrNull { it.source == source }

    /** Undo for the last hide, built from the intent Core committed. */
    var undo: UndoAction? by mutableStateOf(null)
        private set
    /** One "needs you" banner (sync feedback level 3). */
    var problem: String? by mutableStateOf(null)
        private set
    /** Cells read so far by a running Matrix scan, from Core's `ocr-progress`. */
    var ocrProgress: Int by mutableStateOf(0)
        private set
    /** Core's cloud journal: providers, unfinished tasks, versions, recovery files. */
    var cloud: SyncState? by mutableStateOf(null)
        private set
    /** Bumped whenever Core reports new content, so open screens re-query. */
    var contentRevision: String by mutableStateOf("")
        private set

    val browser = SchoolBrowserHost(context)
    /** Google Drive sign-in; the activity in front attaches its consent screen. */
    val drive = DriveTokens(context)
    val web = WebRouter(browser, scope, context)
    private var session: Session? = null
    private var revision = ""
    private var boundaryJob: Job? = null
    private var refreshJob: Job? = null
    private var wakeJob: Job? = null
    private val settingsWrite = Mutex()
    private val opening = Mutex()
    private var networkState = SyncNetwork.UNKNOWN
    private var foreground = false

    /** First launch: no school account configured yet. */
    val needsAccount: Boolean
        get() = credentials?.let {
            it.science.availability == CredentialAvailability.NOT_CONFIGURED &&
                it.tokyoTech.availability == CredentialAvailability.NOT_CONFIGURED
        } ?: false

    // Lifecycle

    /** Opens the session once; the application and a background wake both
     * call this, and the later caller waits for the first. */
    suspend fun start() = opening.withLock {
        if (session != null) return@withLock
        try {
            val data = File(context.filesDir, "core").apply { mkdirs() }
            val files = File(context.filesDir, "files").apply { mkdirs() }
            // Google Drive is Android's provider; Core reads whether it was
            // connected and owns every Drive request.
            val config = SessionConfig(
                dataDirectory = data.absolutePath, filesDirectory = files.absolutePath,
                initialNetwork = SyncNetwork.UNKNOWN, cloudProviders = listOf(CloudProvider.GOOGLE_DRIVE),
                authorizedCloudProviders = emptyList(),
            )
            val session = Session.open(
                config, KeystoreVault(context), browser, null, null, WebCookieSession(), null, drive,
            )
            this.session = session
            web.session = session
            val stream = session.subscribe()
            scope.launch { listen(stream) }
            refresh()
            refreshAccounts()
            readSync()
            refreshCloud()
            status = Status.Ready
            watchNetwork()
            watchPowerSaver()
            // WorkManager can start the process with no activity in front.
            foreground = ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)
            hint(if (foreground) Hint.Foreground else Hint.Background)
            if (foreground) {
                startWaking()
                // The launch check signs in through the school web views,
                // which only run in front; out of front the first `FOCUS`
                // does it instead.
                reportSchool(SchoolSessionLifecycleKind.LAUNCH)
            } else {
                runCatching { session.suspend() }
            }
        } catch (error: Exception) {
            status = Status.Failed(describe(error))
        }
    }

    fun foreground(active: Boolean) {
        foreground = active
        val session = session ?: return
        if (active) {
            runCatching { session.resume() }
            hint(Hint.Foreground)
            reportSchool(SchoolSessionLifecycleKind.FOCUS)
            startWaking()
            scope.launch { refresh() }
            // Permission granted in the system settings while away.
            if (notificationsBlocked) scope.launch { resyncNotifications() }
        } else {
            wakeJob?.cancel()
            wakeJob = null
            hint(Hint.Background)
            reportSchool(SchoolSessionLifecycleKind.FOCUS)
            runCatching { session.suspend() }
        }
    }

    /** One WorkManager wake out of front: Core reads what it may there
     * (LMS deadlines and assignment states over the token, reference files)
     * and runs reminders, for at most two minutes, then suspends again unless
     * the app came to the front. A stopped worker ends it early. */
    suspend fun backgroundWake() {
        start()
        val session = session ?: return
        val operation = runCatching { session.backgroundWake(120u) }.getOrNull() ?: return
        try {
            operation.awaitResult()
        } catch (error: CancellationException) {
            runCatching { operation.cancel() }
            throw error
        } catch (error: Exception) {
            Log.w("isct", "background wake", error)
        }
        refresh()
    }

    /** A lifecycle fact, opened view or refresh request for Core's sync
     * planner, which decides what is due. Never waits. */
    fun hint(hint: Hint) {
        runCatching { session?.syncHint(hint) }
    }

    /** The view now on screen, so Core reads what it shows first. */
    fun open(view: SyncView) = hint(Hint.Open(view))

    fun refresh(target: SyncScope) = hint(Hint.Refresh(target))

    /** Core's periodic wake while the app is in front (the planner's `Wake`
     * hint, then reminders and scheduled tasks), about once a second. */
    private fun startWaking() {
        if (wakeJob?.isActive == true) return
        wakeJob = scope.launch {
            while (true) {
                val session = session ?: return@launch
                runCatching { session.wake().value() }
                delay(1_000)
            }
        }
    }

    /** School sign-in hears what desktop sends (`school-session.ts`): `launch`
     * once when the session opens, then `focus` as the app enters or leaves the
     * front. Core only acts on a visible focus. */
    private fun reportSchool(kind: SchoolSessionLifecycleKind) {
        val session = session ?: return
        runCatching { session.schoolLifecycle(SchoolSessionLifecycle(kind, foreground)) }
    }

    private fun watchNetwork() {
        val manager = context.getSystemService(ConnectivityManager::class.java) ?: return
        val update = { state: SyncNetwork ->
            scope.launch {
                if (networkState != state) {
                    networkState = state
                    hint(Hint.Network(state))
                }
            }
        }
        manager.registerDefaultNetworkCallback(object : ConnectivityManager.NetworkCallback() {
            override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) {
                update(
                    when {
                        capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> SyncNetwork.WIFI
                        capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> SyncNetwork.CELLULAR
                        else -> SyncNetwork.UNKNOWN
                    },
                )
            }

            override fun onLost(network: Network) {
                update(SyncNetwork.OFFLINE)
            }
        })
    }

    /** Battery saver is a fact Core's planner weighs; the shell only reports it. */
    private fun watchPowerSaver() {
        val power = context.getSystemService(PowerManager::class.java) ?: return
        hint(Hint.PowerSaver(power.isPowerSaveMode))
        // A system broadcast still reaches a non-exported receiver.
        context.registerReceiver(
            object : BroadcastReceiver() {
                override fun onReceive(context: Context, intent: Intent) {
                    hint(Hint.PowerSaver(power.isPowerSaveMode))
                }
            },
            IntentFilter(PowerManager.ACTION_POWER_SAVE_MODE_CHANGED),
            Context.RECEIVER_NOT_EXPORTED,
        )
    }

    private suspend fun listen(stream: Subscription) {
        while (true) {
            val update = try {
                stream.next()
            } catch (error: Exception) {
                return
            }
            when (update) {
                is StreamUpdate.Event -> when (update.event.name) {
                    "sync-snapshot" -> runCatching { decodeSyncSnapshot(update.event.payload) }
                        .onSuccess(::apply)
                    "school-session", "school-login-status" -> refreshAccounts()
                    "school-web-open" -> web.handleOpenEvent(update.event.payload)
                    "cloud-sync-state" -> refreshCloud()
                    "ocr-progress" -> runCatching { JSONObject(update.event.payload).getInt("completed") }
                        .onSuccess { ocrProgress = it }
                    // No screen here lists finder results; the catalog
                    // changes once per department while it downloads.
                    "school-web-url", "school-download", "catalog-changed" -> Unit
                    // A key re-read with nothing new renews only its freshness,
                    // which no screen here shows.
                    "workspace-changed" -> if (!onlyFreshness(update.event.payload)) scheduleRefresh()
                    else -> scheduleRefresh()
                }
                is StreamUpdate.SnapshotRequired -> {
                    scheduleRefresh()
                    refreshAccounts()
                    readSync()
                }
            }
        }
    }

    /** Whether a `workspace-changed` names sync freshness and nothing else.
     * An unreadable payload counts as a real change. */
    private fun onlyFreshness(payload: String): Boolean = runCatching {
        val changes = JSONObject(payload).getJSONArray("changes")
        changes.length() > 0 && (0 until changes.length()).all { changes.getString(it) == "freshness" }
    }.getOrDefault(false)

    /** Coalesces bursts of change hints into one read. */
    private fun scheduleRefresh() {
        refreshJob?.cancel()
        refreshJob = scope.launch {
            delay(150)
            refresh()
        }
    }

    // Read models

    suspend fun refresh() {
        val session = session ?: return
        val now = Format.timestamp(Instant.now())
        try {
            val snapshot = (session.snapshot().value() as? OperationResult.Workspace)?.snapshot ?: return
            revision = snapshot.revision
            if (snapshot.contentRevision != contentRevision) {
                records = WorkspaceRecords(snapshot.workspace)
            }
            val result = (
                session.scheduleQuery(
                    listOf(WorkspaceQuery.Today(now), WorkspaceQuery.AssignmentBoard(now, null), WorkspaceQuery.TeachingPeriods),
                ).value() as? OperationResult.Schedule
                )?.snapshot ?: return
            for (value in result.values) {
                when (value) {
                    is WorkspaceQueryResult.Today -> today = value.v1
                    is WorkspaceQueryResult.AssignmentBoard -> {
                        board = value.v1
                        refreshWaiting()
                    }
                    is WorkspaceQueryResult.TeachingPeriods -> periods = value.v1
                    else -> Unit
                }
            }
            // A fresh install has no current term yet, so the overview may fail alone.
            runCatching {
                val overview = session.scheduleQuery(listOf(WorkspaceQuery.WorkspaceOverview(""))).value()
                val value = (overview as? OperationResult.Schedule)?.snapshot?.values?.firstOrNull()
                if (value is WorkspaceQueryResult.WorkspaceOverview) colors = value.v1.colors
            }
            val changed = contentRevision != snapshot.contentRevision
            contentRevision = snapshot.contentRevision
            scheduleBoundary()
            if (changed || weeks.isEmpty()) {
                weeks.clear()
                scope.launch { prefetchTimetable() }
            }
            if (changed) notificationsBlocked = NotificationScheduler.sync(context, session)
        } catch (error: CancellationException) {
            // A newer change hint replaced this read; it is not a failure.
            throw error
        } catch (error: SdkException.Cancelled) {
        } catch (error: Exception) {
            problem = describe(error)
        }
    }

    /** Re-query when Core says the phase or hero changes next. */
    private fun scheduleBoundary() {
        boundaryJob?.cancel()
        val at = Format.date(today?.nextBoundary) ?: return
        val delayMs = maxOf(1_000L, at.toEpochMilli() - System.currentTimeMillis() + 1_000L)
        boundaryJob = scope.launch {
            delay(delayMs)
            refresh()
        }
    }

    suspend fun refreshAccounts() {
        val session = session ?: return
        runCatching { (session.credentialStatus().value() as? OperationResult.Credentials)?.status }
            .getOrNull()?.let { credentials = it }
        runCatching { (session.schoolSession().value() as? OperationResult.SchoolSession)?.snapshot }
            .getOrNull()?.let { sessions = it.states }
    }

    /** On start and after a missed stream; `sync-snapshot` events carry every
     * later one. A read that returns after a newer event does not replace it. */
    private suspend fun readSync() {
        val session = session ?: return
        val snapshot = runCatching { (session.syncSnapshot().value() as? OperationResult.Sync)?.snapshot }
            .getOrNull() ?: return
        apply(snapshot)
    }

    /** A read that started before a newer event must not replace it. */
    private fun apply(snapshot: SyncSnapshot) {
        val shown = syncSnapshot
        if (shown == null || snapshot.generation >= shown.generation) syncSnapshot = snapshot
    }

    /** Weeks already read this revision, by Monday, so the timetable draws
     * them at once; a new revision re-reads them. */
    private val weeks = mutableMapOf<LocalDate, ScheduleView>()

    /** A week read with its neighbours, if this revision has it. */
    fun cachedWeek(monday: LocalDate): ScheduleView? = weeks[monday]

    /** The week from `monday`, read with `WEEK_REACH` weeks on either side in
     * one query, so stepping or swiping through nearby weeks draws at once. */
    suspend fun week(monday: LocalDate): ScheduleView? {
        weeks[monday]?.let { return it }
        val term = today?.term ?: return null
        val first = Format.civil(monday.minusWeeks(WEEK_REACH))
        val last = Format.civil(monday.plusWeeks(WEEK_REACH).plusDays(4))
        val view = (query(WorkspaceQuery.ScheduleView(term, first, last, false)) as? WorkspaceQueryResult.ScheduleView)?.v1
            ?: return null
        for (offset in -WEEK_REACH..WEEK_REACH) {
            val start = monday.plusWeeks(offset)
            val from = Format.civil(start)
            val to = Format.civil(start.plusDays(4))
            // Civil dates compare as text.
            weeks[start] = view.copy(agendas = view.agendas.filter { it.date >= from && it.date <= to })
        }
        return weeks[monday]
    }

    /** Reads this week and its axis ahead of the timetable being opened. */
    private suspend fun prefetchTimetable() {
        timetableAxis(week(thisWeek()) ?: return)
    }

    /** The academic year (April–March) a date belongs to. */
    suspend fun academicYear(date: LocalDate): Int? =
        (query(WorkspaceQuery.AcademicYears(Format.civil(date), 0)) as? WorkspaceQueryResult.AcademicYears)?.v1?.currentYear

    /** Every week of an academic year with its quarter, for the week picker. */
    suspend fun timetableYear(year: Int): TimetableYear? =
        (query(WorkspaceQuery.TimetableYear(year)) as? WorkspaceQueryResult.TimetableYear)?.v1

    suspend fun courseDetail(id: String): CourseDetail? =
        (query(WorkspaceQuery.CourseDetail(id)) as? WorkspaceQueryResult.CourseDetail)?.v1

    /** A course's LMS content lesson by lesson. */
    suspend fun courseContent(id: String): CourseContent? =
        (query(WorkspaceQuery.CourseContent(id, Format.timestamp(Instant.now()))) as? WorkspaceQueryResult.CourseContent)?.v1

    /** One assignment with its LMS text and files, for the in-app page. */
    suspend fun assignmentDetail(id: String): AssignmentDetail? =
        (query(WorkspaceQuery.AssignmentDetail(id)) as? WorkspaceQueryResult.AssignmentDetail)?.v1

    /** The quarter's courses as Core lists them for the course list. */
    suspend fun overview(term: String): WorkspaceOverview? =
        (query(WorkspaceQuery.WorkspaceOverview(term)) as? WorkspaceQueryResult.WorkspaceOverview)?.v1

    /** Core's equal-row timetable axis, widened to the week's earliest start
     * and latest end when those leave the teaching day. */
    suspend fun timetableAxis(week: ScheduleView?): TimetableAxis? {
        val blocks = week?.agendas.orEmpty().flatMap { it.occurrences }
            .filter { it.kind == OccurrenceKind.COURSE || it.kind == OccurrenceKind.EXAM }
        val earliest = blocks.map { it.startTime }.minByOrNull(::clockMinutes)
        val latest = blocks.map { it.endTime }.maxByOrNull(::clockMinutes)
        val axis = (query(WorkspaceQuery.TimetableAxis(earliest, latest)) as? WorkspaceQueryResult.TimetableAxis)?.v1
        if (axis != null) lastTimetableAxis = axis
        return axis
    }

    /** The last axis Core gave, so a timetable draws its rows at once. */
    var lastTimetableAxis: TimetableAxis? = null
        private set

    /** A quarter as an iCalendar file, written by Core as the desktop's export. */
    suspend fun quarterIcs(term: String, scope: CalendarExportScope): String {
        val session = session ?: throw SdkException.Unavailable("quarter calendar")
        val query = WorkspaceQuery.QuarterIcs(term, scope, Format.timestamp(Instant.now()))
        val value = (session.scheduleQuery(listOf(query)).value() as? OperationResult.Schedule)?.snapshot?.values?.firstOrNull()
        return (value as? WorkspaceQueryResult.QuarterIcs)?.v1 ?: throw SdkException.Unavailable("quarter calendar")
    }

    /** Exams Core lists as upcoming (linked, scheduled, not hidden) from today
     * through `days` days ahead, for 今日. */
    suspend fun upcomingExams(days: Long = 14): List<ExamView> {
        val term = today?.term ?: return emptyList()
        val from = Format.civil(Format.today())
        val last = Format.civil(Format.today().plusDays(days))
        val view = (query(WorkspaceQuery.ScheduleView(term, from, from, false)) as? WorkspaceQueryResult.ScheduleView)?.v1
        // Civil dates compare as text.
        return view?.upcomingExams.orEmpty().filter { it.date <= last }
    }

    /** Downloads (or reuses) a course file through the signed-in LMS session. */
    suspend fun openMaterial(files: CourseKey, source: String): Material? {
        val session = session ?: return null
        val result = runCatching { session.openMaterial(files, source, true, false, null).value() }.getOrNull()
        return (result as? OperationResult.Material)?.material
    }

    // Hand-in (docs/ux/mobile.md, Hand-in): files go to the assignment's
    // workspace `submit/`; handing in fills the LMS draft area from it. Core
    // never submits; the person saves the page.

    /** The files waiting in the assignment's `submit/`. */
    suspend fun submitFiles(assignment: String): List<LocalFile> {
        val session = session ?: return emptyList()
        val result = runCatching { session.assignmentSubmitFiles(assignment).value() }.getOrNull()
        return (result as? OperationResult.Files)?.files.orEmpty()
    }

    /** Copies the files into the assignment's `submit/`; the sources stay. */
    suspend fun addSubmitFiles(assignment: String, files: List<SharedFile>): List<LocalFile> {
        val session = session ?: throw SdkException.Closed()
        val listed = (session.addAssignmentFiles(assignment, files).value() as? OperationResult.Files)?.files.orEmpty()
        refreshWaiting()
        return listed
    }

    suspend fun removeSubmitFile(assignment: String, name: String): List<LocalFile> {
        val session = session ?: throw SdkException.Closed()
        val listed = (session.removeAssignmentFile(assignment, name).value() as? OperationResult.Files)?.files.orEmpty()
        refreshWaiting()
        return listed
    }

    /** Re-reads which assignments have files waiting to be handed in. */
    suspend fun refreshWaiting() {
        val session = session ?: return
        val result = runCatching { session.waitingHandIns().value() }.getOrNull() as? OperationResult.Waiting ?: return
        waiting = result.assignments.groupBy({ it.assignmentId }, { it.files.toInt() }).mapValues { it.value.sum() }
    }

    /** Queues `submit/` for the LMS draft area; returns the page that receives it. */
    suspend fun handIn(assignment: String): String {
        val session = session ?: throw SdkException.Closed()
        val messages = UploadMessages(
            t("Preparing your LMS upload…"),
            t("Files sent to the upload area. Wait for LMS to finish, review the files, then submit yourself."),
            t("The upload area is not available. Choose the files in LMS."),
        )
        return (session.handInAssignment(assignment, messages).value() as? OperationResult.Text)?.value
            ?: throw SdkException.Unavailable("")
    }

    /** Gives inbox items to an assignment and removes them; on failure they stay. */
    suspend fun give(items: List<SharedInbox.Item>, assignment: String): Boolean {
        val files = items.map { SharedFile(it.file.path, it.name) }
        if (files.isEmpty()) return false
        return try {
            addSubmitFiles(assignment, files)
            items.forEach { SharedInbox.remove(context, it) }
            true
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            problem = describe(error)
            false
        }
    }

    /** The syllabus departments, as the desktop's course finder lists them. */
    suspend fun departments(): List<FinderDepartment> {
        val session = session ?: return emptyList()
        val term = today?.term ?: return emptyList()
        val year = term.take(4).toUShortOrNull() ?: return emptyList()
        val request = FinderRequest(year, term, false, emptyList(), "", null, 0u, 0u)
        val result = runCatching { session.finderQuery(request).value() }.getOrNull()
        return (result as? OperationResult.Finder)?.page?.context?.departments.orEmpty()
    }

    /** One schedule query's first value, or null when Core cannot answer.
     * A cancelled caller stays cancelled rather than reading "no answer". */
    private suspend fun query(query: WorkspaceQuery): WorkspaceQueryResult? {
        val session = session ?: return null
        val result = try {
            session.scheduleQuery(listOf(query)).value()
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            null
        }
        return (result as? OperationResult.Schedule)?.snapshot?.values?.firstOrNull()
    }

    // Intents

    suspend fun setHidden(item: DeadlineItem, hidden: Boolean) {
        // Offered at once: the re-read that follows can take a moment.
        undo = if (hidden) UndoAction(item.title, item.assignmentId) else null
        mark(item.assignmentId, hidden = hidden, pinned = null)
    }

    suspend fun setPinned(item: DeadlineItem, pinned: Boolean) {
        mark(item.assignmentId, hidden = null, pinned = pinned)
    }

    suspend fun undoHide() {
        val undo = undo ?: return
        this.undo = null
        mark(undo.assignmentId, hidden = false, pinned = null)
    }

    fun dismissUndo() {
        undo = null
    }

    fun dismissProblem() {
        problem = null
    }

    private suspend fun mark(id: String, hidden: Boolean?, pinned: Boolean?) {
        commit(WorkspaceIntent.SetAssignmentMark(id, hidden, pinned, null))
    }

    /** Opening an item of 今日's 新着: Core stops listing it until it changes. */
    suspend fun markRecentSeen(list: RecentKind, id: String) {
        commit(WorkspaceIntent.MarkRecentSeen(list, id))
    }

    /** 新着's read all: Core stops listing everything new now, past the few it
     * shows too. */
    suspend fun markAllRecentSeen() {
        commit(WorkspaceIntent.MarkAllRecentSeen)
    }

    /** Marks one session (`Occurrence.key`) as `exam`, `cancelled` or `class`.
     * Core republishes the schedule; the new content revision re-reads the week. */
    suspend fun markSession(key: String, mark: String) {
        commit(WorkspaceIntent.MarkSession(key, mark))
    }

    /** Records `sectionId` as the lesson of the class at `occurrence` (Core's
     * `CourseContent.classAt`), as a double click on the desktop rail does.
     * `false` when Core did not take it. */
    suspend fun recordLesson(courseId: String, occurrence: String, sectionId: ULong): Boolean =
        commit(WorkspaceIntent.RecordLesson(courseId, occurrence, sectionId))

    /** Commits one intent, then re-reads. A conflict (another device changed
     * the workspace) only re-reads, so the person can retry. */
    private suspend fun commit(intent: WorkspaceIntent): Boolean {
        val session = session ?: return false
        var committed = false
        try {
            (session.mutate(WorkspaceMutation(revision, intent)).value() as? OperationResult.Workspace)
                ?.let { revision = it.snapshot.revision }
            committed = true
        } catch (error: SdkException.Conflict) {
            // Re-read below.
        } catch (error: Exception) {
            problem = describe(error)
        }
        refresh()
        return committed
    }

    /** The study profile as the workspace stores it (`academicProfile`). */
    data class StudyProfile(val admissionYear: Int?, val enrollments: List<StudyEnrollment>)

    suspend fun studyProfile(): StudyProfile? {
        val session = session ?: return null
        val snapshot = runCatching { (session.snapshot().value() as? OperationResult.Workspace)?.snapshot }.getOrNull() ?: return null
        val profile = runCatching { JSONObject(snapshot.workspace).optJSONObject("academicProfile") }.getOrNull() ?: JSONObject()
        val rows = profile.optJSONArray("studyEnrollments")
        return StudyProfile(
            admissionYear = if (profile.has("admissionYear") && !profile.isNull("admissionYear")) profile.getInt("admissionYear") else null,
            enrollments = (0 until (rows?.length() ?: 0)).mapNotNull { index ->
                val row = rows?.optJSONObject(index) ?: return@mapNotNull null
                StudyEnrollment(
                    stage = when (row.optString("stage")) {
                        "undergraduate" -> StudyStage.UNDERGRADUATE
                        "master" -> StudyStage.MASTER
                        "professional" -> StudyStage.PROFESSIONAL
                        "doctoral" -> StudyStage.DOCTORAL
                        else -> StudyStage.UNKNOWN
                    },
                    entryKind = when (row.optString("entryKind")) {
                        "regular" -> StudyEntryKind.REGULAR
                        "transfer" -> StudyEntryKind.TRANSFER
                        else -> StudyEntryKind.UNKNOWN
                    },
                    entryGrade = if (row.has("entryGrade") && !row.isNull("entryGrade")) row.getInt("entryGrade").toUByte() else null,
                    admittedTerm = row.optString("admittedTerm"),
                )
            },
        )
    }

    /** Saves the cohort and degree history through the desktop's intent. */
    suspend fun setStudyProfile(profile: StudyProfile) {
        val session = session ?: return
        session.mutate(WorkspaceMutation(revision, WorkspaceIntent.SetStudyProfile(profile.admissionYear, profile.enrollments))).value()
        refresh()
    }

    /** Pull to refresh: Core's planner decides what to read, and the sync
     * snapshot shows it. */
    fun refreshFromSchool() {
        problem = null
        refresh(SyncScope.Everything)
    }

    suspend fun saveCredentials(profile: SchoolAuthProfile, credentials: SchoolCredentials) {
        val session = session ?: return
        session.saveCredentials(profile, credentials).value()
        refreshAccounts()
        runCatching { session.ensureSchoolSession(profile) }
        hint(Hint.SignedIn(profile))
    }

    /** The saved school OTP setup as an `otpauth://` link, read only when the
     * person asks to add it to an authenticator or copy it. */
    suspend fun otpSetupLink(): String {
        val session = session ?: throw IllegalStateException(t("This is not available right now."))
        return (session.schoolOtpLink(null).value() as? OperationResult.Text)?.value
            ?: throw IllegalStateException(t("This is not available right now."))
    }

    /** Edits `settings.notifications` and replaces the settings object. */
    suspend fun setNotifications(edit: (JSONObject) -> Unit) = updateSettings { settings ->
        val notifications = settings.optJSONObject("notifications") ?: JSONObject()
        edit(notifications)
        settings.put("notifications", notifications)
    }

    /** Edits the settings object and replaces it. Like the desktop store's
     * `updateSettings`, edits run one at a time, each on the settings the
     * previous one committed, so two quick toggles cannot both copy the same
     * old object. */
    suspend fun updateSettings(edit: (JSONObject) -> Unit): Unit = settingsWrite.withLock {
        val session = session ?: return@withLock
        val settings = JSONObject(records.settings.toString())
        edit(settings)
        val mutation = WorkspaceMutation(revision, WorkspaceIntent.ReplaceSettings(settings.toString()))
        try {
            (session.mutate(mutation).value() as? OperationResult.Workspace)?.snapshot?.let {
                revision = it.revision
                records = WorkspaceRecords(it.workspace)
            }
        } catch (error: Exception) {
            problem = describe(error)
        }
        refresh()
        notificationsBlocked = NotificationScheduler.sync(context, session)
    }

    /** Schedules the plan again, after the notification permission changed. */
    suspend fun resyncNotifications() {
        val session = session ?: return
        notificationsBlocked = NotificationScheduler.sync(context, session)
    }

    /** Whether this build can read a Matrix card (arm64; not the x86_64 emulator). */
    val canScanMatrix: Boolean
        get() = runCatching { session?.capabilities()?.ocr }.getOrNull() == true

    /** Reads the 70 Matrix cells from one JPEG/PNG, in row order (A1…J1, A2…). */
    suspend fun scanMatrix(image: ByteArray): List<Recognition> {
        val session = session ?: return emptyList()
        ocrProgress = 0
        return (session.scanMatrix(image).value() as? OperationResult.Recognitions)?.cells.orEmpty()
    }

    /** Starts a live Matrix scan; its seven-second budget starts now. */
    fun beginMatrixCamera(): MatrixCameraSession =
        session?.beginMatrixCamera() ?: throw SdkException.Closed()

    // Backup: the desktop's Backup settings over the same Core journal. Core
    // decides what uploads, merges and restores.

    suspend fun refreshCloud() {
        val session = session ?: return
        (runCatching { session.cloudState().value() }.getOrNull() as? OperationResult.Cloud)
            ?.let { cloud = it.state }
    }

    /** Turning Drive on first shows Google's consent; Core keeps it off with
     * the reason if that fails. */
    suspend fun setCloudEnabled(provider: CloudProvider, enabled: Boolean) =
        cloudOperation { it.setCloudEnabled(provider, enabled) }

    suspend fun setCloudAutomatic(automatic: Boolean) = cloudOperation { it.setCloudAutomatic(automatic) }

    suspend fun syncCloud(direction: Direction) {
        cloudOperation { it.syncCloud(direction) }
        refresh()
    }

    /** Restores a cloud version or a local recovery file, on every connected device. */
    suspend fun restoreCloud(id: String) {
        cloudOperation { it.restoreCloud(id) }
        refresh()
    }

    /** The current workspace as a backup file (the desktop's Export…). */
    suspend fun exportBackup(): String? {
        val session = session ?: return null
        return try {
            (session.exportBackup().value() as? OperationResult.Text)?.value
        } catch (error: Exception) {
            problem = describe(error)
            null
        }
    }

    /** Imports a backup file; Core saves a recovery file first and publishes
     * the import like any restore. Core rejects anything that is not a backup. */
    suspend fun importBackup(text: String) {
        val session = session ?: return
        session.applyBackup(text).value()
        refreshCloud()
        refresh()
    }

    private suspend fun cloudOperation(start: (Session) -> Operation) {
        val session = session ?: return
        try {
            (start(session).value() as? OperationResult.Cloud)?.let { cloud = it.state }
        } catch (error: Exception) {
            problem = describe(error)
            refreshCloud()
        }
    }

    /** Settings → Reset: what `request` would remove now, or null when Core
     * cannot tell. */
    suspend fun resetPreview(request: ResetRequest): ResetSummary? {
        val session = session ?: return null
        return runCatching { session.resetPreview(request) }.getOrNull()
    }

    /** Clears what `request` names. Core decides what each scope removes and
     * announces the change, which re-reads every screen. */
    suspend fun reset(request: ResetRequest) {
        val session = session ?: return
        session.reset(request, SchoolWebSignOut())
        problem = null
        refreshAccounts()
        readSync()
        refreshCloud()
    }

    /** After a full reset, as the desktop's: the providers that were on are
     * turned on again, then their copy is pulled. Throws when either fails. */
    suspend fun restoreFromCloud(providers: List<CloudProvider>) {
        val session = session ?: return
        for (provider in providers) session.setCloudEnabled(provider, true).value()
        session.syncCloud(Direction.PULL).value()
        refreshCloud()
        refresh()
    }

    suspend fun deleteCredentials(profile: SchoolAuthProfile) {
        val session = session ?: return
        runCatching { session.deleteCredentials(profile).value() }
        refreshAccounts()
    }

    companion object {
        fun describe(error: Throwable): String {
            Log.w("isct", "core error", error)
            return when (error) {
                is SdkException.Network -> t("You're offline. Showing saved data.")
                is SdkException.Authentication -> t("Sign in to your school account again.")
                is SdkException.Maintenance -> t("The school site is under maintenance.")
                is SdkException.Timeout -> t("The school site is not responding.")
                is SdkException.Busy -> t("The previous task has not finished yet.")
                is SdkException.Unavailable -> error.detail.ifEmpty { t("This is not available right now.") }
                else -> error.message ?: error.toString()
            }
        }
    }
}

data class UndoAction(val title: String, val assignmentId: String)

/** Weeks read on either side of the one asked for. */
private const val WEEK_REACH = 4L
