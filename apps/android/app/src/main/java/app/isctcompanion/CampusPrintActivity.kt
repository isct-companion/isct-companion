package app.isctcompanion

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Color as AndroidColor
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.pdf.PdfDocument
import android.graphics.pdf.PdfRenderer
import android.media.ExifInterface
import android.net.Uri
import android.os.Bundle
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.Print
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Remove
import androidx.compose.material.icons.rounded.RotateRight
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.isctcompanion.core.CampusPrintColor
import app.isctcompanion.core.CampusPrintJob
import app.isctcompanion.core.CampusPrintMedia
import app.isctcompanion.core.CampusPrintMessage
import app.isctcompanion.core.CampusPrintSides
import app.isctcompanion.core.campusPrinterCapabilities
import app.isctcompanion.design.IsctTheme
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import java.io.ByteArrayOutputStream
import java.io.Closeable
import java.io.File
import java.io.IOException
import kotlin.math.max
import kotlin.math.min
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/**
 * The app's own print screen, "Print to campus printer" in the share sheet.
 *
 * A shared PDF or image opens here instead of in the system's print dialog,
 * so the campus printer is reached the same way on every phone, whatever
 * the maker did to that dialog (HyperOS lists only its built-in printers
 * in its own). The person picks pages from the thumbnails or by number,
 * copies, sides and colour; Core checks the account and the printer as the
 * screen opens, keeps only the chosen pages, bills the student ID and
 * sends. The send outlives the screen ([CampusPrintRun]): the person may
 * leave, and a notification tells how it ended.
 *
 * The shared file is copied into the cache first: the sender's read grant
 * ends with this activity, and the thumbnails and the send read it later.
 */
class CampusPrintActivity : ComponentActivity() {
    private val model get() = (application as IsctApplication).model

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        // A new share starts over after a finished print; one still sending
        // keeps the screen on it until it ends.
        if (savedInstanceState == null) CampusPrintRun.forget()
        // A file one of the app's readers handed over, or one shared in.
        val handed = handOver.takeIf { it != null && intent.getStringExtra(EXTRA_HAND_OVER) == it.token }
        val uri = handed?.let { Uri.fromFile(it.file) } ?: sharedUri(intent)
        val name = handed?.name
        setContent {
            IsctTheme(model.records.settings.optString("theme").ifEmpty { "warm" }) {
                PrintScreen(uri, name, onClose = ::finish)
            }
        }
    }

    override fun onStart() {
        super.onStart()
        CampusPrintRun.watching = true
    }

    override fun onStop() {
        CampusPrintRun.watching = false
        super.onStop()
    }

    private fun sharedUri(intent: Intent): Uri? =
        intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
            ?: intent.clipData?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.uri
            ?: intent.data

    /** A file the app holds, named for the person; [token] proves the
     * screen was opened by [print], not by another app naming one of the
     * app's own files. */
    private class HandOver(val file: File, val name: String, val token: String)

    companion object {
        private const val EXTRA_HAND_OVER = "app.isctcompanion.print.HAND_OVER"

        @Volatile
        private var handOver: HandOver? = null

        /** The print screen on a file one of the app's readers shows: the
         * reader's print button. */
        fun print(context: Context, file: File, name: String) {
            val token = java.util.UUID.randomUUID().toString()
            handOver = HandOver(file, name, token)
            context.startActivity(Intent(context, CampusPrintActivity::class.java).putExtra(EXTRA_HAND_OVER, token))
        }
    }
}

/** One send at a time, in the process rather than the screen, so leaving
 * the screen does not stop it. While it runs, [CampusPrintSending] keeps
 * HyperOS from freezing the app; when it ends unseen, a notification says
 * how. */
internal object CampusPrintRun {
    sealed interface State {
        data object Idle : State
        data class Sending(val title: String) : State
        data class Ended(val title: String, val printed: Boolean, val message: String) : State
    }

    private val current = MutableStateFlow<State>(State.Idle)
    val state: StateFlow<State> = current
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

    /** Set while the print screen is on screen; then no notification. */
    @Volatile
    var watching = false

    fun forget() {
        if (current.value is State.Ended) current.value = State.Idle
    }

    fun start(app: IsctApplication, job: CampusPrintJob, document: suspend () -> ByteArray?) {
        if (current.value is State.Sending) return
        current.value = State.Sending(job.title)
        scope.launch {
            CampusPrintSending.hold(app)
            val ended = try {
                val bytes = withContext(Dispatchers.IO) { document() }
                if (bytes == null) {
                    State.Ended(job.title, false, t("campusPrint.window.couldNotReadTheFile"))
                } else {
                    val report = app.model.campusPrint(bytes, job)
                    if (report == null) {
                        State.Ended(job.title, false, t("campusPrint.notice.queueFailed"))
                    } else {
                        State.Ended(job.title, report.printed, text(report.message))
                    }
                }
            } catch (error: CancellationException) {
                throw error
            } catch (error: DocumentTooLarge) {
                State.Ended(job.title, false, tooLargeText())
            } catch (error: Exception) {
                Log.w(TAG, "campus print", error)
                State.Ended(job.title, false, t("campusPrint.notice.queueFailed"))
            } finally {
                CampusPrintSending.release(app)
            }
            current.value = ended
            if (!watching) notify(app, ended)
        }
    }

