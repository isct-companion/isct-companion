package app.isctcompanion

import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.pdf.PdfRenderer
import android.media.ExifInterface
import android.net.Uri
import android.os.Bundle
import android.os.CancellationSignal
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.print.PageRange
import android.print.PrintAttributes
import android.print.PrintDocumentAdapter
import android.print.PrintDocumentInfo
import android.print.PrintManager
import android.print.pdf.PrintedPdfDocument
import android.provider.OpenableColumns
import android.util.Log
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.lifecycle.lifecycleScope
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.io.OutputStream
import kotlin.concurrent.thread
import kotlin.math.max
import kotlin.math.min
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** "Print via campus printer" in the system share sheet.
 *
 * HyperOS sends printing from its own apps (Files, the Xiaomi build of WPS)
 * to its own print screen, which lists only the built-in print service's
 * printers, so the campus printer never shows there. A PDF or photo shared
 * here opens Android's own print screen, started by this app, where the
 * campus printer is listed as it is from Chrome. A PDF prints as it is; a
 * photo is fitted onto the page the dialog chose.
 *
 * The shared file is copied first: the sender's read grant ends with this
 * activity, and the print screen may read the document again for its
 * preview. The activity has no window of its own and closes once the print
 * screen does. */
class PrintShareActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val uri = intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
            ?: intent.clipData?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.uri
        if (uri == null) {
            fail()
            return
        }
        lifecycleScope.launch {
            val shared = withContext(Dispatchers.IO) { copy(uri) }
            if (shared == null) {
                fail()
                return@launch
            }
            val adapter = SharedFileAdapter(applicationContext, shared) {
                shared.file.parentFile?.deleteRecursively()
                finish()
            }
            // A wide photo starts on a landscape page; the dialog can change it.
            val start = shared.landscape?.let {
                PrintAttributes.Builder()
                    .setMediaSize(if (it) PrintAttributes.MediaSize.ISO_A4.asLandscape() else PrintAttributes.MediaSize.ISO_A4)
                    .build()
            }
            getSystemService(PrintManager::class.java).print(shared.name, adapter, start)
        }
    }

    private fun fail() {
        Toast.makeText(this, R.string.print_share_failed, Toast.LENGTH_LONG).show()
        finish()
    }

    /** The shared document in this app's cache, or `null` when it cannot be
     * read or is neither a PDF nor an image. Copies older than a day, from a
     * print screen that never closed, are cleared first. */
    private fun copy(uri: Uri): SharedFile? = try {
        val root = File(cacheDir, "print-share")
        root.listFiles()?.filter { System.currentTimeMillis() - it.lastModified() > DAY_MS }?.forEach { it.deleteRecursively() }
        val folder = File(root, System.nanoTime().toString()).apply { mkdirs() }
        val file = File(folder, "document")
        val input = contentResolver.openInputStream(uri) ?: throw IOException("No stream for $uri")
        input.use { source -> file.outputStream().use { source.copyTo(it) } }
        val name = displayName(uri) ?: getString(R.string.print_share_label)
        val type = intent.type?.takeUnless { it.endsWith("/*") } ?: contentResolver.getType(uri).orEmpty()
        when {
            type == "application/pdf" || name.endsWith(".pdf", ignoreCase = true) || isPdf(file) ->
                SharedFile(file, name, landscape = null)
            else -> imageSize(file)?.let { (width, height) -> SharedFile(file, name, landscape = width > height) }
        }.also { if (it == null) folder.deleteRecursively() }
    } catch (error: Exception) {
        Log.w(TAG, "Could not read the shared document", error)
        null
    }

    private fun displayName(uri: Uri): String? =
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) cursor.getString(0)?.takeIf { it.isNotBlank() } else null
        }

    private companion object {
        const val TAG = "isct-print-share"
        const val DAY_MS = 24 * 60 * 60 * 1000L
    }
}

/** A shared document copied into the cache. `landscape` is `null` for a
 * PDF, and for a photo whether it is wider than tall (as it is shown). */
private class SharedFile(val file: File, val name: String, val landscape: Boolean?)

private fun isPdf(file: File): Boolean =
    file.inputStream().use { input -> ByteArray(5).let { input.read(it) == 5 && String(it) == "%PDF-" } }

/** An image's size as shown, with its EXIF rotation applied; `null` when
 * the file is not an image. */
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

/** Hands the copy to the print screen: a PDF's bytes as they are, or a
 * photo drawn onto one page of the size and margins the dialog chose. The
 * writing runs off the main thread; its result is reported back on it. */
