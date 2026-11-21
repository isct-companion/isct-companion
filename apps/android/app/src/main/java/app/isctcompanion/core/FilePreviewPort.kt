package app.isctcompanion.core

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.webkit.MimeTypeMap
import androidx.core.content.FileProvider
import java.io.File

/** Whether the app shows this viewer itself; the rest open in another app.
 * On Android Core names media and office files EXTERNAL, so VIDEO, AUDIO and
 * DOCUMENT never arrive. */
fun showsInApp(viewer: Viewer): Boolean = when (viewer) {
    Viewer.PDF, Viewer.MARKDOWN, Viewer.TEXT, Viewer.IMAGE -> true
    Viewer.VIDEO, Viewer.AUDIO, Viewer.DOCUMENT, Viewer.TOO_LARGE, Viewer.EXTERNAL -> false
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
