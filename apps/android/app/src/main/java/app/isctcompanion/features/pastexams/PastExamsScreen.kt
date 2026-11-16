package app.isctcompanion.features.pastexams

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.ExamAccount
import app.isctcompanion.core.ExamAccountStatus
import app.isctcompanion.core.ExamAction
import app.isctcompanion.core.ExamAnswer
import app.isctcompanion.core.ExamCourseImportPreview
import app.isctcompanion.core.ExamDocument
import app.isctcompanion.core.ExamLibrary
import app.isctcompanion.core.ExamRequest
import app.isctcompanion.core.ExamRequestCursor
import app.isctcompanion.core.ExamRequestPage
import app.isctcompanion.core.ExamSubject
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
import org.json.JSONObject

/** Where a past-exams page leads: a course's sittings, a document or a request. */
sealed interface ExamPage {
    data class Subject(val code: String, val title: String, val instructor: String?) : ExamPage
    data class Document(val id: String) : ExamPage
    data class Request(val request: ExamRequest) : ExamPage
}

internal enum class ExamTab(val key: String, val path: String) {
    Browse("examShare.browse", "/search"),
    Requests("examShare.requests", "/request"),
    Library("examShare.library", "/library"),
}

internal const val EXAM_SITE = "https://sciencetokyoexam.com"

/** The account, the one Core call path and the page stack every past-exams
 * page shares, as iOS's `ExamShareStore`. */
@Stable
class ExamStore(private val model: AppModel) {
    var account by mutableStateOf<ExamAccount?>(null)
    var running by mutableStateOf(0)
    var problem by mutableStateOf<String?>(null)
    var notice by mutableStateOf<String?>(null)
    val stack = mutableStateListOf<ExamPage>()
    val busy: Boolean get() = running > 0
    val settings: JSONObject get() = model.records.settings

    fun push(page: ExamPage) { stack.add(page) }
    fun pop() { if (stack.isNotEmpty()) stack.removeAt(stack.lastIndex) }

    /** Runs one action. A failure shows its message, and a lost session
     * resets the pages through the account snapshot. */
    suspend fun call(action: ExamAction): ExamAnswer? {
        running++
        problem = null
        return try {
            model.examShare(action)
        } catch (_: SdkException.Cancelled) {
            null
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            problem = AppModel.describe(e)
            try {
                (model.examShare(ExamAction.AccountSnapshot) as? ExamAnswer.Account)?.v1?.let { account = it }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                // Keep the original failure if the session has already closed.
            }
            null
        } finally {
            running--
        }
    }

    suspend fun account(action: ExamAction) {
        (call(action) as? ExamAnswer.Account)?.v1?.let {
            account = it
            if (it.status != ExamAccountStatus.CONNECTED) stack.clear()
        }
    }

    suspend fun courses(): ExamCourseImportPreview? = model.examCourseImport(null)

    fun subjectName(subject: ExamSubject): String =
        Localizer.pick(mapOf("ja" to subject.name, "en" to subject.nameEn.orEmpty()), settings) ?: subject.code

    /** "2024 期末 2", the desktop's sitting label. */
    fun sitting(year: UInt?, occasion: String?, number: UInt, variant: String): String = listOfNotNull(
        year?.toString() ?: t("examShare.unknownYearShort"),
        occasion?.let { val label = t("examShare.occasion.$it"); if (number > 1u) "$label $number" else label },
        variant.ifBlank { null },
    ).joinToString(" ")

    fun title(document: ExamDocument): String = listOfNotNull(
        document.subjects.joinToString(" / ") { subjectName(it) }.ifBlank { null },
        sitting(document.academicYear, document.occasion, document.occasionNumber, document.variant),
        document.role?.let { t("examShare.role.$it") },
    ).joinToString(" · ")
}

internal val LocalExamStore = staticCompositionLocalOf<ExamStore> { error("No exam store") }

/**
 * ScienceTokyoExam, as the desktop and iOS show it: this quarter's courses
 * (or a search), requests and the person's own documents, each a list that
 * opens its course, request or document as a page of its own. Core decides
 * everything shown here.
 */
