package app.isctcompanion.core

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.webkit.MimeTypeMap
import androidx.core.content.FileProvider
import java.io.File

/** Lets an installed viewer read only this file for the lifetime of the intent. */
internal fun openLocalFile(
    context: Context,
    file: File,
    mimeType: String = MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase()) ?: "*/*",
    displayName: String = file.name,
): Boolean {
    val uri = try {
        FileProvider.getUriForFile(context, "${context.packageName}.files", file, displayName)
    } catch (_: IllegalArgumentException) {
        return false
    }
    return try {
        context.startActivity(
            Intent(Intent.ACTION_VIEW).setDataAndType(uri, mimeType).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION),
        )
        true
    } catch (_: ActivityNotFoundException) {
        false
    }
}
