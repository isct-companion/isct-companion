package app.isctcompanion.widgets

import android.content.Context
import app.isctcompanion.core.NativeWidgets
import java.io.FileOutputStream
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking

/** This class is reached only from a main-app Session, never from a widget. */
internal class WidgetStorage(private val context: Context) : NativeWidgets {
    private val file = widgetFile(context)

    override fun current(): String? = file.current()

    override fun replace(json: String) {
        Files.createDirectories(file.toPath().parent)
        val temporary = file.toPath().resolveSibling("staged.json")
        try {
            FileOutputStream(temporary.toFile()).use {
                it.write(json.encodeToByteArray())
                it.fd.sync()
            }
            Files.move(temporary, file.toPath(), StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
        } finally {
            Files.deleteIfExists(temporary)
        }
    }

    override fun clear() {
        Files.deleteIfExists(file.toPath())
        Files.deleteIfExists(file.toPath().resolveSibling("staged.json"))
    }

    override fun reload(courses: Boolean, assignments: Boolean) {
        // Core calls on a blocking worker. Enqueue Glance work before the
        // current WorkManager opportunity can end.
        runBlocking(Dispatchers.IO) {
            refreshWidgets(context, courses, assignments)
        }
    }
}
