package app.isctcompanion.features.pastexams

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Clear
import androidx.compose.material.icons.rounded.Public
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
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
import app.isctcompanion.core.ExamOccasion
import app.isctcompanion.core.ExamRequest
import app.isctcompanion.core.ExamRequestCursor
import app.isctcompanion.core.ExamRequestPage
import app.isctcompanion.core.ExamSubject
import app.isctcompanion.core.ExamUploadPhase
import app.isctcompanion.core.SdkException
import app.isctcompanion.design.Card
import app.isctcompanion.design.Localizer
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.Tag
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

private enum class Tab(val key: String, val path: String) {
    Browse("examShare.browse", "/search"),
    Requests("examShare.requests", "/request"),
    Library("examShare.library", "/library"),
}

/** The service's own pages Core does not answer a link for (terms, privacy,
 * search), and where a tab leads before Core has named its page. */
internal const val EXAM_SITE = "https://sciencetokyoexam.com"

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
    /** Requests listed on a board, read by the request page. */
    val requests = mutableMapOf<String, ExamRequest>()
    /** The board's and the library's pages on the site, as Core last named them. */
    var requestBoardUrl: String? = null
    var libraryUrl: String? = null
    /** The last thing done (posted, reported, uploaded), shown briefly on
     * whichever page is in front. */
    var notice by mutableStateOf<String?>(null)

    fun board(page: ExamRequestPage) {
        page.items.forEach { requests[it.id] = it }
        requestBoardUrl = page.webUrl
    }
}

/** One page's calls into Core; a failure shows on the page. */
@Stable
internal class ExamCalls(private val model: AppModel) {
    private var running by mutableIntStateOf(0)
    val busy: Boolean get() = running > 0
    var connecting by mutableStateOf(false)
        private set
    var error by mutableStateOf<String?>(null)

    suspend fun call(action: ExamAction): ExamAnswer? {
        running++
        if (action == ExamAction.Connect) connecting = true
        error = null
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
            if (action == ExamAction.Connect) connecting = false
            running--
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

/** A past-exams page with its failure or notice floating above the foot. */
@Composable
internal fun ExamFrame(calls: ExamCalls, content: @Composable () -> Unit) {
    Box(Modifier.fillMaxSize().background(Theme.colors.bg)) {
        content()
        Feedback(calls, Modifier.align(Alignment.BottomCenter))
    }
}

/** The page's failure or the shared notice, briefly, above the page's foot. */
@Composable
private fun Feedback(calls: ExamCalls, modifier: Modifier) {
    val account = LocalModel.current.examAccount
    // The unsupported status already says why the call failed.
    val problem = calls.error?.takeIf { account?.status != ExamAccountStatus.UNSUPPORTED }
    val message = problem ?: ExamViews.notice ?: return
    LaunchedEffect(message) {
        delay(4_000)
        calls.error = null
        ExamViews.notice = null
    }
    Text(
        message,
        color = if (problem != null) Theme.colors.danger else Theme.colors.ink,
        style = MaterialTheme.typography.bodyMedium,
        modifier = modifier
            .padding(Theme.gutter)
            .fillMaxWidth()
            .background(Theme.colors.surface, RoundedCornerShape(Theme.radiusPanel))
            .clickable { calls.error = null; ExamViews.notice = null }
            .padding(horizontal = 14.dp, vertical = 10.dp),
    )
}

@Composable
internal fun ConfirmDialog(confirmation: Confirmation, dismiss: () -> Unit, confirm: (ExamAction) -> Unit) {
    AlertDialog(
        onDismissRequest = dismiss,
        title = { Text(t(confirmation.key)) },
        confirmButton = {
            TextButton(onClick = { dismiss(); confirm(confirmation.action) }) { Text(t("common.ok"), color = Theme.colors.danger) }
        },
        dismissButton = { TextButton(onClick = dismiss) { Text(t("common.cancel")) } },
    )
}

/**
 * ScienceTokyoExam: this quarter's courses (or a search), requests and the
 * person's own documents, each a list that opens its course, request or
 * document as a destination of its own. Core decides everything shown here.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PastExamsScreen(onBack: (() -> Unit)? = null) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val uri = LocalUriHandler.current
    val links = rememberExamLinks()
    val scope = rememberCoroutineScope()
    val calls = rememberExamCalls()
    val choices by rememberExamChoices(calls)
    val account = model.examAccount
    var tab by rememberSaveable { mutableStateOf(Tab.Browse) }

    /** Reads the account, or changes it with [action]; the lists reload themselves. */
    suspend fun reload(action: ExamAction = ExamAction.Account) {
        calls.call(action)
    }

    // Returning to the page (from a pushed one, or to the app) reads the account again.
    LifecycleResumeEffect(Unit) {
        val refresh = scope.launch { reload() }
        onPauseOrDispose { refresh.cancel() }
    }

    ExamFrame(calls) {
        TabPage(
            t("examShare.title"),
            navigationIcon = {
                if (onBack != null) IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, t("common.back")) }
            },
            actions = {
                ExamLinkButton(links) {
                    when (tab) {
                        Tab.Browse -> null
                        Tab.Requests -> ExamViews.requestBoardUrl
                        Tab.Library -> ExamViews.libraryUrl
                    } ?: (EXAM_SITE + tab.path)
                }
            },
        ) {
            when (account?.status) {
                ExamAccountStatus.CONNECTED -> Column(Modifier.fillMaxSize()) {
                    // An upload the site may have half-taken waits for recovery.
                    account.upload.takeIf { it.phase == ExamUploadPhase.PENDING }?.let { upload ->
                        Card(Modifier.padding(horizontal = Theme.gutter, vertical = 4.dp)) {
                            Row(
                                Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Box(Modifier.weight(1f)) { ExamUploadProgress(upload, false) }
                                TextButton(enabled = !calls.busy, onClick = {
                                    scope.launch {
                                        (calls.call(ExamAction.RecoverUpload) as? ExamAnswer.Uploaded)?.let { answer ->
                                            ExamViews.notice = t("examShare.upload.uploaded")
                                            answer.v1?.let(navigator::openExamDocument)
                                        }
                                    }
                                }) { Text(t("examShare.upload.recover")) }
                            }
                        }
                    }
                    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(horizontal = Theme.gutter, vertical = 4.dp)) {
                        Tab.entries.forEachIndexed { index, choice ->
                            SegmentedButton(
                                selected = tab == choice,
                                onClick = { tab = choice; calls.error = null },
                                shape = SegmentedButtonDefaults.itemShape(index, Tab.entries.size),
                                label = { Text(t(choice.key), maxLines = 1) },
                            )
                        }
                    }
                    when (tab) {
                        Tab.Browse -> BrowseList(calls, choices)
                        Tab.Requests -> RequestList(calls)
                        Tab.Library -> LibraryList(calls) { reload(it) }
                    }
                }
                null -> Box(Modifier.fillMaxSize(), Alignment.Center) { CircularProgressIndicator() }
                else -> AccountGate(calls) { reload(it) }
            }
        }
    }
}

