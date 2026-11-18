package app.isctcompanion.features.pastexams

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Color
import android.graphics.pdf.PdfRenderer
import android.net.Uri
import android.os.ParcelFileDescriptor
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.ExpandLess
import androidx.compose.material.icons.rounded.ExpandMore
import androidx.compose.material.icons.rounded.MoreVert
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.pdf.ExperimentalPdfApi
import androidx.pdf.PdfDocument
import androidx.pdf.SandboxedPdfLoader
import androidx.pdf.compose.PdfViewer
import androidx.pdf.compose.rememberPdfViewerState
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.ExamAccountStatus
import app.isctcompanion.core.ExamAction
import app.isctcompanion.core.ExamAnswer
import app.isctcompanion.core.ExamLocalDocument
import app.isctcompanion.core.ExamRequest
import app.isctcompanion.core.ExamSet
import app.isctcompanion.core.ExamSubjectPage
import app.isctcompanion.core.ExamUploadForm
import app.isctcompanion.core.ExamUploadStart
import app.isctcompanion.design.Card
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** How a subject page was reached: from a course (Core finds its subject and
 * class) or by a subject code. */
sealed interface ExamSubjectEntry {
    data class Course(val courseId: String) : ExamSubjectEntry
    data class Code(val code: String, val classId: UInt?) : ExamSubjectEntry
}

@Composable
private fun BackButton() {
    val navigator = LocalNavigator.current
    IconButton(onClick = { navigator.back() }) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, t("common.back")) }
}

@Composable
private fun Loading() {
    Box(Modifier.fillMaxSize(), Alignment.Center) { CircularProgressIndicator() }
}

/** One course's sittings as the desktop's panel shows them: the person's own
 * teachers first, the others folded away, a button per document. */
@Composable
fun ExamSubjectScreen(entry: ExamSubjectEntry) {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    val links = rememberExamLinks()
    val calls = rememberExamCalls()
    val choices by rememberExamChoices(calls)
    val account = model.examAccount
    val connected = account?.status == ExamAccountStatus.CONNECTED
    var page by remember { mutableStateOf<ExamSubjectPage?>(null) }
    var othersOpen by rememberSaveable { mutableStateOf(false) }
    var menu by remember { mutableStateOf(false) }
    var requesting by rememberSaveable { mutableStateOf(false) }
    var uploadForm by remember { mutableStateOf<ExamUploadForm?>(null) }

    /** The subject again once shown; a course entry resolves it the first time. */
    suspend fun load() {
        val action = page?.let { ExamAction.Subject(it.subject.code, it.classId) } ?: when (entry) {
            is ExamSubjectEntry.Course -> ExamAction.CourseSubject(entry.courseId)
            is ExamSubjectEntry.Code -> ExamAction.Subject(entry.code, entry.classId)
        }
        (calls.call(action) as? ExamAnswer.Subject)?.v1?.let { page = it }
    }

    suspend fun reload(action: ExamAction = ExamAction.Account) {
        if ((calls.call(action) as? ExamAnswer.Account)?.v1?.status == ExamAccountStatus.CONNECTED) load()
    }

    // Returning to the page (from a document, or to the app) reloads it.
    LifecycleResumeEffect(entry) {
        val refresh = scope.launch { reload() }
        onPauseOrDispose { refresh.cancel() }
    }

    val current = page
    ExamFrame(calls) {
        TabPage(
            current?.let { it.subject.name }.orEmpty(),
            navigationIcon = { BackButton() },
            actions = {
                if (current != null && connected) Box {
                    IconButton(onClick = { menu = true }) { Icon(Icons.Rounded.Add, t("examShare.upload.title")) }
                    DropdownMenu(menu, { menu = false }) {
                        DropdownMenuItem({ Text(t("examShare.upload.title")) }, enabled = !calls.busy && choices != null, onClick = {
                            menu = false
                            scope.launch {
                                uploadForm = (calls.call(ExamAction.UploadForm(ExamUploadStart.Subject(current.subject.code))) as? ExamAnswer.UploadForm)?.v1
                            }
                        })
                        DropdownMenuItem({ Text(t("examShare.newRequest")) }, enabled = choices != null, onClick = { menu = false; requesting = true })
                        DropdownMenuItem({ Text(t("examShare.openOnWeb")) }, onClick = { menu = false; links.open(current.webUrl) })
                    }
                }
            },
        ) {
            when {
                account == null -> Loading()
                !connected -> AccountGate(calls) { reload(it) }
                current != null -> {
                    // A class of one's own puts its teachers' sittings first.
                    val grouped = current.classId != null
                    val mine = current.sets.filter { !grouped || it.mine }
                    val others = if (grouped) current.sets.filter { !it.mine } else emptyList()
                    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(Theme.gutter), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        if (current.sets.isEmpty()) {
                            item { Text(t("examShare.noItems"), color = Theme.colors.muted) }
                        } else {
                            if (mine.isEmpty()) item { Text(t("examShare.noOwnTeacher"), color = Theme.colors.muted) }
                            else cardRows(mine, { it.documents.first().id }) { SetRow(it, withTeachers = !it.mine) }
                            if (others.isNotEmpty()) {
                                item(key = "others") {
                                    Row(
                                        Modifier.fillMaxWidth().clickable { othersOpen = !othersOpen }.padding(vertical = 8.dp),
                                        verticalAlignment = Alignment.CenterVertically,
                                        horizontalArrangement = Arrangement.spacedBy(4.dp),
                                    ) {
                                        Icon(if (othersOpen) Icons.Rounded.ExpandLess else Icons.Rounded.ExpandMore, null, tint = Theme.colors.muted)
                                        Text(t("examShare.otherTeachers", "count" to others.size), color = Theme.colors.muted)
                                    }
                                }
                                if (othersOpen) cardRows(others, { "other:" + it.documents.first().id }) { SetRow(it, withTeachers = true) }
                            }
                        }
                        item { ExamCredit() }
                    }
                }
                calls.busy -> Loading()
                else -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                    TextButton(onClick = { scope.launch { load() } }) { Text(t("common.retry")) }
                }
            }
        }
    }
    val form = uploadForm
    val formChoices = choices
    if (form != null && formChoices != null) {
        UploadSheet(form, formChoices, dismiss = { uploadForm = null }) {
            uploadForm = null
            ExamViews.notice = t("examShare.upload.uploaded")
            scope.launch { load() }
        }
    }
    if (requesting && current != null && formChoices != null) {
        RequestDialog(current, formChoices, calls.busy, calls.error, dismiss = { requesting = false }) { draft ->
            scope.launch {
                if (calls.call(ExamAction.CreateRequest(draft)) != null) {
                    requesting = false
                    ExamViews.notice = t("examShare.requestCreated")
                }
            }
        }
    }
}