    private fun notify(context: Context, ended: State.Ended) {
        if (context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(RESULT_CHANNEL, t("tools.campusPrinter.campusPrinter"), NotificationManager.IMPORTANCE_DEFAULT),
        )
        val notification = Notification.Builder(context, RESULT_CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(ended.title)
            .setContentText(ended.message)
            .setStyle(Notification.BigTextStyle().bigText(ended.message))
            .setAutoCancel(true)
            .build()
        manager.notify(RESULT_ID, notification)
    }

    private const val TAG = "isct-print"
    private const val RESULT_CHANNEL = "campus-print-result"
    private const val RESULT_ID = 7303
}

/** A document larger than Core takes (`maxDocumentBytes`): not read whole,
 * and not sent. */
private class DocumentTooLarge : IOException()

/** The most Core sends in one job. */
private fun maxDocumentBytes(): Long =
    runCatching { campusPrinterCapabilities().maxDocumentBytes.toLong() }.getOrDefault(64L * 1024 * 1024)

/** Core's words for a document past its limit. */
private fun tooLargeText(): String = t("campusPrint.notice.tooLarge", "megabytes" to maxDocumentBytes() / (1024 * 1024))

/** Core's words for how a print ended or why it would not go through. */
private fun text(message: CampusPrintMessage): String =
    t(message.key, *message.args.map { (name, value) -> Pair<String, Any>(name, value) }.toTypedArray())

// ---- The shared document ----

/** The shared file, copied into the cache. [pages] is 0 when a PDF's pages
 * cannot be counted (one locked with a password): it prints whole. */
private class SharedDocument(
    val file: File,
    val name: String,
    val isPdf: Boolean,
    val pages: Int,
    /** For an image: whether it is wider than tall, as shown. */
    val landscape: Boolean,
)

private sealed interface Loaded {
    data object Loading : Loaded
    data class Ready(val document: SharedDocument) : Loaded
    /** Not a PDF or an image, or nothing was shared. */
    data object Unsupported : Loaded
}

private fun load(context: Context, uri: Uri?, given: String?): Loaded {
    if (uri == null) return Loaded.Unsupported
    return try {
        val root = File(context.cacheDir, "print-share")
        root.listFiles()?.filter { System.currentTimeMillis() - it.lastModified() > DAY_MS }?.forEach { it.deleteRecursively() }
        val folder = File(root, System.nanoTime().toString()).apply { mkdirs() }
        val file = File(folder, "document")
        val input = context.contentResolver.openInputStream(uri) ?: throw IOException("No stream for $uri")
        input.use { source -> file.outputStream().use { source.copyTo(it) } }
        val name = given?.takeIf { it.isNotBlank() } ?: context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) cursor.getString(0)?.takeIf { it.isNotBlank() } else null
        } ?: "document"
        if (isPdf(file)) {
            val pages = runCatching {
                ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY).use { descriptor ->
                    PdfRenderer(descriptor).use { it.pageCount }
                }
            }.getOrDefault(0)
            Loaded.Ready(SharedDocument(file, name, isPdf = true, pages = pages, landscape = false))
        } else {
            val size = imageSize(file)
            if (size == null) {
                folder.deleteRecursively()
                Loaded.Unsupported
            } else {
                Loaded.Ready(SharedDocument(file, name, isPdf = false, pages = 1, landscape = size.first > size.second))
            }
        }
    } catch (error: Exception) {
        Log.w("isct-print", "Could not read the shared document", error)
        Loaded.Unsupported
    }
}

private fun isPdf(file: File): Boolean = file.inputStream().use { input ->
    val head = ByteArray(1024)
    val read = input.read(head).coerceAtLeast(0)
    String(head, 0, read, Charsets.ISO_8859_1).contains("%PDF-")
}

/** An image's size as shown (EXIF rotation applied); `null` if not an image. */
private fun imageSize(file: File): Pair<Int, Int>? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeFile(file.path, bounds)
    if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
    return if (rotation(file) % 180 == 0) bounds.outWidth to bounds.outHeight else bounds.outHeight to bounds.outWidth
}

private fun rotation(file: File): Int = try {
    when (ExifInterface(file.path).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL)) {
        ExifInterface.ORIENTATION_ROTATE_90 -> 90
        ExifInterface.ORIENTATION_ROTATE_180 -> 180
        ExifInterface.ORIENTATION_ROTATE_270 -> 270
        else -> 0
    }
} catch (_: IOException) {
    0
}

/** The image at most [maxSide] pixels on its longer side, upright and
 * then turned [turn] degrees clockwise more. */
