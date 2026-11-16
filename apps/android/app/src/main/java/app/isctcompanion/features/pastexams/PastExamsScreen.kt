package app.isctcompanion.features.pastexams

import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.R
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.ExamAccountStatus
import app.isctcompanion.core.ExamAction
import app.isctcompanion.core.ExamAnswer
import app.isctcompanion.core.ExamChoices
import app.isctcompanion.core.ExamDocument
import app.isctcompanion.core.ExamLibrary
import app.isctcompanion.core.ExamLocalDocument
import app.isctcompanion.core.ExamOccasion
import app.isctcompanion.core.ExamReportReason
import app.isctcompanion.core.ExamRequest
import app.isctcompanion.core.ExamRequestCursor
import app.isctcompanion.core.ExamRequestDraft
import app.isctcompanion.core.ExamRequestPage
import app.isctcompanion.core.ExamRole
import app.isctcompanion.core.ExamSubject
import app.isctcompanion.core.ExamSubjectPage
import app.isctcompanion.core.ExamUploadPhase
import app.isctcompanion.core.SdkException
import app.isctcompanion.design.Card
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

private enum class Tab(val key: String) {
    Browse("examShare.browse"), Requests("examShare.requests"), Library("examShare.library"),
}

internal data class Confirmation(val key: String, val action: ExamAction)

/** The API word an enum value stands for (`WRONG_ANSWER` → `wrongAnswer`),
 * which names its label; the API calls `NOT_APPLICABLE` `none`. */
internal val Enum<*>.wire: String
    get() = if (this == ExamOccasion.NOT_APPLICABLE) "none" else name.lowercase().split('_')
        .mapIndexed { index, word -> if (index == 0) word else word.replaceFirstChar(Char::uppercase) }
        .joinToString("")

/** What one past-exams page hands the next across a navigation: Core answers
 * with values, which routes cannot carry. Lost with the process, when each
 * page loads its own again. */
internal object ExamViews {
    /** Documents opened from a list, taken by the document page they open. */
    val documents = mutableMapOf<String, ExamLocalDocument>()
    /** Requests listed on a board, read by the request page. */
    val requests = mutableMapOf<String, ExamRequest>()
    /** The board's page on the site, from the last board Core answered. */
    var requestBoardUrl: String? = null

    fun board(page: ExamRequestPage) {
        page.items.forEach { requests[it.id] = it }
        requestBoardUrl = page.webUrl
    }
}

/** One page's calls into Core: one at a time, a failure shown on the page. */
@Stable
internal class ExamCalls(private val model: AppModel) {
    var busy by mutableStateOf(false)
        private set
    var connecting by mutableStateOf(false)
        private set
    var error by mutableStateOf<String?>(null)
    var notice by mutableStateOf<String?>(null)

    suspend fun call(action: ExamAction): ExamAnswer? {
        if (busy) return null
        busy = true
        connecting = action == ExamAction.Connect
        error = null
        notice = null
        return try {
            model.examShare(action)
        } catch (_: SdkException.Cancelled) {
            null
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error = AppModel.describe(e)
            null
        } finally {
            connecting = false
            busy = false
        }
    }
}

@Composable
internal fun rememberExamCalls(): ExamCalls {
    val model = LocalModel.current
    return remember(model) { ExamCalls(model) }
}

