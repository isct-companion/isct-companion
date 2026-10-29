package app.isctcompanion.core

import app.isctcompanion.design.t
import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import android.widget.Toast
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

    suspend fun save(
        session: Session,
        url: String,
        referer: String?,
        contentDisposition: String?,
        mimeType: String?,
        openExternal: (Uri) -> Unit,
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
                val saved = withContext(Dispatchers.IO) { publish(File(file.path), file.name, file.mimeType) }
                notice(if (saved) t("app.savedName", "name" to file.name) else t("lmsFile.theFileCouldNotBeDownloaded"))
            }
            is SchoolFile.External -> openExternal(Uri.parse(file.url))
            null -> notice(t("lmsFile.theFileCouldNotBeDownloaded"))
        }
    }

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

    private fun notice(text: String) = Toast.makeText(context, text, Toast.LENGTH_SHORT).show()
}