private fun decodeUpright(file: File, maxSide: Int, turn: Int = 0): Bitmap? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeFile(file.path, bounds)
    var sample = 1
    while (max(bounds.outWidth, bounds.outHeight) / (sample * 2) >= maxSide) sample *= 2
    val decoded = BitmapFactory.decodeFile(file.path, BitmapFactory.Options().apply { inSampleSize = sample }) ?: return null
    val degrees = (rotation(file) + turn) % 360
    if (degrees == 0) return decoded
    val turned = Bitmap.createBitmap(decoded, 0, 0, decoded.width, decoded.height, Matrix().apply { postRotate(degrees.toFloat()) }, true)
    if (turned !== decoded) decoded.recycle()
    return turned
}

/** How an image is put on its page: the paper, which way the page lies,
 * and how far the person turned the image. */
private data class ImageLayout(val media: CampusPrintMedia, val landscape: Boolean, val turn: Int) {
    /** The page in points. */
    val page: Pair<Int, Int>
        get() {
            val (width, height) = when (media) {
                CampusPrintMedia.A4 -> 595 to 842
                CampusPrintMedia.LETTER -> 612 to 792
            }
            return if (landscape) height to width else width to height
        }
}

/** The bytes to send: a PDF as it is, read at most one byte past Core's
 * limit so a larger one is refused ([DocumentTooLarge]) without being read
 * whole; an image as [layout] puts it, as large as fits inside the
 * printer's margin and centred. */
private fun documentBytes(document: SharedDocument, layout: ImageLayout, marginMils: Int): ByteArray? {
    if (document.isPdf) {
        val limit = maxDocumentBytes()
        val bytes = document.file.inputStream().use { input -> input.readNBytes((limit + 1).coerceAtMost(Int.MAX_VALUE.toLong()).toInt()) }
        if (bytes.size > limit) throw DocumentTooLarge()
        return bytes
    }
    val bitmap = decodeUpright(document.file, MAX_PHOTO_SIDE, layout.turn) ?: return null
    val (pageWidth, pageHeight) = layout.page
    val margin = marginMils * 72f / 1000f
    val pdf = PdfDocument()
    try {
        val page = pdf.startPage(PdfDocument.PageInfo.Builder(pageWidth, pageHeight, 1).create())
        val scale = min((pageWidth - 2 * margin) / bitmap.width, (pageHeight - 2 * margin) / bitmap.height)
        val drawnWidth = bitmap.width * scale
        val drawnHeight = bitmap.height * scale
        val left = (pageWidth - drawnWidth) / 2
        val top = (pageHeight - drawnHeight) / 2
        page.canvas.drawBitmap(bitmap, null, RectF(left, top, left + drawnWidth, top + drawnHeight), Paint(Paint.FILTER_BITMAP_FLAG))
        pdf.finishPage(page)
        return ByteArrayOutputStream().also { pdf.writeTo(it) }.toByteArray()
    } finally {
        pdf.close()
        bitmap.recycle()
    }
}

/** Page pictures for the thumbnails, drawn one at a time (a renderer
 * keeps one page open) off the main thread. */
private class Thumbnails(private val document: SharedDocument) : Closeable {
    private val lock = Mutex()
    private var descriptor: ParcelFileDescriptor? = null
    private var renderer: PdfRenderer? = null

    suspend fun page(index: Int, widthPx: Int): ImageBitmap? = lock.withLock {
        withContext(Dispatchers.IO) {
            runCatching {
                val open = renderer ?: ParcelFileDescriptor.open(document.file, ParcelFileDescriptor.MODE_READ_ONLY).let { opened ->
                    descriptor = opened
                    PdfRenderer(opened).also { renderer = it }
                }
                open.openPage(index).use { page ->
                    val heightPx = (widthPx.toFloat() * page.height / page.width).toInt().coerceAtLeast(1)
                    val bitmap = Bitmap.createBitmap(widthPx, heightPx, Bitmap.Config.ARGB_8888)
                    bitmap.eraseColor(AndroidColor.WHITE)
                    page.render(bitmap, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
                    bitmap.asImageBitmap()
                }
            }.getOrNull()
        }
    }

    override fun close() {
        runCatching { renderer?.close() }
        runCatching { descriptor?.close() }
    }
}

// ---- Pages ----

/** "1-3, 5" as page numbers, or `null` if it is not a valid list within
 * [total] pages. Spaces, full-width commas and en dashes are taken. */
private fun parsePages(text: String, total: Int): Set<Int>? {
    val pages = sortedSetOf<Int>()
    for (part in text.split(Regex("[,，、\\s]+"))) {
        if (part.isEmpty()) continue
        val match = Regex("^(\\d+)(?:[-–~〜](\\d+))?$").matchEntire(part) ?: return null
        val first = match.groupValues[1].toIntOrNull() ?: return null
        val last = match.groupValues[2].ifEmpty { match.groupValues[1] }.toIntOrNull() ?: return null
        if (first < 1 || last > total || first > last) return null
        pages.addAll(first..last)
    }
    return pages.ifEmpty { null }
}

/** Page numbers as runs: {1, 2, 3, 5} is "1-3, 5". */
private fun formatPages(pages: Set<Int>): String {
    val sorted = pages.sorted()
    val parts = mutableListOf<String>()
    var index = 0
    while (index < sorted.size) {
        var end = index
        while (end + 1 < sorted.size && sorted[end + 1] == sorted[end] + 1) end++
        parts += if (end > index) "${sorted[index]}-${sorted[end]}" else "${sorted[index]}"
        index = end + 1
    }
    return parts.joinToString(", ")
}

// ---- The screen ----

private sealed interface PrinterCheck {
    data object Checking : PrinterCheck
    data class Ready(val studentId: String) : PrinterCheck
    data class Problem(val message: String) : PrinterCheck
}

@Composable
private fun PrintScreen(uri: Uri?, name: String?, onClose: () -> Unit) {
    val context = LocalContext.current
    val app = context.applicationContext as IsctApplication
    val run by CampusPrintRun.state.collectAsState()
    val loaded by produceState<Loaded>(Loaded.Loading, uri) {
        value = withContext(Dispatchers.IO) { load(context, uri, name) }
    }
    Column(
        Modifier
            .fillMaxSize()
            .background(Theme.colors.bg)
            .safeDrawingPadding(),
    ) {
        TopBar(onClose)
        when (val current = run) {
            is CampusPrintRun.State.Sending -> SendingView(current.title, onClose)
            is CampusPrintRun.State.Ended -> EndedView(current, onClose, onAgain = { CampusPrintRun.forget() })
            CampusPrintRun.State.Idle -> when (val document = loaded) {
                Loaded.Loading -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator(color = Theme.colors.accent)
                }
                Loaded.Unsupported -> Message(Icons.Rounded.ErrorOutline, t("campusPrint.screen.otherFiles"), Theme.colors.warn)
                is Loaded.Ready -> SettingsView(app, document.document)
            }
        }
    }
}

