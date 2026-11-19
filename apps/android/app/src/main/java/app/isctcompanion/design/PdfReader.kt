package app.isctcompanion.design

import android.content.Context
import android.graphics.Bitmap
import android.net.Uri
import android.util.Log
import android.util.Size
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.automirrored.rounded.OpenInNew
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.pdf.ExperimentalPdfApi
import androidx.pdf.PdfDocument
import androidx.pdf.SandboxedPdfLoader
import androidx.pdf.compose.PdfViewer
import androidx.pdf.compose.rememberPdfViewerState
import app.isctcompanion.LocalNavigator
import java.io.File
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.withTimeout

// Every PDF the app shows goes through androidx.pdf: the reader (past exams,
// LMS files) and the first-page thumbnail (hand-in). It renders in its own
// sandboxed process, so a malformed file cannot take the app down.

private const val OPEN_TIMEOUT_MS = 8_000L

/** Opens `file` in androidx.pdf's sandboxed service. When that service's
 * process cannot start, binding waits without an error, so it gets a time
 * limit and fails like any other unreadable file. The caller closes it. */
suspend fun openPdf(context: Context, file: File): PdfDocument = try {
    withTimeout(OPEN_TIMEOUT_MS) { SandboxedPdfLoader(context).openDocument(Uri.fromFile(file)) }
} catch (timeout: TimeoutCancellationException) {
    // A timeout of our own is a failure to open, not a cancellation of the caller.
    throw IllegalStateException("The PDF service did not open ${file.name} in time", timeout)
}

/** The first page, `width` pixels wide, or null when it cannot be read. */
suspend fun pdfFirstPage(context: Context, file: File, width: Int): Bitmap? = runCatching {
    openPdf(context, file).use { document ->
        val page = document.getPageInfo(0)
        val height = (width.toLong() * page.height / page.width.coerceAtLeast(1)).toInt().coerceAtLeast(1)
        document.getPageBitmapSource(0).use { source -> source.getBitmap(Size(width, height)) }
    }
}.onFailure { if (it is CancellationException) throw it }.getOrNull()

/** A PDF to read: pages scroll in as they come, with zoom, selection and
 * search. A file it cannot open says so, with a retry. */
@OptIn(ExperimentalPdfApi::class)
@Composable
fun PdfReader(path: String, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    var attempt by remember(path) { mutableIntStateOf(0) }
    val document by produceState<Result<PdfDocument>?>(null, path, attempt) {
        value = null
        value = runCatching { openPdf(context, File(path)) }
            .onFailure {
                if (it is CancellationException) throw it
                Log.w("PdfReader", "androidx.pdf could not open the file", it)
            }
    }
    // Close the document this effect saw: `document` read inside onDispose
    // would be the newly loaded one, closed the moment it arrived.
    val opened = document
    DisposableEffect(opened) { onDispose { opened?.getOrNull()?.close() } }
    val loaded = opened?.getOrNull()
    when {
        opened == null -> Box(modifier.fillMaxSize(), Alignment.Center) { CircularProgressIndicator() }
        loaded == null -> Column(
            modifier.fillMaxSize(),
            verticalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterVertically),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(t("error.unavailable"), color = Theme.colors.muted)
            TextButton(onClick = { attempt++ }) { Text(t("common.retry")) }
        }
        else -> PdfViewer(loaded, rememberPdfViewerState(), modifier.fillMaxSize().background(Theme.colors.surface2))
    }
}

/** A downloaded PDF pushed full screen, with its file still offered to
 * another app from the bar. */
@Composable
fun PdfScreen(path: String, title: String) {
    val navigator = LocalNavigator.current
    val context = LocalContext.current
    TabPage(
        title.ifEmpty { File(path).name },
        navigationIcon = {
            IconButton(onClick = { navigator.back() }) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, t("common.back")) }
        },
        actions = {
            IconButton(onClick = { viewInAnotherApp(context, File(path)) }) {
                Icon(Icons.AutoMirrored.Rounded.OpenInNew, t("common.openInAnotherApp"))
            }
        },
    ) {
        PdfReader(path)
    }
}
