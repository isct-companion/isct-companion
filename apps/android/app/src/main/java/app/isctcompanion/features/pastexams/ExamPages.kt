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
import androidx.compose.foundation.horizontalScroll
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
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.ExpandLess
import androidx.compose.material.icons.rounded.ExpandMore
import androidx.compose.material.icons.rounded.MoreVert
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
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
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
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
import app.isctcompanion.core.ExamDocument
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

/** One subject's sittings, from Core's `sets`: the course's own teachers'
 * first, the others folded away, a button per document. */
@Composable
fun ExamSubjectScreen(entry: ExamSubjectEntry) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val scope = rememberCoroutineScope()
    val uri = LocalUriHandler.current
    val calls = rememberExamCalls()
    val opener = rememberExamOpener(calls)
    val choices by rememberExamChoices(calls)
    val connected = model.examAccount?.status == ExamAccountStatus.CONNECTED
    var page by remember { mutableStateOf<ExamSubjectPage?>(null) }
    var othersOpen by rememberSaveable { mutableStateOf(false) }
    var menu by remember { mutableStateOf(false) }
    var requestForm by rememberSaveable { mutableStateOf(false) }
    var uploadForm by remember { mutableStateOf<ExamUploadForm?>(null) }

    /** The subject again once shown; a course entry resolves it the first time. */
    suspend fun load() {
        val action = page?.let { ExamAction.Subject(it.subject.code, it.classId) } ?: when (entry) {
            is ExamSubjectEntry.Course -> ExamAction.CourseSubject(entry.courseId)
            is ExamSubjectEntry.Code -> ExamAction.Subject(entry.code, entry.classId)
        }
        (calls.call(action) as? ExamAnswer.Subject)?.v1?.let { page = it }
    }

    suspend fun reloadVisible(action: ExamAction = ExamAction.Account) {
        if ((calls.call(action) as? ExamAnswer.Account)?.v1?.status == ExamAccountStatus.CONNECTED) load()
    }

    fun uploaded(id: String?) {
        calls.notice = t("examShare.upload.uploaded")
        if (id != null) navigator.openExamDocument(id) else scope.launch { load() }
    }

    // Returning to the page (from a document, or to the app) reloads it.
    LifecycleResumeEffect(entry) {
        val refresh = scope.launch { reloadVisible() }
        onPauseOrDispose { refresh.cancel() }
    }

    val editingUpload = uploadForm
    val formChoices = choices
    if (editingUpload != null && formChoices != null) {
        ExamUploadScreen(form = editingUpload, choices = formChoices, onBack = { uploadForm = null }, onUploaded = { id ->
            uploadForm = null
            uploaded(id)
        })
        return
    }

    val current = page
    TabPage(
        current?.subject?.name ?: t("examShare.title"),
        navigationIcon = { BackButton() },
        actions = {
            if (current != null && connected) Box {
                IconButton(onClick = { menu = true }) { Icon(Icons.Rounded.Add, t("examShare.upload.title")) }
                DropdownMenu(menu, { menu = false }) {
                    DropdownMenuItem({ Text(t("examShare.upload.title")) }, enabled = !calls.busy, onClick = {
                        menu = false
                        scope.launch {
                            uploadForm = (calls.call(ExamAction.UploadForm(ExamUploadStart.Subject(current.subject.code))) as? ExamAnswer.UploadForm)?.v1
                        }
                    })
                    DropdownMenuItem({ Text(t("examShare.newRequest")) }, onClick = { menu = false; requestForm = true })
                    DropdownMenuItem({ Text(t("examShare.openOnWeb")) }, onClick = { menu = false; uri.openUri(current.webUrl) })
                }
            }
        },
    ) {
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(Theme.gutter), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (!connected) {
                item { ExamAccountPanel(calls, { reloadVisible(it) }, ::uploaded) }
                return@LazyColumn
            }
            item { ExamStatus(calls) }
            when {
                current != null -> {
                    val mine = current.sets.filter { it.mine }
                    // Without a class of one's own, every sitting is listed alike.
                    val grouped = mine.isNotEmpty()
                    val others = if (grouped) current.sets.filter { !it.mine } else emptyList()
                    if (current.sets.isEmpty()) item { Text(t("examShare.noItems"), color = Theme.colors.muted) }
                    setRows(if (grouped) mine else current.sets, "set", opener, withTeachers = !grouped)
                    if (others.isNotEmpty()) {
                        item {
                            Row(
                                Modifier.fillMaxWidth().clickable { othersOpen = !othersOpen }.padding(vertical = 8.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(4.dp),
                            ) {
                                Icon(if (othersOpen) Icons.Rounded.ExpandLess else Icons.Rounded.ExpandMore, null, tint = Theme.colors.muted)
                                Text(t("examShare.otherTeachers", "count" to others.size), color = Theme.colors.muted)
                            }
                        }
                        if (othersOpen) setRows(others, "other", opener, withTeachers = true)
                    }
                }
                !calls.busy -> item {
                    TextButton(onClick = { scope.launch { load() } }) { Text(t("common.retry")) }
                }
                else -> Unit
            }
            item { ExamCredit() }
        }
    }
    current?.takeIf { requestForm }?.let { shown -> formChoices?.let {
        RequestDialog(shown, it, calls.busy, calls.error, { requestForm = false }) { draft ->
            scope.launch {
                if (calls.call(ExamAction.CreateRequest(draft)) != null) {
                    requestForm = false
                    calls.notice = t("examShare.requestCreated")
                }
            }
        }
    } }
}