@Composable
private fun TopBar(onClose: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .height(56.dp)
            .padding(horizontal = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconButton(onClick = onClose) {
            Icon(Icons.Rounded.Close, contentDescription = t("common.close"), tint = Theme.colors.ink)
        }
        Text(
            t("campusPrint.window.printToCampusPrinter"),
            modifier = Modifier.weight(1f),
            textAlign = TextAlign.Center,
            fontSize = 17.sp,
            fontWeight = FontWeight.Bold,
            color = Theme.colors.ink,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Spacer(Modifier.width(48.dp))
    }
}

@Composable
private fun ColumnScope.SettingsView(app: IsctApplication, document: SharedDocument) {
    val capabilities = remember { runCatching { campusPrinterCapabilities() }.getOrNull() }
    val maxCopies = capabilities?.maxCopies?.toInt() ?: 99
    val total = document.pages
    var copies by rememberSaveable { mutableIntStateOf(1) }
    var sides by rememberSaveable { mutableStateOf(capabilities?.defaultSides ?: CampusPrintSides.LONG_EDGE) }
    var color by rememberSaveable { mutableStateOf(capabilities?.defaultColor ?: CampusPrintColor.MONO) }
    var media by rememberSaveable { mutableStateOf(capabilities?.media?.firstOrNull() ?: CampusPrintMedia.A4) }
    // An image's page lies the way the image does until the person says
    // otherwise; turning the image turns the page with it.
    var landscape by rememberSaveable { mutableStateOf(document.landscape) }
    var turn by rememberSaveable { mutableIntStateOf(0) }
    val margin = capabilities?.marginMils ?: 167
    var choosing by rememberSaveable { mutableStateOf(false) }
    var pagesText by rememberSaveable { mutableStateOf("") }
    var checkRound by rememberSaveable { mutableIntStateOf(0) }
    var check by remember { mutableStateOf<PrinterCheck>(PrinterCheck.Checking) }
    LaunchedEffect(checkRound) {
        check = PrinterCheck.Checking
        val state = app.model.campusPrinterState()
        val problem = state?.problem
        val studentId = state?.studentId
        // Core answers with a student ID to bill or a reason it would not
        // print; no answer (no session, a failed call) is neither.
        check = when {
            problem != null -> PrinterCheck.Problem(text(problem))
            studentId != null -> PrinterCheck.Ready(studentId)
            else -> PrinterCheck.Problem(t("campusPrint.screen.checkFailed"))
        }
    }

    val canChoose = document.isPdf && total > 1
    val chosen: Set<Int>? = if (choosing && canChoose) parsePages(pagesText, total) else null
    val pagesBad = choosing && canChoose && chosen == null
    val count = when {
        !canChoose -> max(total, 1)
        choosing -> chosen?.size ?: 0
        else -> total
    }
    val sheets = if (sides == CampusPrintSides.ONE_SIDED) count else (count + 1) / 2
    fun toggle(page: Int) {
        if (!choosing) {
            choosing = true
            pagesText = "$page"
            return
        }
        val next = (parsePages(pagesText, total) ?: emptySet()).toMutableSet()
        if (!next.add(page)) next.remove(page)
        pagesText = formatPages(next)
        if (next.size == total) choosing = false
    }

    Column(
        Modifier
            .weight(1f)
            .verticalScroll(rememberScrollState())
            .padding(start = Theme.gutter, end = Theme.gutter, bottom = Theme.gutter),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        StatusLine(check, onCheckAgain = { checkRound++ })

        Panel {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Box(
                    Modifier
                        .size(44.dp)
                        .clip(RoundedCornerShape(12.dp))
                        .background(Theme.colors.accentSoft),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(Icons.Rounded.Description, contentDescription = null, tint = Theme.colors.accent)
                }
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(document.name, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    if (canChoose) {
                        Text(t("campusPrint.screen.allPages", "count" to total), fontSize = 13.sp, color = Theme.colors.muted)
                    }
                }
            }
            if (!document.isPdf) {
                ImagePage(
                    document,
                    ImageLayout(media, landscape, turn),
                    margin,
                    onTurn = {
                        turn = (turn + 90) % 360
                        landscape = !landscape
                    },
                )
            } else if (total > 0) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        when {
                            !canChoose -> ""
                            choosing && chosen != null -> t("campusPrint.screen.chosen", "count" to chosen.size)
                            else -> t("campusPrint.screen.tapToChoose")
                        },
                        modifier = Modifier.weight(1f),
                        fontSize = 12.sp,
                        color = Theme.colors.muted,
                    )
                    if (choosing && canChoose) {
                        Text(
                            t("campusPrint.screen.chooseAll"),
                            modifier = Modifier
                                .clip(RoundedCornerShape(8.dp))
                                .clickable(role = Role.Button) { choosing = false }
                                .padding(horizontal = 8.dp, vertical = 10.dp),
                            fontSize = 13.sp,
                            fontWeight = FontWeight.SemiBold,
                            color = Theme.colors.accent,
                        )
                    }
                }
                PageStrip(document, chosen = if (choosing) chosen.orEmpty() else null, enabled = canChoose, onToggle = ::toggle)
            }
        }

        Panel(padding = 4.dp) {
            OptionRow(t("campusPrint.window.copies"), inline = true) {
                Stepper(copies, onLess = { copies = max(1, copies - 1) }, onMore = { copies = min(maxCopies, copies + 1) })
            }
            if (canChoose) {
                OptionRow(t("campusPrint.screen.pages")) {
                    Segmented(
                        listOf(false to t("campusPrint.screen.allPages", "count" to total), true to t("campusPrint.screen.choosePages")),
                        choosing,
                    ) { choosing = it }
                    if (choosing) {
                        val hint = when {
                            !pagesBad -> null
                            pagesText.isBlank() -> t("campusPrint.screen.pagesEmpty")
                            else -> t("campusPrint.screen.pagesInvalid", "count" to total)
                        }
                        val pagesHint: (@Composable () -> Unit)? = hint?.let { words -> { Text(words) } }
                        OutlinedTextField(
                            value = pagesText,
                            onValueChange = { pagesText = it },
                            label = { Text(t("campusPrint.screen.pagesField")) },
                            placeholder = { Text(t("campusPrint.screen.pagesExample")) },
                            isError = pagesBad,
                            singleLine = true,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii),
                            supportingText = pagesHint,
                            colors = OutlinedTextFieldDefaults.colors(
                                focusedBorderColor = Theme.colors.accent,
                                focusedLabelColor = Theme.colors.accent,
                                cursorColor = Theme.colors.accent,
                            ),
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }
                }
            }
            if (!document.isPdf) {
                OptionRow(t("campusPrint.screen.orientation")) {
                    Segmented(
                        listOf(false to t("campusPrint.screen.portrait"), true to t("campusPrint.screen.landscape")),
                        landscape,
                    ) { landscape = it }
                }
            }
            OptionRow(t("campusPrint.window.sides")) {
                Segmented(
                    listOf(
                        CampusPrintSides.ONE_SIDED to t("campusPrint.window.oneSided"),
                        CampusPrintSides.LONG_EDGE to t("campusPrint.window.longEdge"),
                        CampusPrintSides.SHORT_EDGE to t("campusPrint.window.shortEdge"),
                    ),
                    sides,
                ) { sides = it }
            }
            OptionRow(t("campusPrint.window.color"), last = document.isPdf) {
                Segmented(
                    listOf(CampusPrintColor.MONO to t("campusPrint.window.blackAndWhite"), CampusPrintColor.COLOR to t("campusPrint.window.inColor")),
                    color,
                ) { color = it }
            }
            // A PDF prints on the paper its pages are made for; only an
            // image is laid out on the paper chosen here.
            if (!document.isPdf) {
                OptionRow(t("campusPrint.screen.paper"), last = true) {
                    Segmented(
                        (capabilities?.media ?: listOf(CampusPrintMedia.A4, CampusPrintMedia.LETTER)).map {
                            it to when (it) {
                                CampusPrintMedia.A4 -> "A4"
                                CampusPrintMedia.LETTER -> "Letter"
                            }
                        },
                        media,
                    ) { media = it }
                }
            }
        }

        Text(
            t("campusPrint.screen.rules"),
            modifier = Modifier.padding(horizontal = 4.dp),
            fontSize = 12.sp,
            lineHeight = 18.sp,
            color = Theme.colors.muted,
        )
    }

    // Larger than Core takes: refused here, before anything is read.
    val tooLarge = remember(document.file) { document.isPdf && document.file.length() > maxDocumentBytes() }
    val blocked = pagesBad || tooLarge || check is PrinterCheck.Problem || check is PrinterCheck.Checking
    BottomBar(
        summary = if (tooLarge) tooLargeText() else if (pagesBad || document.isPdf && total == 0) "" else t("campusPrint.screen.summary", "pages" to count, "sheets" to sheets, "copies" to copies),
    ) {
        PrimaryButton(t("campusPrint.window.print"), Icons.Rounded.Print, enabled = !blocked) {
            val job = CampusPrintJob(
                title = document.name,
                copies = copies.toUInt(),
                sides = sides,
                color = color,
                pages = if (choosing && chosen != null) chosen.sorted().map { it.toUInt() } else emptyList(),
            )
            val layout = ImageLayout(media, landscape, turn)
            CampusPrintRun.start(app, job) { documentBytes(document, layout, margin) }
        }
    }
}

