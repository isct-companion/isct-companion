package app.isctcompanion.design

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Color as AndroidColor
import android.graphics.pdf.PdfRenderer
import android.os.ParcelFileDescriptor
import android.webkit.MimeTypeMap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import app.isctcompanion.core.Viewer
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

// The app's own file viewers, one per `Viewer` Core decides (`file_preview`).
// Core says how a file shows; these only draw it.

/** Whether the app shows this viewer itself; the rest open in another app.
 * On Android Core names media and office files EXTERNAL, so VIDEO, AUDIO and
 * DOCUMENT never arrive. */
fun showsInApp(viewer: Viewer): Boolean = when (viewer) {
    Viewer.PDF, Viewer.MARKDOWN, Viewer.TEXT, Viewer.IMAGE -> true
    Viewer.VIDEO, Viewer.AUDIO, Viewer.DOCUMENT, Viewer.TOO_LARGE, Viewer.EXTERNAL -> false
}

/** `file` shown the way Core said; `fallback` for a viewer the app does not
 * draw or a file it cannot read. */
@Composable
fun FileViewer(file: File, viewer: Viewer, fallback: @Composable () -> Unit) {
    when (viewer) {
        Viewer.PDF -> PdfReader(file, fallback)
        Viewer.MARKDOWN -> TextFile(file, fallback) { MarkdownView(it, Modifier.fillMaxSize()) }
        Viewer.TEXT -> TextFile(file, fallback) { text ->
            SelectionContainer {
                Text(
                    text,
                    color = Theme.colors.ink,
                    modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(Theme.gutter),
                )
            }
        }
        Viewer.IMAGE -> ImageReader(file, fallback)
        else -> fallback()
    }
}

/** What a file viewer says when it cannot show the file, with another app
 * to try. */
@Composable
fun FileUnavailable(file: File) {
    val context = LocalContext.current
    Box(Modifier.fillMaxSize(), Alignment.Center) {
        EmptyState(
            Icons.Rounded.Description,
            t("lms.thisFileTypeCantBe"),
            action = t("common.openInDefaultApp") to { openInAnotherApp(context, file) },
        )
    }
}

/** Hands a downloaded file to the app that views its type; false when none
 * can or the file is not one the app shares. */
fun openInAnotherApp(context: Context, file: File): Boolean {
    val uri = runCatching { FileProvider.getUriForFile(context, "${context.packageName}.files", file) }.getOrNull()
        ?: return false
    val type = MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase()) ?: "*/*"
    return try {
        context.startActivity(Intent(Intent.ACTION_VIEW).setDataAndType(uri, type).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
        true
    } catch (_: ActivityNotFoundException) {
        false
    }
}

@Composable
private fun Spinner() {
    Box(Modifier.fillMaxSize(), Alignment.Center) { CircularProgressIndicator() }
}

/** A text file read whole; Core keeps text in the app under its size limit. */
@Composable
private fun TextFile(file: File, fallback: @Composable () -> Unit, content: @Composable (String) -> Unit) {
    val text by produceState<Result<String>?>(null, file) {
        value = withContext(Dispatchers.IO) { runCatching { file.readText() } }
    }
    when (val loaded = text) {
        null -> Spinner()
        else -> loaded.getOrNull()?.let { content(it) } ?: fallback()
    }
}

