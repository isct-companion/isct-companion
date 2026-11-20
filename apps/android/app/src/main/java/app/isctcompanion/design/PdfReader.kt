package app.isctcompanion.design

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Bitmap
import android.net.Uri
import android.util.Log
import android.util.Size
import android.view.ContextThemeWrapper
import android.view.View
import android.view.ViewTreeObserver
import android.view.accessibility.AccessibilityManager
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.automirrored.rounded.OpenInNew
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.pdf.ExperimentalPdfApi
import androidx.pdf.PdfDocument
import androidx.pdf.SandboxedPdfLoader
import androidx.pdf.view.PdfView
import app.isctcompanion.LocalNavigator
import app.isctcompanion.R
import app.isctcompanion.core.openLocalFile
import java.io.File
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.withTimeout

// Every PDF the app shows goes through androidx.pdf: the reader (past exams,
// LMS files) and the first-page thumbnail (hand-in). It renders in its own
// sandboxed process, so a malformed file cannot take the app down.

/** How long the sandboxed service may take to open a file. When its process
 * cannot start, binding waits without an error, so without a limit the
 * reader would spin forever. */
private const val OPEN_TIMEOUT_MS = 8_000L

/** Opens `file` in androidx.pdf's sandboxed service; the caller closes it. A
 * timeout is a failure to open, not a cancellation of the caller. */
suspend fun openPdf(context: Context, file: File): PdfDocument = try {
    withTimeout(OPEN_TIMEOUT_MS) { SandboxedPdfLoader(context).openDocument(Uri.fromFile(file)) }
} catch (timeout: TimeoutCancellationException) {
    throw IllegalStateException("The PDF service did not open ${file.name} in time", timeout)
}

/** The first page, `width` pixels wide, or null when it cannot be read. */
suspend fun pdfFirstPage(context: Context, file: File, width: Int): Bitmap? = try {
    openPdf(context, file).use { document ->
        val page = document.getPageInfo(0)
        val height = (width.toLong() * page.height / page.width.coerceAtLeast(1)).toInt().coerceAtLeast(1)
        document.getPageBitmapSource(0).use { source -> source.getBitmap(Size(width, height)) }
    }
} catch (cancelled: CancellationException) {
    throw cancelled
} catch (error: Exception) {
    Log.w("PdfReader", "Could not render a PDF thumbnail", error)
    null
}

/** A PDF to read: pages scroll in as they come, with zoom, selection and
 * search. `failure` is shown when the file cannot be opened. */
@OptIn(ExperimentalPdfApi::class)
@Composable
fun PdfReader(file: File, failure: @Composable () -> Unit) {
    val context = LocalContext.current
    val result by produceState<Result<PdfDocument>?>(null, file) {
        // The attempt owns the document, including when cancellation races with loading.
        var opened: PdfDocument? = null
        try {
            value = try {
                Result.success(openPdf(context, file).also { opened = it })
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                Log.w("PdfReader", "Could not open PDF", error)
                Result.failure(error)
            }
            if (value?.isSuccess == true) awaitCancellation()
        } finally {
            opened?.close()
        }
    }
    val loaded = result?.getOrNull()
    // An id lets PdfView keep its page and zoom across configuration changes.
    val viewId = rememberSaveable { View.generateViewId() }
    when {
        result == null -> Box(Modifier.fillMaxSize(), Alignment.Center) { CircularProgressIndicator() }
        loaded == null -> failure()
        else -> AndroidView(
            factory = { viewContext ->
                PdfView(ContextThemeWrapper(viewContext, R.style.Theme_IsctCompanion_PdfReader)).apply {
                    id = viewId
                    addOnAttachStateChangeListener(FastScrollAutoHide(this))
                }
            },
            modifier = Modifier.fillMaxSize().background(Theme.colors.surface2),
            update = { view -> view.pdfDocument = loaded },
        )
    }
}

/** Shows androidx.pdf's page scroller the way other readers do: while the
 * page scrolls, draggable to jump, faded out once it stops.
 *
 * PdfView pins the scroller on screen whenever any accessibility service is
 * on, and many phones always run one (vendor gesture or security services),
 * so the "page 3/10" label never went away. Only a screen reader needs it
 * pinned. The switch is library-internal and PdfView sets it again when a
 * document loads, its state is restored or accessibility changes, so before
 * each frame a pinned scroller is set back to auto-hide unless touch
 * exploration (TalkBack and the like) is on. */
@SuppressLint("RestrictedApi")
private class FastScrollAutoHide(private val view: PdfView) :
    ViewTreeObserver.OnPreDrawListener, View.OnAttachStateChangeListener {
    private val accessibility = view.context.getSystemService(AccessibilityManager::class.java)

    override fun onPreDraw(): Boolean {
        if (view.fastScrollVisibility == PdfView.FastScrollVisibility.ALWAYS_SHOW &&
            accessibility?.isTouchExplorationEnabled != true
        ) {
            view.fastScrollVisibility = PdfView.FastScrollVisibility.AUTO_HIDE
        }
        return true
    }

    override fun onViewAttachedToWindow(v: View) = v.viewTreeObserver.addOnPreDrawListener(this)

    override fun onViewDetachedFromWindow(v: View) = v.viewTreeObserver.removeOnPreDrawListener(this)
}

/** A downloaded PDF pushed full screen, its file still offered to another
 * app from the bar. */
@Composable
fun PdfScreen(path: String, title: String) {
    val navigator = LocalNavigator.current
    val context = LocalContext.current
    val file = File(path)
    var attempt by remember(path) { mutableIntStateOf(0) }
    TabPage(
        title.ifEmpty { file.name },
        navigationIcon = {
            IconButton(onClick = { navigator.back() }) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, t("common.back")) }
        },
        actions = {
            IconButton(onClick = { openLocalFile(context, file) }) {
                Icon(Icons.AutoMirrored.Rounded.OpenInNew, t("common.openInAnotherApp"))
            }
        },
        compact = true,
    ) {
        key(attempt) {
            PdfReader(file) {
                Box(Modifier.fillMaxSize(), Alignment.Center) {
                    EmptyState(
                        Icons.Rounded.Description,
                        t("error.unavailable"),
                        action = t("common.retry") to { attempt++ },
                    )
                }
            }
        }
    }
}