@Composable
private fun StatusLine(check: PrinterCheck, onCheckAgain: () -> Unit) {
    when (check) {
        PrinterCheck.Checking -> Pill(Theme.colors.surface, Theme.colors.muted) {
            CircularProgressIndicator(Modifier.size(12.dp), strokeWidth = 2.dp, color = Theme.colors.muted)
            Text(t("campusPrint.screen.checking"), fontSize = 13.sp, color = Theme.colors.muted)
        }
        is PrinterCheck.Ready -> Pill(Theme.colors.okSoft, Theme.colors.ok) {
            Box(Modifier.size(8.dp).clip(CircleShape).background(Theme.colors.ok))
            Text(
                t("tools.campusPrinter.readyToPrint") + " · " + t("tools.campusPrinter.billedTo", "id" to check.studentId),
                fontSize = 13.sp,
                fontWeight = FontWeight.SemiBold,
                color = Theme.colors.ok,
            )
        }
        is PrinterCheck.Problem -> Column(
            Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(Theme.radiusSurface))
                .background(Theme.colors.warnSoft)
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                Icon(Icons.Rounded.ErrorOutline, contentDescription = null, tint = Theme.colors.warn)
                Text(check.message, fontSize = 14.sp, lineHeight = 21.sp, color = Theme.colors.ink, modifier = Modifier.weight(1f))
            }
            Row(
                Modifier
                    .clip(RoundedCornerShape(Theme.radiusControl))
                    .clickable(role = Role.Button, onClick = onCheckAgain)
                    .background(Theme.colors.surface)
                    .padding(horizontal = 14.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                Icon(Icons.Rounded.Refresh, contentDescription = null, tint = Theme.colors.accent, modifier = Modifier.size(18.dp))
                Text(t("tools.campusPrinter.checkAgain"), fontSize = 14.sp, fontWeight = FontWeight.SemiBold, color = Theme.colors.accent)
            }
        }
    }
}