/** Core's form choices and limits, read once per page. */
@Composable
internal fun rememberExamChoices(calls: ExamCalls): State<ExamChoices?> {
    val model = LocalModel.current
    return produceState<ExamChoices?>(null) {
        value = try {
            (model.examShare(ExamAction.Choices) as ExamAnswer.Choices).v1
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (failure: Exception) {
            calls.error = AppModel.describe(failure)
            null
        }
    }
}

/** Opens documents from a list: the one opening shows a spinner, and the
 * last one tapped is the one shown. */
@Stable
internal class ExamOpener(
    private val model: AppModel,
    private val scope: CoroutineScope,
    private val calls: ExamCalls,
    private val show: (String) -> Unit,
) {
    var opening by mutableStateOf<String?>(null)
        private set

    fun open(id: String) {
        opening = id
        calls.error = null
        scope.launch {
            val opened = try {
                (model.examShare(ExamAction.Open(id, false)) as? ExamAnswer.Document)?.v1
            } catch (_: SdkException.Cancelled) {
                null
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (failure: Exception) {
                if (opening == id) calls.error = AppModel.describe(failure)
                null
            }
            if (opening != id) return@launch
            opening = null
            if (opened != null) {
                ExamViews.documents[id] = opened
                show(id)
            }
        }
    }
}

@Composable
internal fun rememberExamOpener(calls: ExamCalls): ExamOpener {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val scope = rememberCoroutineScope()
    return remember(model, navigator, calls) { ExamOpener(model, scope, calls, navigator::openExamDocument) }
}

/** The page's last failure and notice, and a spinner while a call runs. */
@Composable
internal fun ExamStatus(calls: ExamCalls) {
    val account = LocalModel.current.examAccount
    // The unsupported status already says why the call failed.
    calls.error?.takeIf { account?.status != ExamAccountStatus.UNSUPPORTED }?.let { Text(it, color = Theme.colors.danger) }
    calls.notice?.let { Text(it, color = Theme.colors.muted) }
    if (calls.busy) CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp)
}

/** The account: who is signed in, what connecting needs, and an upload to
 * recover. [reload] reads (or changes) the account and reloads the page. */
@Composable
internal fun ExamAccountPanel(calls: ExamCalls, reload: suspend (ExamAction) -> Unit, uploaded: (String?) -> Unit) {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    val uri = LocalUriHandler.current
    val account = model.examAccount
    val connected = account?.status == ExamAccountStatus.CONNECTED
    var confirmDisconnect by remember { mutableStateOf(false) }
    Column {
        account?.email?.let { Text(it, color = Theme.colors.muted) }
        ExamStatus(calls)
        account?.upload?.takeIf { connected && it.phase == ExamUploadPhase.PENDING }?.let { upload ->
            ExamUploadProgress(upload, false)
            TextButton(enabled = !calls.busy, onClick = {
                scope.launch { (calls.call(ExamAction.RecoverUpload) as? ExamAnswer.Uploaded)?.let { uploaded(it.v1) } }
            }) { Text(t("examShare.upload.recover")) }
        }
        if (calls.connecting) TextButton(onClick = {
            scope.launch {
                try {
                    model.examShare(ExamAction.CancelConnect)
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (failure: Exception) {
                    calls.error = AppModel.describe(failure)
                }
            }
        }) { Text(t("common.cancel")) }
        when (account?.status) {
            ExamAccountStatus.CONNECTED -> TextButton(enabled = !calls.busy, onClick = { confirmDisconnect = true }) {
                Text(t("examShare.disconnect"))
            }
            // Terms and privacy are the service's own pages, which Core does not model.
            ExamAccountStatus.NEEDS_CONSENT -> Row {
                TextButton(onClick = { uri.openUri("https://sciencetokyoexam.com/terms") }) { Text(t("examShare.terms")) }
                TextButton(onClick = { uri.openUri("https://sciencetokyoexam.com/privacy") }) { Text(t("examShare.privacy")) }
                TextButton(enabled = !calls.busy, onClick = { scope.launch { reload(ExamAction.AcceptTerms) } }) { Text(t("examShare.accept")) }
            }
            ExamAccountStatus.UNSUPPORTED -> Text(t("error.exam-share-unsupported"))
            ExamAccountStatus.DISCONNECTED, null -> TextButton(enabled = !calls.busy, onClick = {
                scope.launch { reload(ExamAction.Connect) }
            }) { Text(t("examShare.connect")) }
        }
    }
    if (confirmDisconnect) ConfirmDialog(Confirmation("examShare.disconnectHint", ExamAction.Disconnect), { confirmDisconnect = false }) {
        scope.launch { reload(ExamAction.Disconnect) }
    }
}

@Composable
internal fun ConfirmDialog(confirmation: Confirmation, dismiss: () -> Unit, confirm: (ExamAction) -> Unit) {
    AlertDialog(onDismissRequest = dismiss, title = { Text(t(confirmation.key)) },
        confirmButton = { TextButton(onClick = { dismiss(); confirm(confirmation.action) }) { Text(t("common.ok")) } },
        dismissButton = { TextButton(onClick = dismiss) { Text(t("common.cancel")) } })
}

/** The past-exams tab: the account, subject search and this quarter's
 * courses, the request board and the library. Subjects, documents and
 * requests open as their own destinations. */
@Composable
fun PastExamsScreen(onBack: (() -> Unit)? = null) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val uri = LocalUriHandler.current
    val scope = rememberCoroutineScope()
    val calls = rememberExamCalls()
    val opener = rememberExamOpener(calls)
    val account = model.examAccount
    val connected = account?.status == ExamAccountStatus.CONNECTED
    var tab by rememberSaveable { mutableStateOf(Tab.Browse) }
    var query by rememberSaveable { mutableStateOf("") }
    var subjects by remember { mutableStateOf<List<ExamSubject>?>(null) }
    var requests by remember { mutableStateOf<ExamRequestPage?>(null) }
    var mine by rememberSaveable { mutableStateOf(false) }
    var library by remember { mutableStateOf<ExamLibrary?>(null) }
    var confirmation by remember { mutableStateOf<Confirmation?>(null) }
    val choices by rememberExamChoices(calls)

    // What a signed-out, consenting or unsupported account cannot show.
    LaunchedEffect(connected) {
        if (connected) return@LaunchedEffect
        confirmation = null
        subjects = null
        requests = null
        library = null
    }

    fun show(answer: ExamAnswer?) {
        when (answer) {
            is ExamAnswer.Subjects -> subjects = answer.v1
            is ExamAnswer.Library -> library = answer.v1
            is ExamAnswer.Requests -> {
                ExamViews.board(answer.v1)
                requests = answer.v1
            }
            else -> Unit
        }
    }

    suspend fun loadRequests(cursor: ExamRequestCursor? = null) {
        (calls.call(ExamAction.Requests(mine, cursor)) as? ExamAnswer.Requests)?.v1?.let {
            ExamViews.board(it)
            requests = it.copy(items = (if (cursor == null) emptyList() else requests?.items.orEmpty()) + it.items)
        }
    }

    /** Loads a tab's view: the board's first page or the library. Search
     * results stay as they were. */
    suspend fun load(view: Tab) {
        when (view) {
            Tab.Browse -> Unit
            Tab.Requests -> loadRequests()
            Tab.Library -> show(calls.call(ExamAction.Library))
        }
    }

    fun showTab(view: Tab) {
        tab = view
        calls.error = null
        val missing = when (view) {
            Tab.Browse -> false
            Tab.Requests -> requests == null
            Tab.Library -> library == null
        }
        if (connected && missing) scope.launch { load(view) }
    }

    /** The refresh rule: read the account (or change it with [action]) and,
     * when connected, reload the view on screen. */
    suspend fun reloadVisible(action: ExamAction = ExamAction.Account) {
        if ((calls.call(action) as? ExamAnswer.Account)?.v1?.status != ExamAccountStatus.CONNECTED) return
        load(tab)
    }

    fun uploaded(id: String?) {
        requests = null
        library = null
        calls.notice = t("examShare.upload.uploaded")
        if (id != null) navigator.openExamDocument(id) else scope.launch { reloadVisible() }
    }

    // Returning to the page (from a pushed one, or to the app) reloads it.
    LifecycleResumeEffect(Unit) {
        val refresh = scope.launch { reloadVisible() }
        onPauseOrDispose { refresh.cancel() }
    }

    TabPage(
        title = t("examShare.title"),
        navigationIcon = {
            if (onBack != null) IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Rounded.ArrowBack, t("common.back"))
            }
        },
    ) {
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(Theme.gutter), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            item { ExamAccountPanel(calls, { reloadVisible(it) }, ::uploaded) }
            if (account?.status == ExamAccountStatus.UNSUPPORTED) return@LazyColumn
            item {
                Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Tab.entries.forEach { choice ->
                        FilterChip(selected = tab == choice, enabled = !calls.busy, onClick = { showTab(choice) }, label = { Text(t(choice.key)) })
                    }
                }
            }
            if (!connected) {
                // Nothing to list until the account is connected.
            } else if (tab == Tab.Browse) {
                item {
                    OutlinedTextField(query, { query = it.take(choices?.limits?.query?.toInt() ?: it.length) }, label = { Text(t("examShare.search")) }, singleLine = true, modifier = Modifier.fillMaxWidth())
                    TextButton(enabled = !calls.busy && query.isNotBlank(), onClick = {
                        scope.launch { show(calls.call(ExamAction.Search(query))) }
                    }) { Text(t("examShare.search")) }
                }
                if (subjects?.isEmpty() == true) item { Text(t("examShare.noItems")) }
                items(subjects.orEmpty(), key = { it.code }) { hit ->
                    TextButton(onClick = { navigator.openExamSubject(hit.code) }) { Text("${hit.name} · ${hit.code}") }
                }
                item { CurrentCourses(enabled = true) { id -> navigator.openPastExams(id) } }
            } else if (tab == Tab.Requests) {
                item {
                    ChoiceRow("examShare.openRequests", "mine".takeIf { mine }, listOf("mine" to t("examShare.myRequests"))) {
                        mine = it != null; requests = null; scope.launch { loadRequests() }
                    }
                    TextButton(onClick = { showTab(Tab.Browse) }) { Text(t("examShare.newRequest")) }
                }
                if (requests?.items?.isEmpty() == true) item { Text(t("examShare.noItems")) }
                items(requests?.items.orEmpty(), key = { it.id }) { request ->
                    Card(Modifier.clickable { navigator.openExamRequest(request.id, mine) }) {
                        Column(Modifier.padding(12.dp)) {
                            Text(request.subject.name)
                            Text(requestFacts(request), color = Theme.colors.muted)
                        }
                    }
                }
                requests?.cursor?.let { cursor -> item {
                    TextButton(enabled = !calls.busy, onClick = { scope.launch { loadRequests(cursor) } }) { Text(t("examShare.more")) }
                } }
            } else {
                library?.let { content ->
                    val groups = if (content.offline) {
                        item { Text(t("examShare.libraryUnavailable"), color = Theme.colors.muted) }
                        listOf("examShare.downloaded" to content.downloaded)
                    } else {
                        listOf("examShare.saved" to content.saved, "examShare.downloaded" to content.downloaded, "examShare.submitted" to content.submitted)
                    }
                    item { TextButton(onClick = { uri.openUri(content.webUrl) }) { Text(t("examShare.openOnWeb")) } }
                    groups.forEach { (key, exams) ->
                        item { SectionLabel(t(key)); if (exams.isEmpty()) Text(t("examShare.noItems")) }
                        items(exams, key = { "$key:${it.id}" }) { exam ->
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                TextButton(modifier = Modifier.weight(1f), onClick = { opener.open(exam.id) }) { Text(examLabel(exam)) }
                                if (opener.opening == exam.id) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                                if (key == "examShare.downloaded" || key == "examShare.submitted") {
                                    TextButton(enabled = !calls.busy, onClick = {
                                        confirmation = if (key == "examShare.downloaded")
                                            Confirmation("examShare.removeDownload", ExamAction.RemoveDownload(exam.id))
                                        else Confirmation("examShare.deleteSubmission", ExamAction.DeleteSubmission(exam.id))
                                    }) { Text(t("common.delete")) }
                                }
                            }
                        }
                    }
                }
            }
            item { ExamCredit() }
        }
    }
    confirmation?.let {
        ConfirmDialog(it, { confirmation = null }) { action -> scope.launch { show(calls.call(action)) } }
    }
}

