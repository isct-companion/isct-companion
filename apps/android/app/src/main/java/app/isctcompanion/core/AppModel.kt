package app.isctcompanion.core

import app.isctcompanion.BackgroundWake
import app.isctcompanion.design.lookup
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
import app.isctcompanion.widgets.WidgetStorage
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ProcessLifecycleOwner
import app.isctcompanion.design.Format
import app.isctcompanion.design.Localizer
import java.io.File
import java.time.Instant
import java.time.LocalDate
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
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
class AppModel(internal val context: Context, private val scope: CoroutineScope) {
    /** Newer releases from GitHub, offered on 今日 (`AppUpdater`). */
    val updater = AppUpdater(context)

    sealed interface Status {
        data object Opening : Status
        data object Ready : Status
        data class Failed(val message: String) : Status
    }

    var status: Status by mutableStateOf(Status.Opening)
        private set
    val notices = app.isctcompanion.design.AppNotices()
    var announcements: AnnouncementsView? by mutableStateOf(null)
        private set
    var announcementsReady by mutableStateOf(false)
        private set
    var today: TodayView? by mutableStateOf(null)
        private set
    var board: AssignmentBoard? by mutableStateOf(null)
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
    /** Core's school session per account: state, problem, retry time and
     * maintenance, rendered as they are. */
    private var schoolSession: SchoolSessionSnapshot? by mutableStateOf(null)
    val sessions: SchoolSessionProfiles?
        get() = schoolSession?.profiles
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
    private var ocrOperation: ULong? = null
    /** The ScienceTokyoExam account and its upload, as Core's `exam-account`
     * events and `Account` answers report it; null until the page reads it. */
    var examAccount: ExamAccount? by mutableStateOf(null)
        private set
    /** Bumped whenever Core reports new content, so open screens re-query. */
    var contentRevision: String by mutableStateOf("")
        private set

    val browser = SchoolBrowserHost(context)
    /** Google Drive sign-in; the activity in front attaches its consent screen. */
    val drive = DriveTokens(context)
    val web = WebRouter(browser, scope, context, notices::show)
    internal var session: Session? = null
        private set
    internal val backup = CloudBackup(
        session = { session },
        refreshWorkspace = { refresh() },
    )
    private var revision = ""
    private val dashboardReads = ScheduleReads()
    internal val timetableWeeks = TimetableWeeks()
    private val sessionMarks = Mutex()
    private var boundaryJob: Job? = null
    /** The one timer for Core's next deadline (`WakeDirective`). */
    private var deadlineJob: Job? = null
    private var directive: WakeDirective? = null
    private val settingsWrite = Mutex()
    private val opening = Mutex()
    private var networkState = SyncNetwork.UNKNOWN

    /** First launch: no school account configured yet. */
    val needsAccount: Boolean
        get() = credentials?.let {
            it.science.availability == CredentialAvailability.NOT_CONFIGURED &&
                it.tokyoTech.availability == CredentialAvailability.NOT_CONFIGURED
        } ?: false

    // Lifecycle

    /** Whether the session was opened running rather than suspended. */
    private var openedInFront = false

