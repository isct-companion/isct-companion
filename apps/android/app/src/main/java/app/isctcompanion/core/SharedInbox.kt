package app.isctcompanion.core

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.io.File
import java.io.IOException
import java.util.UUID
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.isActive
import kotlinx.coroutines.withContext

/** Owns only the current selection, copied while its content URI grants last. */
object SharedInbox {
    data class Target(val assignmentId: String, val title: String)
    data class Item(val file: File, val name: String)
    data class Selection(val folder: File, val files: List<Item>, val target: Target?)

    private var generation = 0L
    var selection: Selection? by mutableStateOf(null)
        private set
    var working: Boolean by mutableStateOf(false)
        private set
    var error: String? by mutableStateOf(null)
        private set

    suspend fun receive(context: Context, uris: List<Uri>, target: Target? = null) {
        if (uris.isEmpty()) return
        val current = ++generation
        val folder = File(context.cacheDir, "hand-in-${UUID.randomUUID()}")
        var adopted = false
        try {
            val files = withContext(Dispatchers.IO) {
                if (!folder.mkdir()) throw IOException("Could not create a temporary folder")
                uris.map { uri ->
                    val name = displayName(context, uri)
                    val file = File(folder, "${UUID.randomUUID()}-$name")
                    val input = context.contentResolver.openInputStream(uri)
                        ?: throw IOException("Could not read $name")
                    input.use { source -> file.outputStream().use { source.copyTo(it) } }
                    Item(file, name)
                }
            }
            if (generation != current) return
            selection?.folder?.deleteRecursively()
            selection = Selection(folder, files, target)
            adopted = true
        } catch (failure: CancellationException) {
            throw failure
        } catch (failure: Exception) {
            if (generation == current) error = AppModel.describe(failure)
        } finally {
            if (!adopted) folder.deleteRecursively()
        }
    }

    fun discard(item: Selection) {
        if (selection != item) return
        generation += 1
        selection = null
        item.folder.deleteRecursively()
    }

    suspend fun give(item: Selection, target: Target, model: AppModel): WebRouter.PushedPage? {
        if (working || selection != item) return null
        working = true
        try {
            // Finish reading Core's receipt even if the picker is dismissed.
            val page = withContext(NonCancellable) { model.handIn(item, target) }
            if (!currentCoroutineContext().isActive || selection != item) {
                page.uploadId?.let(model.web::cancelHandIn)
                return null
            }
            discard(item)
            return page
        } finally {
            working = false
        }
    }

    fun dismissError() { error = null }

    /** The sender's file name without folders. */
    fun displayName(context: Context, uri: Uri): String {
        val queried = runCatching {
            context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
                if (cursor.moveToFirst()) cursor.getString(0) else null
            }
        }.getOrNull()
        val name = (queried ?: uri.lastPathSegment ?: "file").substringAfterLast('/').trim()
        return if (name.isEmpty() || name == "." || name == "..") "file" else name
    }
}
