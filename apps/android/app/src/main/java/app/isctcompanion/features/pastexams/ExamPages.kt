package app.isctcompanion.features.pastexams

import android.graphics.BitmapFactory
import android.net.Uri
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
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.ExpandLess
import androidx.compose.material.icons.rounded.ExpandMore
import androidx.compose.material.icons.rounded.MoreVert
import androidx.compose.material3.AlertDialog
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
import androidx.compose.ui.unit.dp
import androidx.pdf.ExperimentalPdfApi
import androidx.pdf.PdfDocument
import androidx.pdf.SandboxedPdfLoader
import androidx.pdf.compose.PdfViewer
import androidx.pdf.compose.rememberPdfViewerState
import app.isctcompanion.core.ExamAction
import app.isctcompanion.core.ExamAnswer
import app.isctcompanion.core.ExamLocalDocument
import app.isctcompanion.core.ExamSet
import app.isctcompanion.core.ExamSubjectPage
import app.isctcompanion.core.ExamTeacher
import app.isctcompanion.design.Card
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
private fun BackButton() {
    val store = LocalExamStore.current
    IconButton(onClick = store::pop) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, t("common.back")) }
}

internal fun uniqueTeachers(teachers: List<ExamTeacher>): List<ExamTeacher> = teachers.distinctBy { it.id }

/** One course's sittings as the desktop's panel shows them: the person's own
 * teachers first, the others folded away, a button per document. */
@Composable
internal fun SubjectPage(route: ExamPage.Subject) {
    val store = LocalExamStore.current
    val scope = rememberCoroutineScope()
    val uri = LocalUriHandler.current
    var page by remember { mutableStateOf<ExamSubjectPage?>(null) }
    var failed by remember { mutableStateOf(false) }
    var othersOpen by remember { mutableStateOf(false) }
    var menu by remember { mutableStateOf(false) }
    var uploading by remember { mutableStateOf(false) }
    var requesting by remember { mutableStateOf(false) }
    suspend fun load() {
        failed = false
        val loaded = (store.call(ExamAction.Subject(route.code, route.instructor)) as? ExamAnswer.Subject)?.v1
        if (loaded != null) page = loaded else failed = page == null
    }
    LaunchedEffect(route) { load() }
    TabPage(
        route.title,
        navigationIcon = { BackButton() },
        actions = {
            if (page != null) Box {
                IconButton(onClick = { menu = true }) { Icon(Icons.Rounded.Add, t("examShare.uploadHere")) }
                DropdownMenu(menu, { menu = false }) {
                    DropdownMenuItem({ Text(t("examShare.uploadHere")) }, { menu = false; uploading = true })
                    DropdownMenuItem({ Text(t("examShare.newRequest")) }, { menu = false; requesting = true })
                    DropdownMenuItem({ Text(t("examShare.openInBrowser")) }, {
                        menu = false
                        uri.openUri("$EXAM_SITE/search?course=${Uri.encode(route.code)}")
                    })
                }
            }
        },
    ) {
        val current = page
        when {
            current != null -> {
                val grouped = current.myTeachers.isNotEmpty()
                val mine = current.sets.filter { !grouped || it.mine }
                val others = if (grouped) current.sets.filter { !it.mine } else emptyList()
                LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(Theme.gutter), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (current.sets.isEmpty()) {
                        item { Text(t("examShare.noDocuments"), color = Theme.colors.muted) }
                    } else {
                        if (mine.isEmpty()) item { Text(t("examShare.noOwnTeacher"), color = Theme.colors.muted) }
                        else cardRows(mine, { it.documents.first().id }) { SetRow(it, withTeachers = !it.mine) }
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
                            if (othersOpen) cardRows(others, { "other:" + it.documents.first().id }) { SetRow(it, withTeachers = true) }
                        }
                    }
                }
            }
            failed -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                TextButton(onClick = { scope.launch { load() } }) { Text(t("common.retry")) }
            }
            else -> Box(Modifier.fillMaxSize(), Alignment.Center) { CircularProgressIndicator() }
        }
    }
    val current = page
    if (uploading && current != null) {
        UploadSheet(
            ExamUploadTarget(
                current.subject.code, route.title,
                uniqueTeachers(current.rows.flatMap { it.teachers }), current.myTeachers,
            ),
            dismiss = { uploading = false },
        ) { scope.launch { load() } }
    }
    if (requesting && current != null) {
        RequestDialog(current, dismiss = { requesting = false }) { draft ->
            scope.launch {
                if (store.call(ExamAction.CreateRequest(draft)) != null) {
                    requesting = false
                    store.notice = t("examShare.requestCreated")
                }
            }
        }
    }
}

/** One sitting: a label, and a button per document (question, answer) — the
 * buttons are what opens, as on the desktop and iOS. */