/** One sitting: a label, and a button per document (question, answer) — the
 * buttons are what opens, as on the desktop and iOS. */
@Composable
private fun SetRow(set: ExamSet, withTeachers: Boolean) {
    val navigator = LocalNavigator.current
    ExamRow(
        sittingLabel(set.academicYear, set.occasion, set.occasionNumber, set.variant).ifBlank { set.documents.first().fileName },
        if (withTeachers) set.teachers.joinToString(" / ") { it.name } else null,
        onClick = null,
        leading = { Icon(Icons.Rounded.Description, null, tint = Theme.colors.accent, modifier = Modifier.size(20.dp)) },
        trailing = {
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                set.documents.forEach { document ->
                    val answer = document.role == "answer"
                    val ink = if (answer) Theme.colors.ok else Theme.colors.accent
                    Text(
                        // Core numbers several documents of one role in a sitting.
                        listOfNotNull(
                            document.role?.let { t("examShare.role.$it") } ?: document.fileName,
                            document.number?.toString(),
                        ).joinToString(" "),
                        color = ink,
                        style = MaterialTheme.typography.labelLarge,
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 1,
                        modifier = Modifier
                            .background(if (answer) Theme.colors.okSoft else Theme.colors.accentSoft, CircleShape)
                            .border(1.dp, ink.copy(alpha = 0.35f), CircleShape)
                            .clickable { navigator.openExamDocument(document.id) }
                            .padding(horizontal = 12.dp, vertical = 5.dp),
                    )
                }
            }
        },
    )
}

/** A past exam full screen: PDFs in androidx.pdf (zoom, select and search
 * built in), photos zoomable, text as text; actions in the menu. */