/** The image's page as it will print: the paper the way it lies, the
 * printer's margin, the image fitted inside; and a button to turn it. */
@Composable
private fun ImagePage(document: SharedDocument, layout: ImageLayout, marginMils: Int, onTurn: () -> Unit) {
    val picture by produceState<ImageBitmap?>(null, document.file, layout.turn) {
        value = withContext(Dispatchers.IO) {
            runCatching { decodeUpright(document.file, PREVIEW_SIDE, layout.turn)?.asImageBitmap() }.getOrNull()
        }
    }
    val (pageWidth, pageHeight) = layout.page
    val boxHeight = if (layout.landscape) 150.dp else 210.dp
    val boxWidth = boxHeight * (pageWidth.toFloat() / pageHeight)
    val margin = boxWidth * (marginMils * 72f / 1000f / pageWidth)
    Row(
        Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp, Alignment.CenterHorizontally),
    ) {
        Box(
            Modifier
                .size(boxWidth, boxHeight)
                .shadow(2.dp, RoundedCornerShape(4.dp))
                .clip(RoundedCornerShape(4.dp))
                .background(Color.White)
                .border(1.dp, Theme.colors.line, RoundedCornerShape(4.dp))
                .padding(margin),
            contentAlignment = Alignment.Center,
        ) {
            picture?.let { Image(it, contentDescription = null, contentScale = ContentScale.Fit, modifier = Modifier.fillMaxSize()) }
        }
        Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(4.dp)) {
            RoundIcon(Icons.Rounded.RotateRight, onTurn, t("campusPrint.screen.rotate"))
            Text(t("campusPrint.screen.rotate"), fontSize = 12.sp, color = Theme.colors.muted)
        }
    }
}