/** A photo fitted to the screen; pinch zooms, drag pans while zoomed. */
@Composable
fun ImageReader(file: File, fallback: @Composable () -> Unit) {
    val bitmap by produceState<Result<Bitmap>?>(null, file) {
        value = withContext(Dispatchers.IO) {
            runCatching {
                val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                BitmapFactory.decodeFile(file.path, bounds)
                val options = BitmapFactory.Options().apply {
                    inSampleSize = maxOf(1, maxOf(bounds.outWidth, bounds.outHeight) / 3000)
                }
                checkNotNull(BitmapFactory.decodeFile(file.path, options))
            }
        }
    }
    var scale by remember { mutableFloatStateOf(1f) }
    var offset by remember { mutableStateOf(Offset.Zero) }
    val loaded = bitmap ?: return Spinner()
    val shown = loaded.getOrNull() ?: return fallback()
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

/** The most a PDF is zoomed, and the widest a page is rendered (pixels):
 * enough for small print, little enough that the pages on screen fit in
 * memory. */
private const val MAX_ZOOM = 4f
private const val MAX_PAGE_PIXELS = 2000

/**
 * A PDF on the platform's PdfRenderer: every page in one scrolling column,
 * each rendered as it scrolls in, pinch to zoom and drag sideways while
 * zoomed.
 *
 * androidx.pdf has selection and search, but it is a beta whose sandboxed
 * service has failed to start in this app (every PDF spun forever) and whose
 * page scroller needed its internals overridden; PdfRenderer is the system's
 * own renderer, which androidx.pdf wraps too. `.github/workflows/
 * androidx-pdf-watch.yml` opens an issue once androidx.pdf is stable.
 */
@Composable
fun PdfReader(file: File, fallback: @Composable () -> Unit) {
    val document by produceState<Result<PdfPages>?>(null, file) {
        val opened = withContext(Dispatchers.IO) { runCatching { PdfPages.open(file) } }
        value = opened
        try {
            awaitCancellation()
        } finally {
            // A page may still be rendering; it finishes before the file closes.
            withContext(NonCancellable) { opened.getOrNull()?.close() }
        }
    }
    val pages = document ?: return Spinner()
    val pdf = pages.getOrNull() ?: return fallback()
    var zoom by remember(file) { mutableFloatStateOf(1f) }
    BoxWithConstraints(
        Modifier
            .fillMaxSize()
            .background(Theme.colors.surface2)
            // Two fingers zoom; one scrolls, which the column and the
            // sideways scroll below take.
            .pointerInput(Unit) {
                awaitEachGesture {
                    awaitFirstDown(requireUnconsumed = false)
                    do {
                        val event = awaitPointerEvent()
                        if (event.changes.count { it.pressed } >= 2) {
                            zoom = (zoom * event.calculateZoom()).coerceIn(1f, MAX_ZOOM)
                            event.changes.forEach { it.consume() }
                        }
                    } while (event.changes.any { it.pressed })
                }
            },
    ) {
        val viewport = maxWidth
        val pixels = with(LocalDensity.current) { (viewport.toPx() * 1.5f).toInt() }.coerceIn(1, MAX_PAGE_PIXELS)
        Box(Modifier.fillMaxSize().horizontalScroll(rememberScrollState())) {
            LazyColumn(
                Modifier.width(viewport * zoom).fillMaxHeight(),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                itemsIndexed(pdf.ratios) { index, ratio ->
                    PdfPage(pdf, index, ratio, pixels)
                }
            }
        }
    }
}

@Composable
private fun PdfPage(pdf: PdfPages, index: Int, ratio: Float, pixels: Int) {
    val bitmap by produceState<Bitmap?>(null, pdf, index, pixels) {
        value = runCatching { pdf.render(index, pixels) }.getOrNull()
    }
    Box(Modifier.fillMaxWidth().aspectRatio(ratio).background(androidx.compose.ui.graphics.Color.White)) {
        bitmap?.let { Image(it.asImageBitmap(), null, Modifier.fillMaxSize(), contentScale = ContentScale.FillBounds) }
    }
}

/** An open PdfRenderer with its pages' shapes. PdfRenderer opens one page at
 * a time and is not thread-safe, so pages render one after another. */
private class PdfPages(
    private val descriptor: ParcelFileDescriptor,
    private val renderer: PdfRenderer,
    /** Each page's width over its height. */
    val ratios: List<Float>,
) {
    private val lock = Mutex()

    suspend fun render(index: Int, width: Int): Bitmap = lock.withLock {
        withContext(Dispatchers.IO) {
            renderer.openPage(index).use { page ->
                val height = (width.toLong() * page.height / page.width.coerceAtLeast(1)).toInt().coerceAtLeast(1)
                Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888).also {
                    it.eraseColor(AndroidColor.WHITE)
                    page.render(it, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
                }
            }
        }
    }

    suspend fun close() = lock.withLock {
        runCatching { renderer.close() }
        runCatching { descriptor.close() }
    }

    companion object {
        fun open(file: File): PdfPages {
            val descriptor = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
            val renderer = try {
                PdfRenderer(descriptor)
            } catch (error: Exception) {
                descriptor.close()
                throw error
            }
            val ratios = (0 until renderer.pageCount).map { index ->
                renderer.openPage(index).use { it.width.toFloat() / it.height.coerceAtLeast(1) }
            }
            return PdfPages(descriptor, renderer, ratios)
        }
    }
}
