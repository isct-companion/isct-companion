package app.isctcompanion.core

import app.isctcompanion.design.t
import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

/** Files a school page hands over, saved to the public Downloads folder.
 * School hosts are reached only through a WebView or Core (slotledger
 * `constraint/school-network-two-stacks`), so Core fetches the file with the
 * school session and the TLS each host needs, and names it; this moves Core's
 * staged file into MediaStore and says how it went. A file off the school
 * hosts opens in the system browser.
 *
 * The transfer runs in this process, not in a system download service: if
 * Android kills the app mid-download, the file is lost and the person taps
 * the link again. Accepted so that no third network stack reaches the school
 * hosts. */
class SchoolDownloads(private val context: Context) {
    /** Core's staging folder; whatever it holds is moved out at once. */
    private val staging = File(context.cacheDir, "school-downloads")
    /** The last school file opened in the app's viewer. */
    private val viewing = File(context.cacheDir, "school-viewing")

    suspend fun save(
        session: Session,
        url: String,
        referer: String?,
        contentDisposition: String?,
        mimeType: String?,
        openExternal: (Uri) -> Unit,
        openInApp: (File, String, Viewer) -> Unit,
    ) {
        notice(t("common.downloading"))
        val file = try {
            val result = session.saveSchoolFile(url, referer, contentDisposition, mimeType, staging.absolutePath).value()
            (result as? OperationResult.SchoolFile)?.file
        } catch (failure: SdkException.Authentication) {
            return notice(t("failure.signInToYourSchool"))
        } catch (failure: SdkException) {
            null
        }
        when (file) {
            is SchoolFile.Saved -> {
                val staged = File(file.path)
                // WebView shows no PDF: one the app reads opens in its viewer,
                // from a copy of its own, as a browser opens what it saved.
                val shown = if (showsInApp(file.viewer)) withContext(Dispatchers.IO) { keepForViewing(staged, file.name) } else null
                val saved = withContext(Dispatchers.IO) { publish(staged, file.name, file.mimeType) }
                notice(if (saved) t("app.savedName", "name" to file.name) else t("lmsFile.theFileCouldNotBeDownloaded"))
                shown?.let { openInApp(it, file.name, file.viewer) }
            }
            is SchoolFile.External -> openExternal(Uri.parse(file.url))
            null -> notice(t("lmsFile.theFileCouldNotBeDownloaded"))
        }
    }

    /** A copy of a staged file for the app's viewer, in a cache folder
     * emptied each time one is made, so only the last file viewed stays. */
    private fun keepForViewing(file: File, name: String): File? = runCatching {
        viewing.deleteRecursively()
        viewing.mkdirs()
        file.copyTo(File(viewing, File(name).name.ifEmpty { "download" }), overwrite = true)
    }.getOrNull()

    /** Copies a staged file into Downloads and removes it from staging. */
    private fun publish(file: File, name: String, mimeType: String): Boolean {
        val resolver = context.contentResolver
        val pending = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, name)
            put(MediaStore.Downloads.MIME_TYPE, mimeType)
            put(MediaStore.Downloads.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS)
            put(MediaStore.Downloads.IS_PENDING, 1)
        }
        try {
            val uri = runCatching { resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, pending) }.getOrNull()
                ?: return false
            val copied = runCatching {
                resolver.openOutputStream(uri)?.use { output -> file.inputStream().use { it.copyTo(output) } }
                    ?: error("no output stream")
                resolver.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
            }.isSuccess
            if (!copied) resolver.delete(uri, null, null)
            return copied
        } finally {
            file.delete()
        }
    }

    private fun notice(text: String) = app.isctcompanion.design.Notices.show(text)
}