@Composable
private fun PageStrip(document: SharedDocument, chosen: Set<Int>?, enabled: Boolean, onToggle: (Int) -> Unit) {
    val thumbnails = remember(document.file) { Thumbnails(document) }
    DisposableEffect(thumbnails) { onDispose { thumbnails.close() } }
    val widthPx = with(LocalDensity.current) { THUMB_WIDTH.roundToPx() }
    LazyRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        items((1..max(document.pages, 1)).toList(), key = { it }) { number ->
            val picture by produceState<ImageBitmap?>(null, number) { value = thumbnails.page(number - 1, widthPx) }
            val on = chosen == null || number in chosen
            val marked = chosen != null && number in chosen
            val label = t("campusPrint.screen.page", "number" to number)
            Column(
                Modifier
                    .alpha(if (on) 1f else 0.45f)
                    .toggleable(value = marked, enabled = enabled, role = Role.Checkbox) { onToggle(number) }
                    .semantics { contentDescription = label }
                    .padding(vertical = 4.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                // The page sits low and left in a box a badge's half larger,
                // so the check mark on its corner is drawn whole.
                Box(Modifier.size(THUMB_WIDTH + BADGE / 2, THUMB_HEIGHT + BADGE / 2)) {
                    val shape = RoundedCornerShape(6.dp)
                    Box(
                        Modifier
                            .align(Alignment.BottomStart)
                            .size(THUMB_WIDTH, THUMB_HEIGHT)
                            .clip(shape)
                            .background(Color.White)
                            .border(if (marked) 2.dp else 1.dp, if (marked) Theme.colors.accent else Theme.colors.line, shape),
                        contentAlignment = Alignment.Center,
                    ) {
                        picture?.let { Image(it, contentDescription = null, contentScale = ContentScale.Fit, modifier = Modifier.fillMaxSize().padding(2.dp)) }
                    }
                    if (marked) {
                        Box(
                            Modifier
                                .align(Alignment.TopEnd)
                                .size(BADGE)
                                .clip(CircleShape)
                                .background(Theme.colors.accent),
                            contentAlignment = Alignment.Center,
                        ) {
                            Icon(Icons.Rounded.Check, contentDescription = null, tint = Theme.colors.accentInk, modifier = Modifier.size(12.dp))
                        }
                    }
                }
                Text(
                    "$number",
                    modifier = Modifier.align(Alignment.Start).width(THUMB_WIDTH),
                    textAlign = TextAlign.Center,
                    fontSize = 11.sp,
                    fontWeight = if (marked) FontWeight.Bold else FontWeight.Normal,
                    color = if (marked) Theme.colors.accent else Theme.colors.muted,
                )
            }
        }
    }
}

@Composable
private fun ColumnScope.SendingView(title: String, onClose: () -> Unit) {
    Column(
        Modifier
            .weight(1f)
            .padding(horizontal = Theme.gutter),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Panel {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                CircularProgressIndicator(Modifier.size(22.dp), strokeWidth = 3.dp, color = Theme.colors.accent)
                Text(t("campusPrint.window.sending"), fontSize = 17.sp, fontWeight = FontWeight.Bold, color = Theme.colors.ink)
            }
            Text(title, fontSize = 14.sp, color = Theme.colors.muted, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
        Text(
            t("campusPrint.screen.leaveNote"),
            modifier = Modifier.padding(horizontal = 4.dp),
            fontSize = 13.sp,
            lineHeight = 19.sp,
            color = Theme.colors.muted,
        )
    }
    BottomBar(summary = "") {
        SecondaryButton(t("campusPrint.screen.inBackground"), onClose)
    }
}

@Composable
private fun ColumnScope.EndedView(ended: CampusPrintRun.State.Ended, onClose: () -> Unit, onAgain: () -> Unit) {
    Column(
        Modifier
            .weight(1f)
            .fillMaxWidth()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 24.dp, vertical = 32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Box(
            Modifier
                .size(88.dp)
                .clip(CircleShape)
                .background(if (ended.printed) Theme.colors.okSoft else Theme.colors.warnSoft),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                if (ended.printed) Icons.Rounded.Check else Icons.Rounded.ErrorOutline,
                contentDescription = null,
                tint = if (ended.printed) Theme.colors.ok else Theme.colors.warn,
                modifier = Modifier.size(44.dp),
            )
        }
        Text(ended.message, fontSize = 18.sp, lineHeight = 26.sp, fontWeight = FontWeight.Bold, color = Theme.colors.ink, textAlign = TextAlign.Center)
        Text(ended.title, fontSize = 14.sp, color = Theme.colors.muted, textAlign = TextAlign.Center, maxLines = 2, overflow = TextOverflow.Ellipsis)
    }
    BottomBar(summary = "", plain = true) {
        PrimaryButton(t("common.done"), null, enabled = true, onClick = onClose)
        SecondaryButton(t("campusPrint.screen.printAgain"), onAgain)
    }
}

@Composable
private fun Message(icon: ImageVector, message: String, tint: Color) {
    Column(
        Modifier
            .fillMaxSize()
            .padding(32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.CenterVertically),
    ) {
        Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(44.dp))
        Text(message, fontSize = 15.sp, lineHeight = 22.sp, color = Theme.colors.ink, textAlign = TextAlign.Center)
    }
}

// ---- Pieces ----

@Composable
private fun Panel(padding: Dp = 16.dp, content: @Composable ColumnScope.() -> Unit) {
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Theme.radiusSurface))
            .background(Theme.colors.surface)
            .padding(horizontal = 16.dp, vertical = padding),
        verticalArrangement = Arrangement.spacedBy(12.dp),
        content = content,
    )
}