@Composable
private fun SetRow(set: ExamSet, withTeachers: Boolean) {
    val store = LocalExamStore.current
    ExamRow(
        store.sitting(set.academicYear, set.occasion, set.occasionNumber, set.variant),
        if (withTeachers) set.teachers.joinToString(" / ") { it.name }.ifBlank { t("examShare.unknownTeacher") } else null,
        onClick = null,
        leading = { Icon(Icons.Rounded.Description, null, tint = Theme.colors.accent, modifier = Modifier.size(20.dp)) },
        trailing = {
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                set.documents.forEach { document ->
                    val answer = document.role == "answer"
                    val ink = if (answer) Theme.colors.ok else Theme.colors.accent
                    Text(
                        document.role?.let { t("examShare.role.$it") } ?: t("examShare.file"),
                        color = ink,
                        style = MaterialTheme.typography.labelLarge,
                        fontWeight = FontWeight.SemiBold,
                        modifier = Modifier
                            .background(if (answer) Theme.colors.okSoft else Theme.colors.accentSoft, CircleShape)
                            .border(1.dp, ink.copy(alpha = 0.35f), CircleShape)
                            .clickable { store.push(ExamPage.Document(document.id)) }
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
internal fun DocumentPage(route: ExamPage.Document) {
    val store = LocalExamStore.current
    val scope = rememberCoroutineScope()
    val uri = LocalUriHandler.current
    var file by remember { mutableStateOf<ExamLocalDocument?>(null) }
    var failed by remember { mutableStateOf(false) }
    var menu by remember { mutableStateOf(false) }
    var reporting by remember { mutableStateOf(false) }
    suspend fun open(download: Boolean = false) {
        failed = false
        val opened = (store.call(ExamAction.Open(route.id, download)) as? ExamAnswer.Document)?.v1
        if (opened != null) {
            file = opened
            if (download) store.notice = t("examShare.downloadedNotice")
        } else {
            failed = file == null
        }
    }
    LaunchedEffect(route) { open() }
    val current = file
    TabPage(
        current?.let { store.title(it.document) } ?: "",
        navigationIcon = { BackButton() },
        actions = {
            if (current != null) Box {
                IconButton(onClick = { menu = true }) { Icon(Icons.Rounded.MoreVert, t("examShare.report")) }
                DropdownMenu(menu, { menu = false }) {
                    DropdownMenuItem(
                        { Text(t(if (current.downloaded) "examShare.downloaded" else "examShare.download")) },
                        { menu = false; scope.launch { open(download = true) } },
                        enabled = !current.downloaded && !current.offline,
                    )
                    current.saved?.let { saved ->
                        DropdownMenuItem(
                            { Text(t(if (saved) "examShare.unsave" else "examShare.save")) },
                            {
                                menu = false
                                scope.launch {
                                    (store.call(ExamAction.Save(route.id, !saved)) as? ExamAnswer.Saved)?.v1?.let {
                                        file = current.copy(saved = it)
                                    }
                                }
                            },
                            enabled = !current.offline,
                        )
                    }
                    DropdownMenuItem({ Text(t("examShare.openInBrowser")) }, {
                        menu = false
                        uri.openUri("$EXAM_SITE/view/${Uri.encode(route.id)}")
                    })
                    DropdownMenuItem(
                        { Text(t("examShare.report"), color = Theme.colors.danger) },
                        { menu = false; reporting = true },
                        enabled = !current.offline,
                    )
                }
            }
        },
    ) {
        when {
            current != null -> DocumentView(current)
            failed -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                TextButton(onClick = { scope.launch { open() } }) { Text(t("common.retry")) }
            }
            else -> Box(Modifier.fillMaxSize(), Alignment.Center) { CircularProgressIndicator() }
        }
    }
    if (reporting && current != null) {
        ReportDialog(dismiss = { reporting = false }) { reason, detail ->
            scope.launch {
                if (store.call(ExamAction.Report(route.id, reason, detail)) != null) {
                    reporting = false
                    store.notice = t("examShare.reported")
                }
            }
        }
    }
}

@Composable
private fun DocumentView(file: ExamLocalDocument) {
    when {
        file.document.fileType == "application/pdf" -> PdfView(file.path)
        file.document.fileType == "text/plain" -> {
            val text by produceState<String?>(null, file.path) {
                value = withContext(Dispatchers.IO) { runCatching { File(file.path).readText() }.getOrNull() }
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
 * scroll in, with selection, search and zoom; a 35-megapixel scan no longer
 * decodes into one bitmap. */
@OptIn(ExperimentalPdfApi::class)
@Composable
private fun PdfView(path: String) {
    val context = LocalContext.current
    val document by produceState<Result<PdfDocument>?>(null, path) {
        value = runCatching { SandboxedPdfLoader(context).openDocument(Uri.fromFile(File(path)), null) }
    }
    DisposableEffect(document) { onDispose { document?.getOrNull()?.close() } }
    when (val loaded = document) {
        null -> Box(Modifier.fillMaxSize(), Alignment.Center) { CircularProgressIndicator() }
        else -> loaded.getOrNull()?.let {
            PdfViewer(it, rememberPdfViewerState(), Modifier.fillMaxSize().background(Theme.colors.surface2))
        } ?: Box(Modifier.fillMaxSize(), Alignment.Center) {
            Text(t("error.exam-share-response"), color = Theme.colors.muted)
        }
    }
}

/** A photo fitted to the width; pinch zooms, drag pans while zoomed,
 * double tap resets. */
@Composable
private fun ImageView(path: String) {
    val bitmap by produceState<android.graphics.Bitmap?>(null, path) {
        value = withContext(Dispatchers.IO) {
            runCatching {
                val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                BitmapFactory.decodeFile(path, bounds)
                val options = BitmapFactory.Options().apply {
                    inSampleSize = maxOf(1, maxOf(bounds.outWidth, bounds.outHeight) / 3000)
                }
                BitmapFactory.decodeFile(path, options)
            }.getOrNull()
        }
    }
    var scale by remember { mutableFloatStateOf(1f) }
    var offset by remember { mutableStateOf(Offset.Zero) }
    val shown = bitmap ?: return Box(Modifier.fillMaxSize(), Alignment.Center) { CircularProgressIndicator() }
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
 * it, see the shared one, or manage one's own. */
@Composable
internal fun RequestPage(route: ExamPage.Request) {
    val store = LocalExamStore.current
    val scope = rememberCoroutineScope()
    val uri = LocalUriHandler.current
    var item by remember { mutableStateOf(route.request) }
    var uploading by remember { mutableStateOf(false) }
    var deleting by remember { mutableStateOf(false) }
    TabPage(t("examShare.requests"), navigationIcon = { BackButton() }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(Theme.gutter),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Card {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(item.subject.code, style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
                    Text(store.subjectName(item.subject), style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold)
                    StatusTag(item.status)
                    requestFacts(item).ifBlank { null }?.let { Text(it, color = Theme.colors.muted) }
                    if (item.teachers.isNotEmpty()) Text(item.teachers.joinToString(" / ") { it.name }, color = Theme.colors.muted)
                }
            }
            if (item.description.isNotBlank()) Card {
                Text(item.description, color = Theme.colors.ink, modifier = Modifier.padding(16.dp))
            }
            item.answerExamId?.let { answer ->
                Button(onClick = { store.push(ExamPage.Document(answer)) }, modifier = Modifier.fillMaxWidth()) {
                    Text(t("examShare.viewAnswer"))
                }
            }
            if (!item.mine && item.status == "open") {
                Button(onClick = { uploading = true }, modifier = Modifier.fillMaxWidth()) { Text(t("examShare.answerRequest")) }
            }
            if (item.mine) {
                OutlinedButton(
                    onClick = {
                        scope.launch {
                            val open = item.status != "open"
                            if (store.call(ExamAction.SetRequestOpen(item.id, open)) != null) {
                                item = item.copy(status = if (open) "open" else "closed")
                            }
                        }
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) { Text(t(if (item.status == "open") "examShare.closeRequest" else "examShare.reopenRequest")) }
                TextButton(onClick = { deleting = true }, modifier = Modifier.fillMaxWidth()) {
                    Text(t("common.delete"), color = Theme.colors.danger)
                }
            }
            TextButton(onClick = { uri.openUri("$EXAM_SITE/request") }, modifier = Modifier.fillMaxWidth()) {
                Text(t("examShare.openInBrowser"))
            }
        }
    }
    if (uploading) {
        UploadSheet(
            ExamUploadTarget(
                item.subject.code, store.subjectName(item.subject), item.teachers, item.teachers.map { it.id },
                academicYear = item.academicYear, occasion = item.occasion, role = item.wantedRole, requestId = item.id,
            ),
            dismiss = { uploading = false },
        ) { store.pop() }
    }
    if (deleting) {
        AlertDialog(
            onDismissRequest = { deleting = false },
            title = { Text(t("examShare.deleteRequest")) },
            confirmButton = {
                TextButton(onClick = {
                    deleting = false
                    scope.launch { if (store.call(ExamAction.DeleteRequest(item.id)) != null) store.pop() }
                }) { Text(t("common.ok"), color = Theme.colors.danger) }
            },
            dismissButton = { TextButton(onClick = { deleting = false }) { Text(t("common.cancel")) } },
        )
    }
}
