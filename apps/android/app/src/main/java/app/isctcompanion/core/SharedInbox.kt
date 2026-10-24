package app.isctcompanion.core

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.io.File
import java.util.UUID

/**
 * What was shared into the app (the system share sheet's `ACTION_SEND`),
 * waiting to be given to an assignment, as the iOS share extension's inbox.
 * Each file sits in `files/Inbox/<id>/`; the person gives it to an assignment,
 * Core copies it into that assignment's workspace `submit/` and the inbox
 * entry is removed. Nothing here decides anything.
 */
object SharedInbox {
    /** One shared file. */
    data class Item(val id: String, val file: File, val addedAt: Long) {
        val name: String get() = file.name
    }

    var items: List<Item> by mutableStateOf(emptyList())
        private set

    private fun folder(context: Context) = File(context.filesDir, "Inbox")

    /** Re-reads the folder, oldest first. */
    fun refresh(context: Context) {
        items = folder(context).listFiles().orEmpty()
            .filter { it.isDirectory }
            .mapNotNull { dir -> dir.listFiles()?.firstOrNull { it.isFile }?.let { Item(dir.name, it, dir.lastModified()) } }
            .sortedBy { it.addedAt }
    }

    /** Copies shared content in while the sending app's grant lasts. */
    fun receive(context: Context, uris: List<Uri>) {
        for (uri in uris) {
            runCatching {
                val dir = File(folder(context), UUID.randomUUID().toString()).apply { mkdirs() }
                val target = File(dir, displayName(context, uri))
                context.contentResolver.openInputStream(uri)?.use { input ->
                    target.outputStream().use { input.copyTo(it) }
                } ?: dir.deleteRecursively()
            }
        }
        refresh(context)
    }

    fun remove(context: Context, item: Item) {
        item.file.parentFile?.deleteRecursively()
        refresh(context)
    }

    /** The sender's file name without folders; Core makes it safe everywhere. */
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