@Composable
private fun Pill(background: Color, ink: Color, content: @Composable () -> Unit) {
    Row(
        Modifier
            .clip(RoundedCornerShape(Theme.radiusControl))
            .background(background)
            .padding(horizontal = 12.dp, vertical = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        content()
    }
}

/** One option: its name and its control, beside it ([inline]) or below,
 * with a rule under every row but the [last]. */
@Composable
private fun OptionRow(label: String, inline: Boolean = false, last: Boolean = false, content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.fillMaxWidth()) {
        if (inline) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(label, modifier = Modifier.weight(1f), fontSize = 15.sp, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
                Column { content() }
            }
        } else {
            Column(
                Modifier
                    .fillMaxWidth()
                    .padding(vertical = 12.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Text(label, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
                content()
            }
        }
        if (!last) {
            Box(
                Modifier
                    .fillMaxWidth()
                    .height(1.dp)
                    .background(Theme.colors.line),
            )
        }
    }
}

@Composable
private fun <T> Segmented(options: List<Pair<T, String>>, selected: T, onSelect: (T) -> Unit) {
    val track = RoundedCornerShape(Theme.radiusControl)
    Row(
        Modifier
            .fillMaxWidth()
            .clip(track)
            .background(Theme.colors.line)
            .padding(2.dp)
            .selectableGroup(),
        horizontalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        options.forEach { (value, label) ->
            val on = value == selected
            Box(
                Modifier
                    .weight(1f)
                    .height(44.dp)
                    .then(if (on) Modifier.shadow(1.dp, track) else Modifier)
                    .clip(track)
                    .background(if (on) Theme.colors.surface else Color.Transparent)
                    .selectable(selected = on, role = Role.RadioButton) { onSelect(value) },
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    label,
                    fontSize = 14.sp,
                    fontWeight = if (on) FontWeight.Bold else FontWeight.Medium,
                    color = if (on) Theme.colors.ink else Theme.colors.muted,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.padding(horizontal = 6.dp),
                )
            }
        }
    }
}

@Composable
private fun Stepper(value: Int, onLess: () -> Unit, onMore: () -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        RoundIcon(Icons.Rounded.Remove, onLess)
        Text(
            "$value",
            modifier = Modifier.widthIn(min = 36.dp),
            textAlign = TextAlign.Center,
            fontSize = 17.sp,
            fontWeight = FontWeight.Bold,
            color = Theme.colors.ink,
        )
        RoundIcon(Icons.Rounded.Add, onMore)
    }
}

@Composable
private fun RoundIcon(icon: ImageVector, onClick: () -> Unit, label: String? = null) {
    Box(
        Modifier
            .size(44.dp)
            .clip(CircleShape)
            .background(Theme.colors.surface2)
            .border(1.dp, Theme.colors.line, CircleShape)
            .clickable(role = Role.Button, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = label, tint = Theme.colors.ink, modifier = Modifier.size(20.dp))
    }
}

@Composable
private fun BottomBar(summary: String, plain: Boolean = false, content: @Composable ColumnScope.() -> Unit) {
    Column(
        Modifier
            .fillMaxWidth()
            .background(if (plain) Theme.colors.bg else Theme.colors.surface)
            .padding(start = Theme.gutter, end = Theme.gutter, top = 10.dp, bottom = 12.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        if (summary.isNotEmpty()) {
            Text(summary, modifier = Modifier.fillMaxWidth(), textAlign = TextAlign.Center, fontSize = 13.sp, color = Theme.colors.muted)
        }
        content()
    }
}

@Composable
private fun PrimaryButton(label: String, icon: ImageVector?, enabled: Boolean, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .height(52.dp)
            .clip(RoundedCornerShape(Theme.radiusControl))
            .background(if (enabled) Theme.colors.accent else Theme.colors.line)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val ink = if (enabled) Theme.colors.accentInk else Theme.colors.muted
        if (icon != null) {
            Icon(icon, contentDescription = null, tint = ink, modifier = Modifier.size(20.dp))
            Spacer(Modifier.width(8.dp))
        }
        Text(label, fontSize = 16.sp, fontWeight = FontWeight.Bold, color = ink)
    }
}

@Composable
private fun SecondaryButton(label: String, onClick: () -> Unit) {
    Box(
        Modifier
            .fillMaxWidth()
            .height(52.dp)
            .clip(RoundedCornerShape(Theme.radiusControl))
            .background(Theme.colors.surface)
            .border(1.dp, Theme.colors.line, RoundedCornerShape(Theme.radiusControl))
            .clickable(role = Role.Button, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Text(label, fontSize = 16.sp, fontWeight = FontWeight.Bold, color = Theme.colors.ink)
    }
}

private val THUMB_WIDTH = 52.dp
private val THUMB_HEIGHT = 72.dp
private val BADGE = 18.dp
private const val DAY_MS = 24 * 60 * 60 * 1000L
private const val MAX_PHOTO_SIDE = 4000
private const val PREVIEW_SIDE = 800
