package app.isctcompanion.core

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import app.isctcompanion.design.Format
import java.io.File
import java.time.Instant
import java.time.LocalDate
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
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
    var records: WorkspaceRecords by mutableStateOf(WorkspaceRecords())
        private set
    var credentials: SchoolCredentialStatus? by mutableStateOf(null)
        private set
    var sessions: SchoolSessionStates? by mutableStateOf(null)
        private set
    var pulls: List<PullSnapshot> by mutableStateOf(emptyList())
        private set
    var lastPull: Instant? by mutableStateOf(null)
        private set
    /** Undo for the last hide, built from the intent Core committed. */
    var undo: UndoAction? by mutableStateOf(null)
        private set
    /** One "needs you" banner (sync feedback level 3). */
    var problem: String? by mutableStateOf(null)
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
    private var networkState = PullNetwork.UNKNOWN
    private var foreground = true

    /** First launch: no school account configured yet. */
    val needsAccount: Boolean
        get() = credentials?.let {
            it.science.availability == CredentialAvailability.NOT_CONFIGURED &&
                it.tokyoTech.availability == CredentialAvailability.NOT_CONFIGURED
        } ?: false

    val isPulling: Boolean
        get() = pulls.any {
            it.state in setOf(PullState.RUNNING, PullState.APPLYING, PullState.YIELDING, PullState.CHECKING)
        }

    // Lifecycle

    suspend fun start() {
        if (session != null) return
        try {
            val data = File(context.filesDir, "core").apply { mkdirs() }
            val files = File(context.filesDir, "files").apply { mkdirs() }
            val config = SessionConfig(
                dataDirectory = data.absolutePath, filesDirectory = files.absolutePath,
                initialNetwork = PullNetwork.UNKNOWN, cloudProviders = emptyList(),
                authorizedCloudProviders = emptyList(),
            )
            val session = Session.open(config, KeystoreVault(context), browser, null, null, null)
            this.session = session
            web.session = session
            val stream = session.subscribe()
            scope.launch { listen(stream) }
            refresh()
            refreshAccounts()
            status = Status.Ready
            watchNetwork()
            report(foreground = true)
        } catch (error: Exception) {
            status = Status.Failed(describe(error))
        }
    }

    fun foreground(active: Boolean) {
        val session = session ?: return
        foreground = active
        if (active) {
            runCatching { session.resume() }
            report(foreground = true)
            scope.launch { refresh() }
        } else {
            report(foreground = false)
            runCatching { session.suspend() }
        }
    }

    private fun report(foreground: Boolean) {
        val session = session ?: return
        runCatching { session.lifecycle(foreground, networkState, web.isVisible) }
        runCatching {
            session.schoolLifecycle(
                SchoolSessionLifecycle(
                    if (foreground) SchoolSessionLifecycleKind.FOCUS else SchoolSessionLifecycleKind.LAUNCH,
                    foreground,
                ),
            )
        }
    }

    private fun watchNetwork() {
        val manager = context.getSystemService(ConnectivityManager::class.java) ?: return
        val update = { state: PullNetwork ->
            scope.launch {
                if (networkState != state) {
                    networkState = state
                    report(foreground)
                }
            }
        }
        manager.registerDefaultNetworkCallback(object : ConnectivityManager.NetworkCallback() {
            override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) {
                update(
                    when {
                        capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> PullNetwork.WIFI
                        capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> PullNetwork.CELLULAR
                        else -> PullNetwork.UNKNOWN
                    },
                )
            }

            override fun onLost(network: Network) {
                update(PullNetwork.OFFLINE)
            }
        })
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
                    "pull-state-changed", "school-pull-progress" -> refreshPulls()
                    "school-session", "school-login-status" -> refreshAccounts()
                    "school-web-open" -> web.handleOpenEvent(update.event.payload)
                    "school-web-url", "school-download", "ocr-progress" -> Unit
                    else -> scheduleRefresh()
                }
                is StreamUpdate.SnapshotRequired -> {
                    scheduleRefresh()
                    refreshAccounts()
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

    private suspend fun refreshPulls() {
        val session = session ?: return
        val snapshots = runCatching { (session.pullState().value() as? OperationResult.Pulls)?.snapshots }
            .getOrNull() ?: return
        pulls = snapshots
        if (snapshots.any { it.state == PullState.SUCCEEDED }) lastPull = Instant.now()
        snapshots.firstOrNull { it.state == PullState.FAILED || it.state == PullState.BLOCKED }?.let {
            problem = it.lastError
        }
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

    /** Manual pull: LMS assignments and materials, then 教務Web. */
    suspend fun pull() {
        val session = session ?: return
        problem = null
        for (site in listOf(SchoolSite.SCIENCE_LMS, SchoolSite.KYOMU)) {
            try {
                val result = session.pullSchool(site, null, true).value() as? OperationResult.SchoolPull
                val failed = result?.outcomes?.firstOrNull {
                    it.state == IngestState.RECONNECT_REQUIRED || it.state == IngestState.ERROR
                }
                if (failed != null) {
                    problem = if (failed.state == IngestState.RECONNECT_REQUIRED) {
                        describe(SdkException.Authentication(""))
                    } else {
                        failed.message
                    }
                    break
                }
            } catch (error: Exception) {
                problem = describe(error)
                break
            }
        }
        lastPull = Instant.now()
        refresh()
    }

    suspend fun saveCredentials(profile: SchoolAuthProfile, credentials: SchoolCredentials) {
        val session = session ?: return
        session.saveCredentials(profile, credentials).value()
        refreshAccounts()
        runCatching { session.ensureSchoolSession(profile) }
    }

    /** Edits `settings.notifications` and replaces the settings object. */
    suspend fun setNotifications(edit: (JSONObject) -> Unit) {
        val session = session ?: return
        val settings = JSONObject(records.settings.toString())
        val notifications = settings.optJSONObject("notifications") ?: JSONObject()
        edit(notifications)
        settings.put("notifications", notifications)
        val mutation = WorkspaceMutation(revision, WorkspaceIntent.ReplaceSettings(settings.toString()))
        try {
            session.mutate(mutation).value()
        } catch (error: Exception) {
            problem = describe(error)
        }
        refresh()
        NotificationScheduler.sync(context, session)
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
                is SdkException.Network -> "オフラインです。保存済みのデータを表示しています。"
                is SdkException.Authentication -> "学校アカウントに再ログインが必要です。"
                is SdkException.Maintenance -> "学校のサイトはメンテナンス中です。"
                is SdkException.Timeout -> "学校サイトの応答がありません。"
                is SdkException.Busy -> "前の処理がまだ終わっていません。"
                is SdkException.Unavailable -> error.detail.ifEmpty { "この機能は今使えません。" }
                else -> error.message ?: error.toString()
            }
        }
    }
}

data class UndoAction(val title: String, val assignmentId: String)