/** English only by design: it names the service and its operator. */
@Composable
internal fun ExamCredit() {
    Image(
        painterResource(R.drawable.past_exams_credit),
        "Supported by ScienceTokyoExam · teduryu",
        Modifier.fillMaxWidth().padding(top = 12.dp).height(11.dp),
        alignment = Alignment.CenterEnd,
        contentScale = ContentScale.Fit,
        colorFilter = ColorFilter.tint(Theme.colors.muted),
    )
}

internal fun requestFacts(request: ExamRequest): String = listOfNotNull(
    request.academicYear?.toString(),
    request.occasion?.let { t("examShare.occasion.$it") },
    request.wantedRole?.let { t("examShare.role.$it") },
    t("examShare.requestStatus.${request.status}"),
    request.description.ifEmpty { null },
).joinToString(" · ")

/** Filter chips: [empty] means no filter (null), then one chip per choice. */
@Composable
internal fun <T> ChoiceRow(empty: String, selected: T?, choices: List<Pair<T, String>>, pick: (T?) -> Unit) {
    Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        FilterChip(selected = selected == null, onClick = { pick(null) }, label = { Text(t(empty)) })
        choices.forEach { (value, label) ->
            FilterChip(selected = selected == value, onClick = { pick(value) }, label = { Text(label) })
        }
    }
}