    /** Opens the session once for the activity, background work or printing,
     * and again after a failed start (the Retry of the failure screen).
     * Other system entry points can run without loading the SDK. */
    suspend fun start() = opening.withLock {
        if (status == Status.Ready) return@withLock
        status = Status.Opening
        try {
            // A retry after a failure past this point keeps the open session.
            val session = session ?: openSession()
            refresh()
            refreshAccounts()
            readSync()
            backup.refresh()
            status = Status.Ready
            watchNetwork()
            watchPowerSaver()
            // WorkManager can start the process with no activity in front.
            val foreground = ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)
            hint(if (foreground) Hint.Foreground else Hint.Background)
            runCatching { session.wakeDirective() }.onSuccess(::applyDirective)
            if (foreground && !openedInFront) runCatching { session.resume() }
            if (!foreground && openedInFront) runCatching { session.suspend() }
            // The launch's first foreground came before the session opened, so
            // its update check never ran.
            if (foreground) scope.launch { updater.check(session, launch = true) }
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            status = Status.Failed(describe(error))
        }
    }

    /** Retries a failed start on the process scope so leaving the failure
     * screen does not cancel it. */
    fun retryStart() {
        scope.launch { start() }
    }

    /** Opens Core's session and starts listening to it; [start] runs the
     * reads that follow. */
    private suspend fun openSession(): Session {
        TlsVerifier.install(context)
        val data = File(context.filesDir, "core").apply { mkdirs() }
        val files = File(context.filesDir, "files").apply { mkdirs() }
        // Google Drive is Android's provider; Core reads whether it was
        // connected and owns every Drive request.
        val inFront = ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)
        openedInFront = inFront
        val config = SessionConfig(
            dataDirectory = data.absolutePath, filesDirectory = files.absolutePath,
            appVersion = context.packageManager.getPackageInfo(context.packageName, 0).versionName.orEmpty(),
            initialNetwork = SyncNetwork.UNKNOWN, startSuspended = !inFront, cloudProviders = listOf(CloudProvider.GOOGLE_DRIVE),
            authorizedCloudProviders = emptyList(),
        )
        val session = Session.open(
            config, KeystoreVault(context), browser, null, WebCookieSeed(), null, null, ExamBrowserOpener(context), drive,
        )
        session.attachWidgets(WidgetStorage(context))
        val model = java.lang.ref.WeakReference(this)
        session.attachReminderPort(NotificationScheduler(context) { needed ->
            model.get()?.notificationsBlocked = needed
        })
        this.session = session
        val stream = session.subscribe()
        scope.launch { listen(stream) }
        // The web screen reads page snapshots only once events stream.
        web.session = session
        return session
    }

    suspend fun widgetRoute(url: String): WidgetRoute? {
        val session = session ?: return null
        return try {
            (session.widgetRoute(url).value() as? OperationResult.WidgetRoute)?.route
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            Log.w("isct", "widget link", error)
            null
        }
    }

    fun foreground(active: Boolean) {
        val session = session ?: return
        if (active) {
            runCatching { session.resume() }
            hint(Hint.Foreground)
            armDeadline()
            scheduleRefresh(force = true)
            scope.launch { checkForUpdate() }
        } else {
            announcementsReady = false
            deadlineJob?.cancel()
            deadlineJob = null
            hint(Hint.Background)
            runCatching { session.suspend() }
            scope.launch { BackgroundWake.schedule(context, directive) }
        }
    }

    /** One job from the system's print dialog (`CampusPrintService`): Core
     * verifies the student ID it is billed to, sends the PDF and words how
     * it ended. The print service can start the process with no activity,
     * so the session opens here if it has not. `null` when there is no
     * session to print with. */
    suspend fun campusPrint(document: ByteArray, job: CampusPrintJob): CampusPrintReport? {
        start()
        val session = session ?: return null
        return (withContext(Dispatchers.IO) { session.campusPrint(document, job) }.value() as? OperationResult.CampusPrint)?.report
    }

    /** One WorkManager opportunity out of front: Core reads what is due when
     * [network] allows (LMS deadlines and assignment states over the token,
     * reference files) and runs its local work, within [budgetSeconds], then
     * suspends again unless the app came to the front. A stopped worker ends
     * it early. */
    suspend fun backgroundWake(network: Boolean, budgetSeconds: UInt, systemChange: Boolean) {
        start()
        val session = session ?: return
        if (systemChange) {
            runCatching { session.refreshWidgets() }
            resyncNotifications()
        }
        val id = java.util.UUID.randomUUID().toString()
        val operation = runCatching { session.beginBackgroundOpportunity(id, budgetSeconds, network) }
            .getOrNull() ?: return
        try {
            operation.awaitResult()
        } catch (error: CancellationException) {
            runCatching { session.endBackgroundOpportunity(id) }
            runCatching { operation.cancel() }
            throw error
        } catch (error: Exception) {
            Log.w("isct", "background opportunity", error)
        }
        refresh()
        runCatching { session.wakeDirective() }.onSuccess(::applyDirective)
        BackgroundWake.schedule(context, directive)
    }

    /** A lifecycle fact, opened view or refresh request for Core's sync
     * planner, which decides what is due. Never waits. */
    fun hint(hint: Hint) {
        runCatching { session?.syncHint(hint) }
    }

    /** The view now on screen, so Core reads what it shows first. */
    fun open(view: SyncView) = hint(Hint.Open(view))

    fun refresh(target: SyncScope) = hint(Hint.Refresh(target))

    fun retrySync(plan: RetryPlan) = plan.scopes.forEach(::refresh)

    /** Takes Core's directive, unless an older one; the in-front timer and
     * the WorkManager requests follow it. */
    private fun applyDirective(next: WakeDirective) {
        val current = directive
        if (current != null && current.revision >= next.revision) return
        directive = next
        val foreground = ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)
        if (foreground) armDeadline() else scope.launch { BackgroundWake.schedule(context, next) }
    }

    /** One job sleeps until the directive's deadline and tells Core; the next
     * directive arms it again. Only in front: out of front WorkManager gives
     * Core its opportunities. */
    private fun armDeadline() {
        deadlineJob?.cancel()
        deadlineJob = null
        val session = session ?: return
        val directive = directive ?: return
        val at = Format.date(directive.at) ?: return
        deadlineJob = scope.launch {
            val wait = at.toEpochMilli() - Instant.now().toEpochMilli()
            if (wait > 0) delay(wait)
            runCatching { session.deadlineReached(directive.revision) }
        }
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
                    "school-session" -> {
                        val snapshot = runCatching { decodeSchoolSession(update.event.payload) }.getOrNull()
                        if (snapshot == null) {
                            refreshAccounts()
                        } else {
                            applySchool(snapshot)
                            refreshCredentials()
                        }
                    }
                    "cloud-sync-state" -> backup.refresh()
                    "wake-directive" -> runCatching { decodeWakeDirective(update.event.payload) }
                        .onSuccess(::applyDirective)
                    "exam-account" -> runCatching { decodeExamAccount(update.event.payload) }
                        .onSuccess { examAccount = it }
                    "ocr-progress" -> runCatching {
                        val progress = JSONObject(update.event.payload)
                        if (progress.getLong("operationId").toULong() == ocrOperation) {
                            ocrProgress = progress.getInt("completed")
                        }
                    }
                    "school-web-page", "school-web-notice" -> web.handle(update.event.name, update.event.payload)
                    // No screen here lists finder results; the catalog
                    // changes once per department while it downloads.
                    "school-download", "catalog-changed" -> Unit
                    "workspace-changed" -> scheduleRefresh()
                    else -> Unit
                }
                is StreamUpdate.SnapshotRequired -> {
                    runCatching { session?.refreshWidgets() }
                    scheduleRefresh(force = true)
                    session?.let { runCatching { it.wakeDirective() }.onSuccess(::applyDirective) }
                    web.refreshPages()
                    refreshAccounts()
                    readSync()
                    // Only once the past-exams page read it; events carry it after.
                    if (examAccount != null) scope.launch { runCatching { examShare(ExamAction.Account) } }
                }
            }
        }
    }

    private fun scheduleRefresh(force: Boolean = false): Job? {
        val session = session ?: return null
        if (force) {
            announcementsReady = false
            dashboardReads.invalidate()
            timetableWeeks.invalidate()
        } else {
            timetableWeeks.invalidateIfChanged(session)
        }
        return dashboardReads.refresh(scope, session, { readDashboard(session, it) }, { problem = describe(it) })
    }

    suspend fun refresh() { scheduleRefresh(force = true)?.join() }

    private suspend fun readDashboard(session: Session, generation: Long) {
        val instant = Instant.now()
        val now = instant.toString()
        val previousRevision = revision
        val current = (session.workspaceRevision().value() as OperationResult.Text).value
        val queries = listOf(
            WorkspaceQuery.Records(listOf("courses", "assignments")), WorkspaceQuery.Today(now),
            WorkspaceQuery.AssignmentBoard(now, null), WorkspaceQuery.TeachingPeriods, WorkspaceQuery.Announcements,
        )
        val result = (session.scheduleQuery(queries).value() as OperationResult.Schedule).snapshot
        val overview = try {
            (session.scheduleQuery(listOf(WorkspaceQuery.WorkspaceOverview(""))).value() as OperationResult.Schedule).snapshot
        } catch (e: SdkException) {
            // A new install may have no term for an overview yet.
            null
        }
        val snapshot = if (overview == null) result else result.copy(reads = (result.reads + overview.reads).distinct())
        val boundary = result.values.filterIsInstance<WorkspaceQueryResult.Today>().firstOrNull()?.v1?.nextBoundary
        if (Format.date(boundary)?.let { !it.isAfter(Instant.now()) } == true) return
        if (!dashboardReads.accept(snapshot, generation, session)) return
        if (revision == previousRevision) revision = current
        val key = "$generation-${result.seq}"
        // Publish every view and its revision before the next suspension.
        for (value in result.values) {
            when (value) {
                is WorkspaceQueryResult.Records -> records = WorkspaceRecords(value.v1)
                is WorkspaceQueryResult.Today -> today = value.v1
                is WorkspaceQueryResult.AssignmentBoard -> board = value.v1
                is WorkspaceQueryResult.TeachingPeriods -> periods = value.v1
                is WorkspaceQueryResult.Announcements -> {
                    announcements = value.v1
                    announcementsReady = true
                }
                else -> Unit
            }
        }
        (overview?.values?.firstOrNull() as? WorkspaceQueryResult.WorkspaceOverview)?.let { colors = it.v1.colors }
        contentRevision = key
        scheduleBoundary()
    }

    /** Re-query when Core says the phase or hero changes next. */
    private fun scheduleBoundary() {
        boundaryJob?.cancel()
        val at = Format.date(today?.nextBoundary) ?: return
        val delayMs = maxOf(1L, at.toEpochMilli() - System.currentTimeMillis())
        boundaryJob = scope.launch {
            delay(delayMs)
            dashboardReads.invalidate()
            scheduleRefresh()?.join()
        }
    }

    internal suspend fun refreshAccounts() {
        val session = session ?: return
        runCatching { (session.schoolSession().value() as? OperationResult.SchoolSession)?.snapshot }
            .getOrNull()?.let(::applySchool)
        refreshCredentials()
    }

    private suspend fun refreshCredentials() {
        val session = session ?: return
        runCatching { (session.credentialStatus().value() as? OperationResult.Credentials)?.status }
            .getOrNull()?.let { credentials = it }
    }

    /** Keeps the higher `revision` of Core's `school-session` events and
     * `schoolSession` reads. */
    internal fun applySchool(snapshot: SchoolSessionSnapshot) {
        val current = schoolSession
        if (current != null && snapshot.revision <= current.revision) return
        schoolSession = snapshot
    }

    /** Reads the 70 Matrix cells from one JPEG/PNG, in row order (A1…J1, A2…). */
    suspend fun scanMatrix(image: ByteArray): List<MatrixCell> {
        val session = session ?: return emptyList()
        ocrProgress = 0
        val operation = session.scanMatrix(image)
        val id = operation.id()
        ocrOperation = id
        return try {
            (operation.value() as? OperationResult.Recognitions)?.cells.orEmpty()
        } finally {
            if (ocrOperation == id) ocrOperation = null
        }
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

    suspend fun week(monday: LocalDate): TimetablePage? {
        val session = session ?: return null
        return timetableWeeks.week(monday, session) { scheduleSnapshot(session, it) }
    }

    /** The academic year (April–March) a date belongs to. */
    suspend fun academicYear(date: LocalDate): Int? =
        (query(WorkspaceQuery.AcademicYears(Format.civil(date), 0)) as? WorkspaceQueryResult.AcademicYears)?.v1?.currentYear

    /** Every week of an academic year with its quarter, for the week picker. */
    suspend fun timetableYear(year: Int): TimetableYear? =
        (query(WorkspaceQuery.TimetableYear(year)) as? WorkspaceQueryResult.TimetableYear)?.v1

    /** A course's page as of now: exams already over are left out. */
    suspend fun courseDetail(id: String): CourseDetail? =
        (query(WorkspaceQuery.CourseDetail(id, Format.timestamp(Instant.now()))) as? WorkspaceQueryResult.CourseDetail)?.v1

    /** A course's LMS content lesson by lesson. */
    suspend fun courseContent(id: String): CourseContent? =
        (query(WorkspaceQuery.CourseContent(id, Format.timestamp(Instant.now()))) as? WorkspaceQueryResult.CourseContent)?.v1

    /** One assignment with its LMS text and files, for the in-app page. */
    suspend fun assignmentDetail(id: String): AssignmentDetail? =
        (query(WorkspaceQuery.AssignmentDetail(id)) as? WorkspaceQueryResult.AssignmentDetail)?.v1

    suspend fun deadlineDisplay(at: String): TaskDeadlineView? =
        (query(WorkspaceQuery.DeadlineDisplay(at, Instant.now().toString())) as? WorkspaceQueryResult.DeadlineDisplay)?.v1

    /** The quarter's courses as Core lists them for the course list. */
    suspend fun lectureList(term: String): LectureList? =
        (query(WorkspaceQuery.LectureList(term, Format.timestamp(Instant.now()))) as? WorkspaceQueryResult.LectureList)?.v1

    suspend fun examShare(action: ExamAction): ExamAnswer {
        val session = session ?: throw SdkException.Closed()
        val answer = try {
            (session.examShare(action).value() as OperationResult.ExamShare).answer
        } finally {
            // A connect's browser sign-in is over, whichever way it ended.
            if (action is ExamAction.Connect) SignInService.stop(context)
        }
        if (answer is ExamAnswer.Account) examAccount = answer.v1
        return answer
    }

    suspend fun examCourses(): ExamCourseList? =
        (query(WorkspaceQuery.ExamCourses) as? WorkspaceQueryResult.ExamCourses)?.v1

    /** The last axis Core gave, so a timetable draws its rows at once. */
    var lastTimetableAxis: TimetableAxis? = null

    /** A quarter as an iCalendar file, written by Core as the desktop's export. */
    suspend fun quarterIcs(term: String, scope: CalendarExportScope): String {
        val session = session ?: throw SdkException.Unavailable("quarter calendar")
        val query = WorkspaceQuery.QuarterIcs(term, scope, Format.timestamp(Instant.now()))
        val value = (session.scheduleQuery(listOf(query)).value() as? OperationResult.Schedule)?.snapshot?.values?.firstOrNull()
        return (value as? WorkspaceQueryResult.QuarterIcs)?.v1 ?: throw SdkException.Unavailable("quarter calendar")
    }

    /** Downloads (or reuses) a course file through the signed-in LMS session. */
    /** What an item or link of the course `courseId` opens as; a file is
     * downloaded (or reused) through the signed-in LMS session. */
    suspend fun openMaterial(courseId: String, source: String): Material? {
        val session = session ?: return null
        val result = runCatching { session.openMaterial(courseId, source, true, false, null).value() }.getOrNull()
        return (result as? OperationResult.Material)?.material
    }

    /** Asks Core for a newer release when a check is due (or now, `manual`). */
    suspend fun checkForUpdate(manual: Boolean = false) {
        val session = session ?: return
        updater.check(session, manual)
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
        return scheduleSnapshot(session, query)?.values?.firstOrNull()
    }

    private suspend fun scheduleSnapshot(session: Session, query: WorkspaceQuery): ScheduleSnapshot? {
        val result = try {
            session.scheduleQuery(listOf(query)).value()
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            null
        }
        return (result as? OperationResult.Schedule)?.snapshot
    }

    suspend fun answerAnnouncement(id: String) {
        commit(WorkspaceIntent.AnswerAnnouncement(id))
    }

    // Intents

    suspend fun setHidden(item: DeadlineItem, hidden: Boolean) {
        if (mark(item.assignmentId, hidden = hidden, pinned = null) && hidden) {
            undo = UndoAction(item.title, item.assignmentId)
        }
    }

    suspend fun setPinned(item: DeadlineItem, pinned: Boolean) {
        mark(item.assignmentId, hidden = null, pinned = pinned)
    }

    suspend fun undoHide(operation: UndoAction) {
        if (undo != operation) return
        undo = null
        mark(operation.assignmentId, hidden = false, pinned = null)
    }

    fun dismissUndo(operation: UndoAction) {
        if (undo == operation) undo = null
    }

    fun dismissProblem() {
        problem = null
    }

    private suspend fun mark(id: String, hidden: Boolean?, pinned: Boolean?) =
        commit(WorkspaceIntent.SetAssignmentMark(id, hidden, pinned, null))

    /** Opening an item of 今日's 新着: Core stops listing it until it changes. */
    suspend fun markRecentSeen(list: RecentKind, id: String) {
        commit(WorkspaceIntent.MarkRecentSeen(list, id))
    }

    /** 新着's read all: Core stops listing everything new now, past the few it
     * shows too. */
    suspend fun markAllRecentSeen() {
        commit(WorkspaceIntent.MarkAllRecentSeen)
    }

    /** Marks one session (`Occurrence.key`) as `exam`, `quiz`, `cancelled` or
     * `class` (a plain class; Core clears the mark when that is the default).
     * The commit names `calendar`, which re-queries the week. */
    fun markSession(key: String, mark: String): Job = scope.launch {
        // Each mark uses the workspace revision returned by the preceding commit.
        sessionMarks.withLock { commit(WorkspaceIntent.MarkSession(key, mark)) }
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
            (session.mutateCommit(WorkspaceMutation(revision, intent)).value() as? OperationResult.Committed)
                ?.let { revision = it.commit.revision }
            committed = true
        } catch (error: SdkException.Conflict) {
            // Re-read below.
        } catch (error: Exception) {
            problem = describe(error)
        }
        if (committed) scheduleRefresh()?.join() else refresh()
        return committed
    }

    /** The study profile as the workspace stores it (`academicProfile`). */
    data class StudyProfile(val admissionYear: Int?, val enrollments: List<StudyEnrollment>)

    suspend fun studyProfile(): StudyProfile? {
        val session = session ?: return null
        val records = runCatching {
            val result = session.scheduleQuery(listOf(WorkspaceQuery.Records(listOf("academicProfile")))).value()
            (result as? OperationResult.Schedule)?.snapshot?.values?.firstOrNull() as? WorkspaceQueryResult.Records
        }.getOrNull() ?: return null
        val profile = runCatching { JSONObject(records.v1).optJSONObject("academicProfile") }.getOrNull() ?: JSONObject()
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
        (session.mutateCommit(WorkspaceMutation(revision, WorkspaceIntent.SetStudyProfile(profile.admissionYear, profile.enrollments))).value() as? OperationResult.Committed)
            ?.let { revision = it.commit.revision }
        scheduleRefresh()?.join()
    }

    /** Pull to refresh: Core's planner decides what to read, and the sync
     * snapshot shows it. */
    fun refreshFromSchool() {
        problem = null
        refresh(SyncScope.Everything)
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
        val settings = JSONObject(records.settings.toString())
        edit(settings)
        commit(WorkspaceIntent.ReplaceSettings(settings.toString()))
        Unit
    }

    /** Reports a change in the system's reminder settings. */
    fun resyncNotifications() {
        runCatching { session?.refreshReminderPort() }
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
        try {
            session.reset(request)
        } finally {
            timetableWeeks.invalidate(discard = true)
            lastTimetableAxis = null
        }
        problem = null
        refreshAccounts()
        readSync()
        backup.refresh()
    }

    companion object {
        fun sessionText(session: SchoolProfileSession): String =
            signInText(session.presentation, session.retryAt)

        fun signInText(presentation: SignInPresentation, retryAt: String? = null): String {
            val time = Format.date(retryAt)?.let { Format.time(it) }
            val state = if (time != null) t(presentation.stateKey, "time" to time) else t(presentation.stateKey)
            return listOfNotNull(state, presentation.reasonKey?.let { t(it) }).joinToString(" · ")
        }

        /** An account's site paused for maintenance (LMS for Science Tokyo,
         * 教務Web for Tokyo Tech), until when, with the school's notice. */
        fun maintenanceText(profile: SchoolAuthProfile, maintenance: SchoolMaintenance): String {
            val source = if (profile == SchoolAuthProfile.SCIENCE) "LMS" else "教務Web"
            val line = if (Format.date(maintenance.until) != null) {
                t("schoolSession.maintenance.until", "site" to source, "time" to Format.due(maintenance.until))
            } else {
                t("syncStatus.sourceIsUnderMaintenance", "source" to source)
            }
            return listOfNotNull(line, maintenance.notice?.ifBlank { null }).joinToString("\n")
        }

        fun describe(error: Throwable): String {
            Log.w("isct", "core error", error)
            // A failure Core raised carries its stable code as `detail`; its
            // copy is the shared catalog's `error.<code>`. Other details are
            // never shown.
            coreCode(error)?.let { code ->
                lookup("error.$code")?.let { return it }
            }
            return when (error) {
                is SdkException.Network -> t("failure.youreOfflineShowingSavedData")
                is SdkException.Authentication -> t("failure.signInToYourSchool")
                is SdkException.Maintenance -> t("failure.theSchoolSiteIsUnder")
                is SdkException.Timeout -> t("failure.theSchoolSiteIsNot")
                is SdkException.Busy -> t("failure.thePreviousTaskHasNot")
                is SdkException.Unavailable -> t("failure.thisIsNotAvailableRight")
                is SdkException -> t("error.internal")
                else -> error.message ?: error.toString()
            }
        }

        private fun coreCode(error: Throwable): String? = when (error) {
            is SdkException.InvalidInput -> error.detail
            is SdkException.Conflict -> error.detail
            is SdkException.Storage -> error.detail
            is SdkException.Compatibility -> error.detail
            is SdkException.Authentication -> error.detail
            is SdkException.Maintenance -> error.detail
            is SdkException.Network -> error.detail
            is SdkException.Timeout -> error.detail
            is SdkException.Unavailable -> error.detail
            is SdkException.Busy -> error.detail
            is SdkException.Callback -> error.detail
            is SdkException.Internal -> error.detail
            else -> null
        }
    }
}

data class UndoAction(val title: String, val assignmentId: String, val id: java.util.UUID = java.util.UUID.randomUUID())

/** What opening Core's campus map page answered. */
sealed interface CampusMapOpening {
    data class Ready(val scene: CampusMapScene) : CampusMapOpening

    /** No map file is stored yet: it comes with the data updates. */
    data object Missing : CampusMapOpening

    data class Failed(val message: String) : CampusMapOpening
}