/** Before a usable session: sign in, the terms, or why not. */
@Composable
internal fun AccountGate(calls: ExamCalls, reload: suspend (ExamAction) -> Unit) {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    val uri = LocalUriHandler.current
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(t("examShare.title"), style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold)
        when (model.examAccount?.status) {
            ExamAccountStatus.NEEDS_CONSENT -> {
                Text(t("error.exam-share-consent"), color = Theme.colors.muted)
                Button(enabled = !calls.busy, onClick = { scope.launch { reload(ExamAction.AcceptTerms) } }) {
                    Text(t("examShare.accept"))
                }
                // Terms and privacy are the service's own pages, which Core does not model.
                Row {
                    TextButton(onClick = { uri.openUri("$EXAM_SITE/terms") }) { Text(t("examShare.terms")) }
                    TextButton(onClick = { uri.openUri("$EXAM_SITE/privacy") }) { Text(t("examShare.privacy")) }
                }
            }
            ExamAccountStatus.UNSUPPORTED -> Text(t("error.exam-share-unsupported"), color = Theme.colors.muted)
            else -> {
                Text(t(if (calls.connecting) "common.signingIn" else "error.exam-share-signed-out"), color = Theme.colors.muted)
                Button(enabled = !calls.busy, onClick = { scope.launch { reload(ExamAction.Connect) } }) {
                    Text(t("examShare.connect"))
                }
                // A sign-in waits on the browser; this gives up on it.
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
            }
        }
        ExamCredit()
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

/** A tappable row: title, muted line, and what sits at its end. */
@Composable
internal fun ExamRow(
    title: String,
    meta: String?,
    onClick: (() -> Unit)?,
    leading: @Composable (() -> Unit)? = null,
    trailing: @Composable (() -> Unit)? = null,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
            .padding(horizontal = 16.dp, vertical = 11.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        leading?.invoke()
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(title, color = Theme.colors.ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
            if (!meta.isNullOrBlank()) {
                Text(meta, style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted, maxLines = 2, overflow = TextOverflow.Ellipsis)
            }
        }
        trailing?.invoke()
    }
}

/** Rows in one card, divided by hairlines. */
internal fun <T> LazyListScope.cardRows(items: List<T>, key: (T) -> Any, row: @Composable (T) -> Unit) {
    item(key = "card-${items.firstOrNull()?.let(key)}") {
        Card {
            items.forEachIndexed { index, item ->
                if (index > 0) HorizontalDivider(color = Theme.colors.line, modifier = Modifier.padding(start = 16.dp))
                row(item)
            }
        }
    }
}

@Composable
internal fun StatusTag(status: String) {
    val (ink, soft) = when (status) {
        "fulfilled" -> Theme.colors.ok to Theme.colors.okSoft
        "open" -> Theme.colors.accent to Theme.colors.accentSoft
        else -> Theme.colors.muted to Theme.colors.surface2
    }
    Tag(t("examShare.requestStatus.$status"), color = ink, soft = soft)
}

/** A subject's name in the app's language: English when the site has one. */
internal fun subjectName(subject: ExamSubject): String =
    subject.nameEn?.takeIf { Localizer.language == "en" && it.isNotBlank() } ?: subject.name.ifBlank { subject.code }

/** "2024 期末 2", the desktop's sitting label. */
internal fun sittingLabel(year: UInt?, occasion: String?, number: UInt, variant: String): String = listOfNotNull(
    year?.toString(),
    occasion?.let { val label = t("examShare.occasion.$it"); if (number > 1u) "$label $number" else label },
    variant.ifBlank { null },
).joinToString(" ")

internal fun documentTitle(document: ExamDocument): String = listOfNotNull(
    document.subjects.joinToString(" / ") { subjectName(it) }.ifBlank { null },
    sittingLabel(document.academicYear, document.occasion, document.occasionNumber, document.variant).ifBlank { null },
    document.role?.let { t("examShare.role.$it") },
).joinToString(" · ").ifEmpty { document.fileName }

internal fun requestFacts(item: ExamRequest): String = listOfNotNull(
    item.academicYear?.toString(),
    item.occasion?.let { t("examShare.occasion.$it") },
    item.wantedRole?.let { t("examShare.role.$it") },
).joinToString(" · ")

/** This quarter's courses, or what a search finds; each opens its course. */
@Composable
private fun BrowseList(calls: ExamCalls, choices: ExamChoices?) {
    val navigator = LocalNavigator.current
    var query by rememberSaveable { mutableStateOf("") }
    var hits by remember { mutableStateOf<Pair<String, List<ExamSubject>>?>(null) }
    val courses = rememberCurrentCourses()
    LaunchedEffect(query) {
        val trimmed = query.trim()
        if (trimmed.isEmpty()) return@LaunchedEffect
        delay(300)
        (calls.call(ExamAction.Search(trimmed)) as? ExamAnswer.Subjects)?.v1?.let { hits = query to it }
    }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(Theme.gutter), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item {
            OutlinedTextField(
                query, { query = it.take(choices?.limits?.query?.toInt() ?: 100) },
                placeholder = { Text(t("examShare.search")) },
                leadingIcon = { Icon(Icons.Rounded.Search, null) },
                trailingIcon = {
                    if (query.isNotEmpty()) IconButton(onClick = { query = "" }) { Icon(Icons.Rounded.Clear, t("common.clear")) }
                },
                singleLine = true,
                shape = RoundedCornerShape(Theme.radiusControl),
                modifier = Modifier.fillMaxWidth(),
            )
        }
        if (query.isNotBlank()) {
            val found = hits?.takeIf { it.first == query }?.second
            if (found != null) {
                if (found.isEmpty()) item { Text(t("examShare.noItems"), color = Theme.colors.muted) }
                else cardRows(found, { it.code }) { subject ->
                    ExamRow(subjectName(subject), subject.code, onClick = { navigator.openExamSubject(subject.code) })
                }
            }
        } else {
            currentCourses(courses) { navigator.openPastExams(it) }
        }
        item { ExamCredit() }
    }
}