@Composable
fun ExamDocumentScreen(id: String) {
    val scope = rememberCoroutineScope()
    val links = rememberExamLinks()
    val calls = rememberExamCalls()
    val choices by rememberExamChoices(calls)
    var file by remember(id) { mutableStateOf<ExamLocalDocument?>(null) }
    var menu by remember { mutableStateOf(false) }
    var reporting by rememberSaveable { mutableStateOf(false) }

    suspend fun open(download: Boolean = false) {
        (calls.call(ExamAction.Open(id, download)) as? ExamAnswer.Document)?.v1?.let {
            file = it
            if (download) ExamViews.notice = t("examShare.downloaded")
        }
    }
    LaunchedEffect(id) { open() }

    val current = file
    ExamFrame(calls) {
        TabPage(
            current?.let { documentTitle(it.document) }.orEmpty(),
            navigationIcon = { BackButton() },
            actions = {
                if (current != null) Box {
                    IconButton(onClick = { menu = true }) { Icon(Icons.Rounded.MoreVert, t("examShare.more")) }
                    DropdownMenu(menu, { menu = false }) {
                        DropdownMenuItem(
                            { Text(t(if (current.downloaded) "examShare.downloaded" else "examShare.download")) },
                            { menu = false; scope.launch { open(download = true) } },
                            enabled = !calls.busy && !current.downloaded && !current.offline,
                        )
                        current.saved?.let { saved ->
                            DropdownMenuItem(
                                { Text(t(if (saved) "examShare.unsave" else "examShare.save")) },
                                {
                                    menu = false
                                    scope.launch {
                                        (calls.call(ExamAction.Save(id, !saved)) as? ExamAnswer.Saved)?.v1?.let {
                                            file = current.copy(saved = it)
                                        }
                                    }
                                },
                                enabled = !calls.busy && !current.offline,
                            )
                        }
                        DropdownMenuItem({ Text(t("examShare.openOnWeb")) }, { menu = false; links.open(current.webUrl) })
                        DropdownMenuItem(
                            { Text(t("examShare.report"), color = Theme.colors.danger) },
                            { menu = false; reporting = true },
                            enabled = !calls.busy && !current.offline && choices != null,
                        )
                    }
                }
            },
        ) {
            when {
                current != null -> Column(Modifier.fillMaxSize()) {
                    // A downloaded copy shown without the site.
                    if (current.offline) Text(
                        t("examShare.offline"),
                        style = MaterialTheme.typography.bodySmall,
                        color = Theme.colors.muted,
                        modifier = Modifier.padding(horizontal = Theme.gutter, vertical = 4.dp),
                    )
                    Box(Modifier.weight(1f).fillMaxWidth()) { DocumentView(current) }
                }
                calls.busy -> Loading()
                else -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                    TextButton(onClick = { scope.launch { open() } }) { Text(t("common.retry")) }
                }
            }
        }
    }
    val reasons = choices
    if (reporting && current != null && reasons != null) {
        ReportDialog(reasons.reportReasons, reasons.limits.reportDetail.toInt(), calls.busy, calls.error, dismiss = { reporting = false }) { reason, detail ->
            scope.launch {
                if (calls.call(ExamAction.Report(id, reason, detail)) != null) {
                    reporting = false
                    ExamViews.notice = t("examShare.reported")
                }
            }
        }
    }
}

@Composable
private fun DocumentView(file: ExamLocalDocument) {
    when (file.document.fileType) {
        "application/pdf" -> PdfView(file.path)
        "text/plain" -> {
            val text by produceState<String?>(null, file.path) {
                value = withContext(Dispatchers.IO) {
                    runCatching { File(file.path).readText() }.getOrElse { t("error.storage") }
                }
            }
            Text(
                text ?: t("common.loading"),
                color = Theme.colors.ink,
                modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(Theme.gutter),
            )
        }
        else -> ImageView(file.path)
    }
}

/** androidx.pdf renders in its own sandboxed process, page by page as they
 * scroll in, with selection and zoom. It is a beta: when it cannot open the
 * file, the platform's PdfRenderer shows it a page at a time. */
@OptIn(ExperimentalPdfApi::class)
@Composable
private fun PdfView(path: String) {
    val context = LocalContext.current
    val document by produceState<Result<PdfDocument>?>(null, path) {
        value = runCatching { SandboxedPdfLoader(context).openDocument(Uri.fromFile(File(path))) }
    }
    DisposableEffect(document) { onDispose { document?.getOrNull()?.close() } }
    when (val loaded = document) {
        null -> Box(Modifier.fillMaxSize(), Alignment.Center) { CircularProgressIndicator() }
        else -> loaded.getOrNull()?.let {
            PdfViewer(it, rememberPdfViewerState(), Modifier.fillMaxSize().background(Theme.colors.surface2))
        } ?: RendererPdfView(path)
    }
}