@Composable
internal fun RequestDialog(subject: ExamSubjectPage, choices: ExamChoices, busy: Boolean, error: String?, dismiss: () -> Unit, submit: (ExamRequestDraft) -> Unit) {
    var classId by remember { mutableStateOf<UInt?>(null) }
    var year by remember { mutableStateOf("") }
    var occasion by remember { mutableStateOf<ExamOccasion?>(null) }
    var role by remember { mutableStateOf<ExamRole?>(null) }
    var description by remember { mutableStateOf("") }
    AlertDialog(onDismissRequest = dismiss, title = { Text(t("examShare.newRequest")) },
        text = {
            Column {
                Text(subject.subject.name)
                ChoiceRow("examShare.allClasses", classId, subject.rows.mapNotNull { row ->
                    row.classIds.firstOrNull()?.let { id -> id to (row.labels + row.teachers.map { it.name } + row.taughtIn).joinToString(" · ") }
                }) { classId = it }
                OutlinedTextField(year, { year = it.take(4) }, label = { Text(t("examShare.year")) }, singleLine = true)
                ChoiceRow("examShare.allOccasions", occasion, choices.requestOccasions.map { it to t("examShare.occasion.${it.wire}") }) { occasion = it }
                ChoiceRow("examShare.wantedRole", role, choices.requestRoles.map { it to t("examShare.role.${it.wire}") }) { role = it }
                OutlinedTextField(description, { description = it.take(choices.limits.requestDescription.toInt()) }, label = { Text(t("examShare.description")) })
                error?.let { Text(it, color = Theme.colors.danger) }
            }
        },
        confirmButton = { TextButton(enabled = !busy && (year.isBlank() || year.toUIntOrNull() != null), onClick = {
            submit(ExamRequestDraft(subject.subject.code, classId, year.toUIntOrNull(), occasion, role, description))
        }) { Text(t("examShare.newRequest")) } },
        dismissButton = { TextButton(onClick = dismiss) { Text(t("common.cancel")) } })
}