/** Requests from everyone, or the person's own; each opens its detail. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun RequestList(calls: ExamCalls) {
    val navigator = LocalNavigator.current
    val scope = rememberCoroutineScope()
    var mine by rememberSaveable { mutableStateOf(false) }
    var pages by remember { mutableStateOf(mapOf<Boolean, ExamRequestPage>()) }
    suspend fun load(cursor: ExamRequestCursor? = null) {
        val forMine = mine
        val page = (calls.call(ExamAction.Requests(forMine, cursor)) as? ExamAnswer.Requests)?.v1 ?: return
        ExamViews.board(page)
        val previous = if (cursor == null) emptyList() else pages[forMine]?.items.orEmpty()
        pages = pages + (forMine to page.copy(items = previous + page.items))
    }
    // Shown, or back from a request that changed: read the list again.
    LifecycleResumeEffect(mine) {
        val refresh = scope.launch { load() }
        onPauseOrDispose { refresh.cancel() }
    }
    val page = pages[mine]
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(Theme.gutter), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item {
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                listOf(false to "examShare.openRequests", true to "examShare.myRequests").forEachIndexed { index, (value, key) ->
                    SegmentedButton(
                        selected = mine == value, onClick = { mine = value },
                        shape = SegmentedButtonDefaults.itemShape(index, 2),
                        label = { Text(t(key)) },
                    )
                }
            }
        }
        if (page == null) {
            if (calls.busy) item { Box(Modifier.fillMaxWidth().padding(24.dp), Alignment.Center) { CircularProgressIndicator() } }
        } else {
            if (page.items.isEmpty()) item {
                Text(t("examShare.noItems"), color = Theme.colors.muted)
            } else cardRows(page.items, { it.id }) { item ->
                ExamRow(
                    subjectName(item.subject),
                    listOf(requestFacts(item), item.description).filter { it.isNotBlank() }.joinToString(" · "),
                    onClick = { navigator.openExamRequest(item.id, mine) },
                    trailing = if (item.status != "open") ({ StatusTag(item.status) }) else null,
                )
            }
            page.cursor?.let { cursor ->
                item { TextButton(enabled = !calls.busy, onClick = { scope.launch { load(cursor) } }) { Text(t("examShare.more")) } }
            }
        }
    }
}

/** Saved, downloaded and shared documents; the account at the foot. */
@Composable
private fun LibraryList(calls: ExamCalls, reload: suspend (ExamAction) -> Unit) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val scope = rememberCoroutineScope()
    var library by remember { mutableStateOf<ExamLibrary?>(null) }
    var confirm by remember { mutableStateOf<Confirmation?>(null) }
    fun show(content: ExamLibrary) {
        library = content
        ExamViews.libraryUrl = content.webUrl
    }
    suspend fun load() { (calls.call(ExamAction.Library) as? ExamAnswer.Library)?.v1?.let(::show) }
    LifecycleResumeEffect(Unit) {
        val refresh = scope.launch { load() }
        onPauseOrDispose { refresh.cancel() }
    }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(Theme.gutter), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        val content = library
        if (content == null) {
            if (calls.busy) item { Box(Modifier.fillMaxWidth().padding(24.dp), Alignment.Center) { CircularProgressIndicator() } }
        } else {
            val groups = buildList {
                if (!content.offline) add(Triple("examShare.saved", content.saved, null))
                add(Triple("examShare.downloaded", content.downloaded, "examShare.removeDownload"))
                if (!content.offline) add(Triple("examShare.submitted", content.submitted, "examShare.deleteSubmission"))
            }
            if (content.offline) item { Text(t("examShare.libraryUnavailable"), color = Theme.colors.muted) }
            if (groups.all { it.second.isEmpty() }) item { Text(t("examShare.noItems"), color = Theme.colors.muted) }
            groups.filter { it.second.isNotEmpty() }.forEach { (title, items, removal) ->
                item(key = "label:$title") { SectionLabel(t(title)) }
                cardRows(items, { "$title:${it.id}" }) { document ->
                    ExamRow(
                        document.subjects.joinToString(" / ") { subjectName(it) }.ifBlank { document.fileName },
                        listOfNotNull(
                            sittingLabel(document.academicYear, document.occasion, document.occasionNumber, document.variant).ifBlank { null },
                            document.role?.let { t("examShare.role.$it") },
                        ).joinToString(" · "),
                        onClick = { navigator.openExamDocument(document.id) },
                        trailing = removal?.let { key ->
                            {
                                TextButton(enabled = !calls.busy, onClick = {
                                    confirm = Confirmation(
                                        key,
                                        if (key == "examShare.removeDownload") ExamAction.RemoveDownload(document.id)
                                        else ExamAction.DeleteSubmission(document.id),
                                    )
                                }) { Text(t("common.delete"), color = Theme.colors.danger) }
                            }
                        },
                    )
                }
            }
        }
        item { SectionLabel(t("examShare.account")) }
        item {
            Card {
                model.examAccount?.email?.let { ExamRow(it, null, onClick = null) }
                TextButton(
                    enabled = !calls.busy,
                    onClick = { confirm = Confirmation("examShare.disconnectHint", ExamAction.Disconnect) },
                    modifier = Modifier.padding(horizontal = 4.dp),
                ) { Text(t("examShare.disconnect"), color = Theme.colors.danger) }
            }
            ExamCredit()
        }
    }
    confirm?.let {
        ConfirmDialog(it, { confirm = null }) { action ->
            scope.launch {
                if (action == ExamAction.Disconnect) reload(action)
                else when (val answer = calls.call(action)) {
                    is ExamAnswer.Library -> show(answer.v1)
                    null -> Unit
                    else -> load()
                }
            }
        }
    }
}