private class SharedFileAdapter(
    private val context: Context,
    private val shared: SharedFile,
    private val finished: () -> Unit,
) : PrintDocumentAdapter() {
    private val main = Handler(Looper.getMainLooper())
    private var attributes: PrintAttributes? = null
    private val pageCount: Int by lazy {
        if (shared.landscape != null) return@lazy 1
        try {
            ParcelFileDescriptor.open(shared.file, ParcelFileDescriptor.MODE_READ_ONLY).use { descriptor ->
                PdfRenderer(descriptor).use { it.pageCount }
            }
        } catch (_: Exception) {
            // Encrypted or unusual PDFs: the print screen counts the pages itself.
            PrintDocumentInfo.PAGE_COUNT_UNKNOWN
        }
    }

    override fun onLayout(
        oldAttributes: PrintAttributes?,
        newAttributes: PrintAttributes,
        cancellationSignal: CancellationSignal,
        callback: LayoutResultCallback,
        extras: Bundle?,
    ) {
        if (cancellationSignal.isCanceled) {
            callback.onLayoutCancelled()
            return
        }
        attributes = newAttributes
        val photo = shared.landscape != null
        val info = PrintDocumentInfo.Builder(shared.name)
            .setContentType(if (photo) PrintDocumentInfo.CONTENT_TYPE_PHOTO else PrintDocumentInfo.CONTENT_TYPE_DOCUMENT)
            .setPageCount(pageCount)
            .build()
        // A PDF is the same on any paper; a photo is drawn again for it.
        callback.onLayoutFinished(info, photo && newAttributes != oldAttributes)
    }

    override fun onWrite(
        pages: Array<out PageRange>,
        destination: ParcelFileDescriptor,
        cancellationSignal: CancellationSignal,
        callback: WriteResultCallback,
    ) {
        val chosen = attributes
        thread(name = "print-share") {
            val result = try {
                FileOutputStream(destination.fileDescriptor).use { out ->
                    if (shared.landscape == null) shared.file.inputStream().use { it.copyTo(out) }
                    else writePhoto(context, shared.file, chosen ?: throw IOException("No page chosen"), out)
                }
                null
            } catch (error: Exception) {
                Log.w("isct-print-share", "Could not write the document", error)
                error
            }
            main.post {
                when {
                    cancellationSignal.isCanceled -> callback.onWriteCancelled()
                    result != null -> callback.onWriteFailed(result.message)
                    else -> callback.onWriteFinished(arrayOf(PageRange.ALL_PAGES))
                }
            }
        }
    }

    override fun onFinish() = finished()
}

/** Draws the photo, upright and as large as fits, centred in the page's
 * printable area. */
private fun writePhoto(context: Context, file: File, attributes: PrintAttributes, out: OutputStream) {
    val bitmap = decodeUpright(file) ?: throw IOException("Not an image")
    val document = PrintedPdfDocument(context, attributes)
    try {
        val page = document.startPage(0)
        val area = RectF(page.info.contentRect)
        val scale = min(area.width() / bitmap.width, area.height() / bitmap.height)
        val matrix = Matrix().apply {
            postScale(scale, scale)
            postTranslate(
                area.left + (area.width() - bitmap.width * scale) / 2,
                area.top + (area.height() - bitmap.height * scale) / 2,
            )
        }
        page.canvas.drawBitmap(bitmap, matrix, Paint(Paint.FILTER_BITMAP_FLAG))
        document.finishPage(page)
        document.writeTo(out)
    } finally {
        document.close()
        bitmap.recycle()
    }
}

/** The image at most [MAX_PHOTO_SIDE] pixels on its longer side (plenty for
 * a page at 600 dpi), turned upright by its EXIF orientation. */
private fun decodeUpright(file: File): Bitmap? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeFile(file.path, bounds)
    var sample = 1
    while (max(bounds.outWidth, bounds.outHeight) / (sample * 2) >= MAX_PHOTO_SIDE) sample *= 2
    val decoded = BitmapFactory.decodeFile(file.path, BitmapFactory.Options().apply { inSampleSize = sample }) ?: return null
    val degrees = rotation(file)
    if (degrees == 0) return decoded
    val turned = Bitmap.createBitmap(decoded, 0, 0, decoded.width, decoded.height, Matrix().apply { postRotate(degrees.toFloat()) }, true)
    if (turned !== decoded) decoded.recycle()
    return turned
}

private const val MAX_PHOTO_SIDE = 4000