@Composable
internal fun ReportDialog(reasons: List<ExamReportReason>, detailLimit: Int, busy: Boolean, error: String?, dismiss: () -> Unit, submit: (ExamReportReason, String) -> Unit) {
    var reason by remember { mutableStateOf(reasons.first()) }
    var detail by remember { mutableStateOf("") }
    AlertDialog(onDismissRequest = dismiss, title = { Text(t("examShare.report")) },
        text = { Column {
            reasons.forEach { value ->
                FilterChip(selected = reason == value, onClick = { reason = value }, label = { Text(t("examShare.reason.${value.wire}")) })
            }
            OutlinedTextField(detail, { detail = it.take(detailLimit) }, label = { Text(t("examShare.description")) })
            error?.let { Text(it, color = Theme.colors.danger) }
        } },
        confirmButton = { TextButton(enabled = !busy, onClick = { submit(reason, detail) }) { Text(t("examShare.report")) } },
        dismissButton = { TextButton(onClick = dismiss) { Text(t("common.cancel")) } })
}

/** A sitting's name: year, exam type and round, variant. */
internal fun sittingLabel(year: UInt?, occasion: String?, number: UInt, variant: String): String = listOfNotNull(
    year?.toString(),
    occasion?.let {
        val label = t("examShare.occasion.$it")
        if (number > 1u) "$label $number" else label
    },
    variant,
).filter { it.isNotBlank() }.joinToString(" · ")

internal fun examLabel(exam: ExamDocument): String = listOfNotNull(
    exam.subjects.joinToString(" / ") { it.name },
    sittingLabel(exam.academicYear, exam.occasion, exam.occasionNumber, exam.variant),
    exam.role?.let { t("examShare.role.$it") },
    exam.teachers.joinToString(" / ") { it.name },
    exam.note,
).filter { it.isNotBlank() }.joinToString(" · ").ifEmpty { exam.fileName }
