package app.isctcompanion.features.pastexams

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.webkit.MimeTypeMap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.MoreVert
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
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
import app.isctcompanion.core.ExamAction
import app.isctcompanion.core.ExamAnswer
import app.isctcompanion.core.ExamLocalDocument
import app.isctcompanion.core.copyToDownloads
import app.isctcompanion.core.openLocalFile
import app.isctcompanion.design.EmptyState
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import app.isctcompanion.design.PdfReader

@Composable
fun ExamDocumentScreen(id: String) {
    val scope = rememberCoroutineScope()
    val links = rememberExamLinks()
    val calls = rememberExamCalls()
    val choices by rememberExamChoices(calls)
    var file by remember(id) { mutableStateOf<ExamLocalDocument?>(null) }
    var menu by remember { mutableStateOf(false) }
    var reporting by rememberSaveable { mutableStateOf(false) }
    var downloading by remember { mutableStateOf(false) }
    val context = LocalContext.current

    suspend fun download(opened: ExamLocalDocument) {
        if (downloading) return
        downloading = true
        try {
            if (!opened.downloaded && !opened.offline) {
                calls.call(ExamAction.RetainOpen(opened.document.id, opened.path)) ?: return
                file?.let { if (it.path == opened.path) file = it.copy(downloaded = true) }
            }
            val extension = opened.exportName.substringAfterLast('.', "").lowercase()
            val mimeType = MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension)
                ?: opened.document.fileType
            val saved = withContext(Dispatchers.IO) {
                copyToDownloads(context, File(opened.path), opened.exportName, mimeType)
            }
            ExamViews.notice = if (saved) t("app.savedName", "name" to opened.exportName)
            else t("lmsFile.theFileCouldNotBeDownloaded")
        } finally {
            downloading = false
        }
    }

    suspend fun open() {
        (calls.call(ExamAction.Open(id)) as? ExamAnswer.Document)?.v1?.let { file = it }
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
                            { Text(t("examShare.download")) },
                            {
                                menu = false
                                scope.launch { download(current) }
                            },
                            enabled = !calls.busy && !downloading,
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
            compact = true,
        ) {
            when {
                current != null -> Column(Modifier.fillMaxSize()) {
                    // A downloaded copy shown without the site.
                    if (current.offline) Text(
                        t("examShare.offline"),
                        style = MaterialTheme.typography.bodySmall,
                        color = Theme.colors.muted,
                        modifier = Modifier.padding(horizontal = Theme.gutter, vertical = Theme.rowY),
                    )
                    Box(Modifier.weight(1f).fillMaxWidth()) { ExamDocumentPreview(current) }
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
private fun ExamDocumentPreview(file: ExamLocalDocument) {
    val context = LocalContext.current
    val source = remember(file.path) { File(file.path) }
    var attempt by remember(source) { mutableIntStateOf(0) }
    var externalFailed by remember(source) { mutableStateOf(false) }
    val failure: @Composable () -> Unit = {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                EmptyState(
                    Icons.Rounded.Description,
                    t(if (externalFailed) "examShare.externalOpenFailed" else "examShare.previewFailed"),
                    action = t("common.retry") to {
                        externalFailed = false
                        attempt++
                    },
                )
                TextButton(onClick = {
                    externalFailed = !openLocalFile(context, source, file.document.fileType, file.exportName)
                }) { Text(t("common.openInAnotherApp")) }
            }
        }
    }
    key(source, attempt) {
        when (file.document.fileType) {
            "application/pdf" -> PdfReader(source, failure)
            "text/plain" -> PreviewContent(readPreviewFile(source) { it.readText() }, failure) { text ->
                SelectionContainer {
                    Text(
                        text,
                        color = Theme.colors.ink,
                        modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(Theme.gutter),
                    )
                }
            }
            else -> PreviewContent(readPreviewFile(source, ::decodeDocumentImage), failure) { DocumentImage(it) }
        }
    }
}

@Composable
private fun <T : Any> readPreviewFile(file: File, read: (File) -> T): Result<T>? {
    val result by produceState<Result<T>?>(null, file) {
        value = withContext(Dispatchers.IO) { runCatching { read(file) } }
    }
    return result
}

@Composable
private fun <T : Any> PreviewContent(result: Result<T>?, failure: @Composable () -> Unit, content: @Composable (T) -> Unit) {
    val loaded = result?.getOrNull()
    when {
        result == null -> Loading()
        loaded == null -> failure()
        else -> content(loaded)
    }
}

private fun decodeDocumentImage(file: File): Bitmap {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeFile(file.path, bounds)
    val options = BitmapFactory.Options().apply {
        inSampleSize = maxOf(1, maxOf(bounds.outWidth, bounds.outHeight) / 3000)
    }
    return checkNotNull(BitmapFactory.decodeFile(file.path, options))
}

/** A photo fitted to the screen; pinch zooms, drag pans while zoomed. */
@Composable
private fun DocumentImage(bitmap: Bitmap) {
    var scale by remember { mutableFloatStateOf(1f) }
    var offset by remember { mutableStateOf(Offset.Zero) }
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
            bitmap.asImageBitmap(), null,
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