private fun LazyListScope.setRows(sets: List<ExamSet>, key: String, opener: ExamOpener, withTeachers: Boolean) {
    items(sets, key = { "$key:${it.documents.first().id}" }) { SetRow(it, opener, withTeachers) }
}

/** One sitting: its name and a pill per document (question before answer).
 * The pills are what opens, so the row itself is not a button. */
@Composable
private fun SetRow(set: ExamSet, opener: ExamOpener, withTeachers: Boolean) {
    Card {
        Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                sittingLabel(set.academicYear, set.occasion, set.occasionNumber, set.variant).ifEmpty { set.documents.first().fileName },
                style = MaterialTheme.typography.bodyLarge, color = Theme.colors.ink,
            )
            if (withTeachers && set.teachers.isNotEmpty()) {
                Text(set.teachers.joinToString(" / ") { it.name }, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted)
            }
            Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                set.documents.forEach { document -> RolePill(document, opener) }
            }
        }
    }
}

@Composable
private fun RolePill(document: ExamDocument, opener: ExamOpener) {
    val answer = document.role == "answer"
    val ink = if (answer) Theme.colors.ok else Theme.colors.accent
    Row(
        Modifier
            .background(if (answer) Theme.colors.okSoft else Theme.colors.accentSoft, CircleShape)
            .border(1.dp, ink.copy(alpha = 0.35f), CircleShape)
            .clickable { opener.open(document.id) }
            .padding(horizontal = 12.dp, vertical = 5.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        if (opener.opening == document.id) CircularProgressIndicator(Modifier.size(14.dp), color = ink, strokeWidth = 2.dp)
        Text(
            document.role?.let { t("examShare.role.$it") } ?: document.fileName,
            color = ink,
            style = MaterialTheme.typography.labelLarge,
            fontWeight = FontWeight.SemiBold,
            maxLines = 1,
        )
    }
}

/** A past exam full screen: PDFs in androidx.pdf, photos zoomable, text as
 * text; its actions in the menu. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ExamDocumentScreen(id: String) {
    val scope = rememberCoroutineScope()
    val uri = LocalUriHandler.current
    val calls = rememberExamCalls()
    val choices by rememberExamChoices(calls)
    var file by remember(id) { mutableStateOf(ExamViews.documents.remove(id)) }
    var menu by remember { mutableStateOf(false) }
    var reportForm by rememberSaveable { mutableStateOf(false) }

    suspend fun open(download: Boolean = false) {
        (calls.call(ExamAction.Open(id, download)) as? ExamAnswer.Document)?.v1?.let { file = it }
    }
    LaunchedEffect(id) { if (file == null) open() }

    val current = file
    Scaffold(
        containerColor = Theme.colors.bg,
        topBar = {
            TopAppBar(
                title = {
                    Text(current?.let { examLabel(it.document) }.orEmpty(), maxLines = 1, overflow = TextOverflow.Ellipsis,
                        style = MaterialTheme.typography.titleMedium)
                },
                navigationIcon = { BackButton() },
                actions = {
                    if (current != null) Box {
                        IconButton(onClick = { menu = true }) { Icon(Icons.Rounded.MoreVert, t("examShare.more")) }
                        DropdownMenu(menu, { menu = false }) {
                            DropdownMenuItem(
                                { Text(t(if (current.downloaded) "examShare.downloaded" else "examShare.download")) },
                                enabled = !calls.busy && !current.downloaded && !current.offline,
                                onClick = { menu = false; scope.launch { open(download = true) } },
                            )
                            current.saved?.let { saved ->
                                DropdownMenuItem(
                                    { Text(t(if (saved) "examShare.unsave" else "examShare.save")) },
                                    enabled = !calls.busy && !current.offline,
                                    onClick = {
                                        menu = false
                                        scope.launch {
                                            (calls.call(ExamAction.Save(id, !saved)) as? ExamAnswer.Saved)?.let {
                                                file = current.copy(saved = it.v1)
                                            }
                                        }
                                    },
                                )
                            }
                            DropdownMenuItem({ Text(t("examShare.openOnWeb")) }, onClick = { menu = false; uri.openUri(current.webUrl) })
                            DropdownMenuItem(
                                { Text(t("examShare.report"), color = Theme.colors.danger) },
                                enabled = !calls.busy && !current.offline,
                                onClick = { menu = false; reportForm = true },
                            )
                        }
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = Theme.colors.bg),
            )
        },
    ) { padding ->
        Column(Modifier.padding(padding).fillMaxSize()) {
            val status = calls.error != null || calls.notice != null || current?.offline == true
            if (status) Column(Modifier.padding(horizontal = Theme.gutter, vertical = 4.dp)) {
                if (current?.offline == true) Text(t("examShare.offline"), color = Theme.colors.muted)
                calls.error?.let { Text(it, color = Theme.colors.danger) }
                calls.notice?.let { Text(it, color = Theme.colors.muted) }
            }
            Box(Modifier.weight(1f).fillMaxWidth()) {
                when {
                    current != null -> DocumentView(current)
                    calls.busy -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                    else -> TextButton(onClick = { scope.launch { open() } }, modifier = Modifier.align(Alignment.Center)) {
                        Text(t("common.retry"))
                    }
                }
            }
        }
    }
    current?.takeIf { reportForm }?.let { shown -> choices?.let {
        ReportDialog(it.reportReasons, it.limits.reportDetail.toInt(), calls.busy, calls.error, { reportForm = false }) { reason, detail ->
            scope.launch {
                if (calls.call(ExamAction.Report(shown.document.id, reason, detail)) != null) {
                    reportForm = false
                    calls.notice = t("examShare.reported")
                }
            }
        }
    } }
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

/** One request in full, then what can be done with it: share a file for it,
 * see the shared one, or close, reopen or delete one's own. [mine] names the
 * board it was listed on, which Core answers again after a change. */
@Composable
fun ExamRequestScreen(id: String, mine: Boolean) {
    val navigator = LocalNavigator.current
    val scope = rememberCoroutineScope()
    val uri = LocalUriHandler.current
    val calls = rememberExamCalls()
    val opener = rememberExamOpener(calls)
    val choices by rememberExamChoices(calls)
    var request by remember(id) { mutableStateOf(ExamViews.requests[id]) }
    var missing by remember { mutableStateOf(false) }
    var uploadForm by remember { mutableStateOf<ExamUploadForm?>(null) }
    var confirmation by remember { mutableStateOf<Confirmation?>(null) }

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

    // Returning to the page (from an upload's document, or to the app) reads the board again.
    LifecycleResumeEffect(id) {
        val job = scope.launch { refresh() }
        onPauseOrDispose { job.cancel() }
    }

    val editingUpload = uploadForm
    val formChoices = choices
    if (editingUpload != null && formChoices != null) {
        ExamUploadScreen(form = editingUpload, choices = formChoices, onBack = { uploadForm = null }, onUploaded = { examId ->
            uploadForm = null
            calls.notice = t("examShare.upload.uploaded")
            if (examId != null) navigator.openExamDocument(examId) else scope.launch { refresh() }
        })
        return
    }

    TabPage(t("examShare.requests"), navigationIcon = { BackButton() }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(Theme.gutter),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            ExamStatus(calls)
            val item = request
            if (item == null) {
                if (missing) Text(t("error.unavailable"), color = Theme.colors.muted)
            } else {
                RequestDetail(item)
                item.answerExamId?.let { answer ->
                    Button(onClick = { opener.open(answer) }, modifier = Modifier.fillMaxWidth()) {
                        if (opener.opening == answer) CircularProgressIndicator(Modifier.size(16.dp).padding(end = 4.dp), strokeWidth = 2.dp)
                        Text(t("examShare.viewAnswer"))
                    }
                }
                if (item.status == "open") {
                    Button(enabled = !calls.busy, onClick = {
                        scope.launch {
                            uploadForm = (calls.call(ExamAction.UploadForm(ExamUploadStart.Request(item))) as? ExamAnswer.UploadForm)?.v1
                        }
                    }, modifier = Modifier.fillMaxWidth()) { Text(t("examShare.upload.answerRequest")) }
                }
                if (item.mine) {
                    OutlinedButton(enabled = !calls.busy, onClick = {
                        scope.launch {
                            val open = item.status != "open"
                            val answer = calls.call(ExamAction.SetRequestOpen(item.id, open, mine))
                            // A closed request may leave the open board; its status is then the one asked for.
                            if (take(answer) && request?.status == item.status) request = item.copy(status = if (open) "open" else "closed")
                        }
                    }, modifier = Modifier.fillMaxWidth()) {
                        Text(t(if (item.status == "open") "examShare.closeRequest" else "examShare.reopenRequest"))
                    }
                    TextButton(enabled = !calls.busy, onClick = {
                        confirmation = Confirmation("examShare.deleteRequest", ExamAction.DeleteRequest(item.id, mine))
                    }, modifier = Modifier.fillMaxWidth()) { Text(t("common.delete"), color = Theme.colors.danger) }
                }
            }
            ExamViews.requestBoardUrl?.let { url ->
                TextButton(onClick = { uri.openUri(url) }, modifier = Modifier.fillMaxWidth()) { Text(t("examShare.openOnWeb")) }
            }
        }
    }
    confirmation?.let {
        ConfirmDialog(it, { confirmation = null }) { action ->
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
            Text(requestFacts(item.copy(description = "")), color = Theme.colors.muted)
            if (item.teachers.isNotEmpty()) Text(item.teachers.joinToString(" / ") { it.name }, color = Theme.colors.muted)
        }
    }
    if (item.description.isNotBlank()) Card {
        Text(item.description, color = Theme.colors.ink, modifier = Modifier.padding(16.dp))
    }
}