/** The fallback reader: one page rendered by PdfRenderer, with page steps. */
@Composable
private fun RendererPdfView(path: String) {
    var page by rememberSaveable(path) { mutableStateOf(0) }
    val result by produceState<Result<Pair<Bitmap, Int>>?>(null, path, page) {
        value = withContext(Dispatchers.IO) {
            runCatching {
                ParcelFileDescriptor.open(File(path), ParcelFileDescriptor.MODE_READ_ONLY).use { descriptor ->
                    PdfRenderer(descriptor).use { pdf ->
                        pdf.openPage(page).use { source ->
                            val scale = (1400f / maxOf(source.width, source.height)).coerceAtMost(2f)
                            val bitmap = Bitmap.createBitmap((source.width * scale).toInt().coerceAtLeast(1), (source.height * scale).toInt().coerceAtLeast(1), Bitmap.Config.ARGB_8888)
                            bitmap.eraseColor(Color.WHITE)
                            source.render(bitmap, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
                            bitmap to pdf.pageCount
                        }
                    }
                }
            }
        }
    }
    val preview = result
    val shown = preview?.getOrNull()
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(Theme.gutter)) {
        if (shown == null) {
            Text(t(if (preview == null) "common.loading" else "error.unavailable"), color = Theme.colors.muted)
        } else {
            val (bitmap, pages) = shown
            if (pages > 1) Row(verticalAlignment = Alignment.CenterVertically) {
                TextButton(enabled = page > 0, onClick = { page-- }) { Text(t("common.previousPage")) }
                Text("${page + 1} / $pages")
                TextButton(enabled = page + 1 < pages, onClick = { page++ }) { Text(t("common.nextPage")) }
            }
            Image(bitmap.asImageBitmap(), null, Modifier.fillMaxWidth().aspectRatio(bitmap.width.toFloat() / bitmap.height))
        }
    }
}

/** A photo fitted to the screen; pinch zooms, drag pans while zoomed. */
@Composable
private fun ImageView(path: String) {
    val bitmap by produceState<Result<Bitmap>?>(null, path) {
        value = withContext(Dispatchers.IO) {
            runCatching {
                val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                BitmapFactory.decodeFile(path, bounds)
                val options = BitmapFactory.Options().apply {
                    inSampleSize = maxOf(1, maxOf(bounds.outWidth, bounds.outHeight) / 3000)
                }
                checkNotNull(BitmapFactory.decodeFile(path, options))
            }
        }
    }
    var scale by remember { mutableFloatStateOf(1f) }
    var offset by remember { mutableStateOf(Offset.Zero) }
    val loaded = bitmap ?: return Box(Modifier.fillMaxSize(), Alignment.Center) { CircularProgressIndicator() }
    val shown = loaded.getOrNull() ?: return Box(Modifier.fillMaxSize(), Alignment.Center) {
        Text(t("error.unavailable"), color = Theme.colors.muted)
    }
    Box(
        Modifier
            .fillMaxSize()
            .background(Theme.colors.surface2)
            .pointerInput(Unit) {
                detectTransformGestures { _, pan, zoom, _ ->
                    scale = (scale * zoom).coerceIn(1f, 8f)
                    offset = if (scale == 1f) Offset.Zero else offset + pan
                }
            },
    ) {
        Image(
            shown.asImageBitmap(), null,
            contentScale = ContentScale.Fit,
            modifier = Modifier
                .fillMaxSize()
                .graphicsLayer {
                    scaleX = scale
                    scaleY = scale
                    translationX = offset.x
                    translationY = offset.y
                },
        )
    }
}

/** One request in full, then the one thing to do with it: share a file for
 * it, see the shared one, or manage one's own. [mine] names the board it was
 * listed on, which Core answers again after a change. */
