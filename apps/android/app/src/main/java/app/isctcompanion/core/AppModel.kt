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
import app.isctcompanion.design.Format
import app.isctcompanion.design.Localizer
import java.io.File
import java.time.Instant
import java.time.LocalDate
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
        private set
    var periods: List<TeachingPeriod> by mutableStateOf(emptyList())
        private set
    /** Course id → Core's display colour, identical on every surface. */
    var colors: Map<String, String> by mutableStateOf(emptyMap())
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
    /** Undo for the last hide, built from the intent Core committed. */
    var undo: UndoAction? by mutableStateOf(null)
        private set
    /** One "needs you" banner (sync feedback level 3). */
    var problem: String? by mutableStateOf(null)
        private set
    /** Cells read so far by a running Matrix scan, from Core's `ocr-progress`. */
    var ocrProgress: Int by mutableStateOf(0)
        private set
    /** Bumped whenever Core reports new content, so open screens re-query. */
    var contentRevision: String by mutableStateOf("")
        private set

    val browser = SchoolBrowserHost(context)
    val web = WebRouter(browser, scope, context)
    private var session: Session? = null
    private var revision = ""
    private var boundaryJob: Job? = null
    private var refreshJob: Job? = null
    private var wakeJob: Job? = null
    private val settingsWrite = Mutex()
    private var networkState = SyncNetwork.UNKNOWN
    private var foreground = true

    /** First launch: no school account configured yet. */
    val needsAccount: Boolean
        get() = credentials?.let {
            it.science.availability == CredentialAvailability.NOT_CONFIGURED &&
                it.tokyoTech.availability == CredentialAvailability.NOT_CONFIGURED
        } ?: false

    // Lifecycle

    suspend fun start() {
        if (session != null) return
        try {
            val data = File(context.filesDir, "core").apply { mkdirs() }
            val files = File(context.filesDir, "files").apply { mkdirs() }
            val config = SessionConfig(
                dataDirectory = data.absolutePath, filesDirectory = files.absolutePath,
                initialNetwork = SyncNetwork.UNKNOWN, cloudProviders = emptyList(),
                authorizedCloudProviders = emptyList(),
            )
            val session = Session.open(config, KeystoreVault(context), browser, null, null, WebCookieSession())
            this.session = session
            web.session = session
            val stream = session.subscribe()
            scope.launch { listen(stream) }
            refresh()
            refreshAccounts()
            readSync()
            status = Status.Ready
            watchNetwork()
            watchPowerSaver()
            hint(if (foreground) Hint.Foreground else Hint.Background)
            if (foreground) startWaking()
            reportSchool(SchoolSessionLifecycleKind.LAUNCH)
        } catch (error: Exception) {
            status = Status.Failed(describe(error))
        }
    }

    fun foreground(active: Boolean) {
        val session = session ?: return
        foreground = active
        if (active) {
            runCatching { session.resume() }
            hint(Hint.Foreground)
            reportSchool(SchoolSessionLifecycleKind.FOCUS)
            startWaking()
            scope.launch { refresh() }
        } else {
            wakeJob?.cancel()
            wakeJob = null
            hint(Hint.Background)
            reportSchool(SchoolSessionLifecycleKind.FOCUS)
            runCatching { session.suspend() }
        }
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
                        .onSuccess { syncSnapshot = it }
                    "school-session", "school-login-status" -> refreshAccounts()
                    "school-web-open" -> web.handleOpenEvent(update.event.payload)
                    "ocr-progress" -> runCatching { JSONObject(update.event.payload).getInt("completed") }
                        .onSuccess { ocrProgress = it }
                    "school-web-url", "school-download" -> Unit
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
                    listOf(WorkspaceQuery.Today(now), WorkspaceQuery.AssignmentBoard(now), WorkspaceQuery.TeachingPeriods),
                ).value() as? OperationResult.Schedule
                )?.snapshot ?: return
            for (value in result.values) {
                when (value) {
                    is WorkspaceQueryResult.Today -> today = value.v1
                    is WorkspaceQueryResult.AssignmentBoard -> board = value.v1
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
            if (changed) NotificationScheduler.sync(context, session)
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
        val shown = syncSnapshot
        if (shown == null || snapshot.generation >= shown.generation) syncSnapshot = snapshot
    }

    suspend fun week(monday: LocalDate): ScheduleView? {
        val session = session ?: return null
        val term = today?.term ?: return null
        val query = WorkspaceQuery.ScheduleView(term, Format.civil(monday), Format.civil(monday.plusDays(4)), false)
        val result = runCatching { session.scheduleQuery(listOf(query)).value() }.getOrNull()
        return ((result as? OperationResult.Schedule)?.snapshot?.values?.firstOrNull() as? WorkspaceQueryResult.ScheduleView)?.v1
    }

    suspend fun courseDetail(id: String): CourseDetail? {
        val session = session ?: return null
        val result = runCatching { session.scheduleQuery(listOf(WorkspaceQuery.CourseDetail(id))).value() }.getOrNull()
        return ((result as? OperationResult.Schedule)?.snapshot?.values?.firstOrNull() as? WorkspaceQueryResult.CourseDetail)?.v1
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
        val session = session ?: return
        val mutation = WorkspaceMutation(revision, WorkspaceIntent.SetAssignmentMark(id, hidden, pinned, null))
        try {
            (session.mutate(mutation).value() as? OperationResult.Workspace)?.let { revision = it.snapshot.revision }
        } catch (error: SdkException.Conflict) {
            // Another device changed the workspace: re-read and let the person retry.
        } catch (error: Exception) {
            problem = describe(error)
        }
        refresh()
    }

    /** Pull to refresh: Core's planner decides what to read, and the sync
     * snapshot shows it. */
    fun refreshFromSchool() = refresh(SyncScope.Everything)

    suspend fun saveCredentials(profile: SchoolAuthProfile, credentials: SchoolCredentials) {
        val session = session ?: return
        session.saveCredentials(profile, credentials).value()
        refreshAccounts()
        runCatching { session.ensureSchoolSession(profile) }
        hint(Hint.SignedIn(profile))
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
        NotificationScheduler.sync(context, session)
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