@Composable
fun PastExamsScreen() {
    val model = LocalModel.current
    val store = remember(model) { ExamStore(model) }
    var tab by rememberSaveable { mutableStateOf(ExamTab.Browse) }
    LaunchedEffect(store) { if (store.account == null) store.account(ExamAction.Account) }
    BackHandler(enabled = store.stack.isNotEmpty()) { store.pop() }

    androidx.compose.runtime.CompositionLocalProvider(LocalExamStore provides store) {
        Box(Modifier.fillMaxSize().background(Theme.colors.bg)) {
            AnimatedContent(
                targetState = store.stack.lastOrNull(),
                transitionSpec = {
                    val forward = targetState != null && (initialState == null || store.stack.size > 1)
                    if (forward) {
                        (slideInHorizontally { it / 3 } + fadeIn()) togetherWith (slideOutHorizontally { -it / 6 } + fadeOut())
                    } else {
                        (slideInHorizontally { -it / 6 } + fadeIn()) togetherWith (slideOutHorizontally { it / 3 } + fadeOut())
                    }
                },
                label = "exam-page",
            ) { page ->
                when (page) {
                    null -> Root(tab) { tab = it }
                    is ExamPage.Subject -> SubjectPage(page)
                    is ExamPage.Document -> DocumentPage(page)
                    is ExamPage.Request -> RequestPage(page)
                }
            }
            Feedback(Modifier.align(Alignment.BottomCenter))
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Root(tab: ExamTab, select: (ExamTab) -> Unit) {
    val store = LocalExamStore.current
    val uri = LocalUriHandler.current
    TabPage(
        t("examShare.title"),
        actions = {
            IconButton(onClick = { uri.openUri(EXAM_SITE + tab.path) }) {
                Icon(Icons.Rounded.Public, t("examShare.openInBrowser"))
            }
        },
    ) {
        when (store.account?.status) {
            ExamAccountStatus.CONNECTED -> Column(Modifier.fillMaxSize()) {
                SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(horizontal = Theme.gutter, vertical = 4.dp)) {
                    ExamTab.entries.forEachIndexed { index, choice ->
                        SegmentedButton(
                            selected = tab == choice,
                            onClick = { select(choice) },
                            shape = SegmentedButtonDefaults.itemShape(index, ExamTab.entries.size),
                            label = { Text(t(choice.key), maxLines = 1) },
                        )
                    }
                }
                when (tab) {
                    ExamTab.Browse -> BrowseList()
                    ExamTab.Requests -> RequestList()
                    ExamTab.Library -> LibraryList()
                }
            }
            null -> Box(Modifier.fillMaxSize(), Alignment.Center) { CircularProgressIndicator() }
            else -> AccountGate()
        }
    }
}

/** Before a usable session: sign in, the terms, or why not. */
@Composable
private fun AccountGate() {
    val store = LocalExamStore.current
    val scope = rememberCoroutineScope()
    val uri = LocalUriHandler.current
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(t("examShare.title"), style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold)
        when (store.account?.status) {
            ExamAccountStatus.NEEDS_CONSENT -> {
                Text(t("examShare.consentHint"), color = Theme.colors.muted)
                Button(enabled = !store.busy, onClick = { scope.launch { store.account(ExamAction.AcceptTerms) } }) {
                    Text(t("examShare.accept"))
                }
                Row {
                    TextButton(onClick = { uri.openUri("$EXAM_SITE/terms") }) { Text(t("examShare.terms")) }
                    TextButton(onClick = { uri.openUri("$EXAM_SITE/privacy") }) { Text(t("examShare.privacy")) }
                }
            }
            ExamAccountStatus.UNAVAILABLE -> Text(t("error.exam-share-unsupported"), color = Theme.colors.muted)
            else -> {
                Text(t(if (store.busy) "examShare.connectingHint" else "examShare.connectHint"), color = Theme.colors.muted)
                Button(enabled = !store.busy, onClick = { scope.launch { store.account(ExamAction.Connect) } }) {
                    Text(t("examShare.connect"))
                }
                // A sign-in waits on the browser; this gives up on it.
                if (store.busy) TextButton(onClick = {
                    scope.launch { store.call(ExamAction.CancelConnect) }
                }) { Text(t("common.cancel")) }
            }
        }
        Text(t("examShare.partner"), style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
    }
}

/** The shared notice or failure, briefly, above the page's foot. */
@Composable
private fun Feedback(modifier: Modifier) {
    val store = LocalExamStore.current
    val message = store.problem ?: store.notice ?: return
    LaunchedEffect(message) {
        delay(4_000)
        store.problem = null
        store.notice = null
    }
    Text(
        message,
        color = if (store.problem != null) Theme.colors.danger else Theme.colors.ink,
        style = MaterialTheme.typography.bodyMedium,
        modifier = modifier
            .padding(Theme.gutter)
            .fillMaxWidth()
            .background(Theme.colors.surface, RoundedCornerShape(Theme.radiusPanel))
            .clickable { store.problem = null; store.notice = null }
            .padding(horizontal = 14.dp, vertical = 10.dp),
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

/** This quarter's courses, or what a search finds; each opens its course. */
@Composable
private fun BrowseList() {
    val store = LocalExamStore.current
    val model = LocalModel.current
    var query by rememberSaveable { mutableStateOf("") }
    var hits by remember { mutableStateOf<Pair<String, List<ExamSubject>>?>(null) }
    var courses by remember { mutableStateOf<ExamCourseImportPreview?>(null) }
    LaunchedEffect(model.contentRevision) { courses = store.courses() }
    LaunchedEffect(query) {
        val trimmed = query.trim()
        if (trimmed.isEmpty()) return@LaunchedEffect
        delay(300)
        (store.call(ExamAction.Search(trimmed)) as? ExamAnswer.Subjects)?.v1?.let { hits = query to it }
    }
    val searching = query.isNotBlank()
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(Theme.gutter), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item {
            OutlinedTextField(
                query, { query = it.take(100) },
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
        if (searching) {
            val found = hits?.takeIf { it.first == query }?.second
            if (found != null) {
                if (found.isEmpty()) item { Text(t("examShare.noSubjects"), color = Theme.colors.muted) }
                else cardRows(found, { it.code }) { subject ->
                    ExamRow(
                        store.subjectName(subject),
                        listOfNotNull(subject.code, subject.examCount?.let { t("examShare.documentCount", "count" to it.toInt()) }).joinToString(" · "),
                        onClick = { store.push(ExamPage.Subject(subject.code, store.subjectName(subject), null)) },
                    )
                }
            }
        } else courses?.let { preview ->
            item { SectionLabel(t("examShare.myCourses")) }
            cardRows(preview.courses, { it.courseId }) { course ->
                val code = course.subjectCodes.firstOrNull()
                ExamRow(
                    course.title,
                    if (code == null) t("examShare.noSubjectCode")
                    else listOf(course.subjectCodes.joinToString(" / "), course.instructor).filter { it.isNotBlank() }.joinToString(" · "),
                    onClick = code?.let { { store.push(ExamPage.Subject(it, course.title, course.instructor.ifBlank { null })) } },
                )
            }
        }
        item { Text(t("examShare.partner"), style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted, modifier = Modifier.padding(top = 8.dp)) }
    }
}

/** Requests from everyone, or the person's own; each opens its detail. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun RequestList() {
    val store = LocalExamStore.current
    val scope = rememberCoroutineScope()
    var mine by rememberSaveable { mutableStateOf(false) }
    var pages by remember { mutableStateOf(mapOf<Boolean, ExamRequestPage>()) }
    suspend fun load(cursor: ExamRequestCursor? = null) {
        val forMine = mine
        val page = (store.call(ExamAction.Requests(forMine, cursor)) as? ExamAnswer.Requests)?.v1 ?: return
        val previous = if (cursor == null) emptyList() else pages[forMine]?.items.orEmpty()
        pages = pages + (forMine to page.copy(items = previous + page.items))
    }
    // Back from a request that changed: read the list again.
    LaunchedEffect(mine) { load() }
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
        if (page != null) {
            if (page.items.isEmpty()) item {
                Text(t(if (mine) "examShare.noMyRequests" else "examShare.noOpenRequests"), color = Theme.colors.muted)
            } else cardRows(page.items, { it.id }) { item ->
                ExamRow(
                    store.subjectName(item.subject),
                    listOf(requestFacts(item), item.description).filter { it.isNotBlank() }.joinToString(" · "),
                    onClick = { store.push(ExamPage.Request(item)) },
                    trailing = if (item.status != "open") ({ StatusTag(item.status) }) else null,
                )
            }
            page.cursor?.let { cursor ->
                item { TextButton(onClick = { scope.launch { load(cursor) } }) { Text(t("examShare.more")) } }
            }
        }
    }
}

internal fun requestFacts(item: ExamRequest): String = listOfNotNull(
    item.academicYear?.toString(),
    item.occasion?.let { t("examShare.occasion.$it") },
    item.wantedRole?.let { t("examShare.role.$it") },
).joinToString(" · ")

@Composable
internal fun StatusTag(status: String) {
    val (ink, soft) = when (status) {
        "fulfilled" -> Theme.colors.ok to Theme.colors.okSoft
        "open" -> Theme.colors.accent to Theme.colors.accentSoft
        else -> Theme.colors.muted to Theme.colors.surface2
    }
    Tag(t("examShare.requestStatus.$status"), color = ink, soft = soft)
}

/** Saved, downloaded and shared documents; the account at the foot. */
@Composable
private fun LibraryList() {
    val store = LocalExamStore.current
    val scope = rememberCoroutineScope()
    var library by remember { mutableStateOf<ExamLibrary?>(null) }
    var confirm by remember { mutableStateOf<Pair<String, ExamAction>?>(null) }
    suspend fun load() { (store.call(ExamAction.Library) as? ExamAnswer.Library)?.v1?.let { library = it } }
    LaunchedEffect(Unit) { load() }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(Theme.gutter), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        library?.let { content ->
            val groups = buildList {
                if (content.offline) add(Triple("examShare.downloaded", content.downloaded, "examShare.removeDownload"))
                else {
                    add(Triple("examShare.saved", content.saved, null))
                    add(Triple("examShare.downloaded", content.downloaded, "examShare.removeDownload"))
                    add(Triple("examShare.submitted", content.submitted, "examShare.deleteSubmission"))
                }
            }
            if (content.offline) item { Text(t("examShare.libraryUnavailable"), color = Theme.colors.muted) }
            groups.filter { it.second.isNotEmpty() }.forEach { (title, items, removal) ->
                item { SectionLabel(t(title)) }
                cardRows(items, { "$title:${it.id}" }) { document ->
                    ExamRow(
                        document.subjects.joinToString(" / ") { store.subjectName(it) },
                        listOfNotNull(
                            store.sitting(document.academicYear, document.occasion, document.occasionNumber, document.variant),
                            document.role?.let { t("examShare.role.$it") },
                        ).joinToString(" · "),
                        onClick = { store.push(ExamPage.Document(document.id)) },
                        trailing = removal?.let {
                            {
                                TextButton(onClick = {
                                    confirm = it to if (it == "examShare.removeDownload") ExamAction.RemoveDownload(document.id)
                                    else ExamAction.DeleteSubmission(document.id)
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
                store.account?.email?.let { ExamRow(it, null, onClick = null) }
                TextButton(onClick = { confirm = "examShare.disconnectHint" to ExamAction.Disconnect }, modifier = Modifier.padding(horizontal = 4.dp)) {
                    Text(t("examShare.disconnect"), color = Theme.colors.danger)
                }
            }
            Text(t("examShare.partner"), style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted, modifier = Modifier.padding(top = 8.dp))
        }
    }
    confirm?.let { (key, action) ->
        AlertDialog(
            onDismissRequest = { confirm = null },
            title = { Text(t(key)) },
            confirmButton = {
                TextButton(onClick = {
                    confirm = null
                    scope.launch {
                        if (action == ExamAction.Disconnect) store.account(action)
                        else if (store.call(action) != null) load()
                    }
                }) { Text(t("common.ok"), color = Theme.colors.danger) }
            },
            dismissButton = { TextButton(onClick = { confirm = null }) { Text(t("common.cancel")) } },
        )
    }
}