@Composable
fun ExamRequestScreen(id: String, mine: Boolean) {
    val navigator = LocalNavigator.current
    val scope = rememberCoroutineScope()
    val links = rememberExamLinks()
    val calls = rememberExamCalls()
    val choices by rememberExamChoices(calls)
    var request by remember(id) { mutableStateOf(ExamViews.requests[id]) }
    var missing by remember { mutableStateOf(false) }
    var uploadForm by remember { mutableStateOf<ExamUploadForm?>(null) }
    var deleting by remember { mutableStateOf(false) }

    /** Takes this request from a board Core answered, when it is listed. */
    fun take(answer: ExamAnswer?): Boolean {
        val board = (answer as? ExamAnswer.Requests)?.v1 ?: return false
        ExamViews.board(board)
        board.items.firstOrNull { it.id == id }?.let { request = it }
        return true
    }

    suspend fun refresh() {
        if (take(calls.call(ExamAction.Requests(mine, null)))) missing = request == null
    }

    // Returning to the page (from the answer's document, or to the app) reads the board again.
    LifecycleResumeEffect(id) {
        val job = scope.launch { refresh() }
        onPauseOrDispose { job.cancel() }
    }

    ExamFrame(calls) {
        TabPage(t("examShare.requests"), navigationIcon = { BackButton() }) {
            val item = request
            when {
                item != null -> Column(
                    Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(Theme.gutter),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    RequestDetail(item)
                    item.answerExamId?.let { answer ->
                        Button(onClick = { navigator.openExamDocument(answer) }, modifier = Modifier.fillMaxWidth()) {
                            Text(t("examShare.viewAnswer"))
                        }
                    }
                    if (!item.mine && item.status == "open") {
                        Button(enabled = !calls.busy && choices != null, onClick = {
                            scope.launch {
                                uploadForm = (calls.call(ExamAction.UploadForm(ExamUploadStart.Request(item))) as? ExamAnswer.UploadForm)?.v1
                            }
                        }, modifier = Modifier.fillMaxWidth()) { Text(t("examShare.upload.answerRequest")) }
                    }
                    if (item.mine) {
                        OutlinedButton(
                            enabled = !calls.busy,
                            onClick = {
                                scope.launch {
                                    val open = item.status != "open"
                                    val answer = calls.call(ExamAction.SetRequestOpen(item.id, open, mine))
                                    // A closed request may leave the open board; its status is then the one asked for.
                                    if (take(answer) && request?.status == item.status) request = item.copy(status = if (open) "open" else "closed")
                                }
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) { Text(t(if (item.status == "open") "examShare.closeRequest" else "examShare.reopenRequest")) }
                        TextButton(enabled = !calls.busy, onClick = { deleting = true }, modifier = Modifier.fillMaxWidth()) {
                            Text(t("common.delete"), color = Theme.colors.danger)
                        }
                    }
                    TextButton(
                        onClick = { links.open(ExamViews.requestBoardUrl ?: "$EXAM_SITE/request") },
                        modifier = Modifier.fillMaxWidth(),
                    ) { Text(t("examShare.openOnWeb")) }
                }
                missing -> Box(Modifier.fillMaxSize(), Alignment.Center) { Text(t("error.unavailable"), color = Theme.colors.muted) }
                calls.busy -> Loading()
                else -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                    TextButton(onClick = { scope.launch { refresh() } }) { Text(t("common.retry")) }
                }
            }
        }
    }
    val form = uploadForm
    val formChoices = choices
    if (form != null && formChoices != null) {
        UploadSheet(form, formChoices, dismiss = { uploadForm = null }) {
            uploadForm = null
            ExamViews.notice = t("examShare.upload.uploaded")
            navigator.back()
        }
    }
    if (deleting) {
        ConfirmDialog(Confirmation("examShare.deleteRequest", ExamAction.DeleteRequest(id, mine)), { deleting = false }) { action ->
            scope.launch {
                if (take(calls.call(action))) {
                    ExamViews.requests.remove(id)
                    navigator.back()
                }
            }
        }
    }
}

@Composable
private fun RequestDetail(item: ExamRequest) {
    Card {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(item.subject.code, style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
            Text(item.subject.name, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold)
            StatusTag(item.status)
            requestFacts(item).ifBlank { null }?.let { Text(it, color = Theme.colors.muted) }
            if (item.teachers.isNotEmpty()) Text(item.teachers.joinToString(" / ") { it.name }, color = Theme.colors.muted)
        }
    }
    if (item.description.isNotBlank()) Card {
        Text(item.description, color = Theme.colors.ink, modifier = Modifier.padding(16.dp))
    }
}
