package app.isctcompanion.widgets

import android.content.Context
import android.net.Uri
import org.json.JSONObject
import java.io.File
import java.nio.file.NoSuchFileException
import java.nio.file.Files
import java.time.Instant
import java.time.ZoneId

internal enum class Purpose(val entries: String, val label: String) {
    Courses("courseEntries", "widget.courses"),
    Assignments("assignmentEntries", "widget.assignments"),
}

internal fun widgetFile(context: Context) = File(context.filesDir, "Widgets/current.json")

internal fun File.current(): String? = try {
    Files.readString(toPath())
} catch (_: NoSuchFileException) {
    null
}

internal data class WidgetRow(
    val title: String,
    val detail: String?,
    val color: String?,
    val starts: Instant?,
    val ends: Instant?,
    val dated: Boolean,
    val overdue: Boolean,
    val destination: Uri,
)

internal data class WidgetDisplay(
    val locale: String?,
    val zone: ZoneId,
    val destination: Uri,
    val primary: WidgetRow? = null,
    val additional: List<WidgetRow> = emptyList(),
    val message: String? = null,
    val updated: Instant? = null,
)

internal object WidgetFile {
    private const val SCHEMA_VERSION = 1
    private val tokyo = ZoneId.of("Asia/Tokyo")
    private val sync = Uri.parse("isctcompanion://widget/sync")

    fun read(context: Context, purpose: Purpose, now: Instant): WidgetDisplay =
        runCatching {
            val raw = widgetFile(context).current() ?: return unavailable()
            val file = JSONObject(raw)
            check(file.getInt("schemaVersion") == SCHEMA_VERSION)
            val locale = file.getString("locale")
            val zone = ZoneId.of(file.getString("schoolTimeZone"))
            val terminal = file.getJSONObject("terminalContent")
            val start = Instant.parse(file.getString("coverageStart"))
            val end = Instant.parse(file.getString("coverageEnd"))
            if (now < start || now >= end) {
                return WidgetDisplay(locale, zone, terminal.destination(), message = terminal.getString("messageKey"))
            }
            val entries = file.getJSONArray(purpose.entries)
            val entry = (entries.length() - 1 downTo 0).asSequence().map(entries::getJSONObject)
                .firstOrNull { Instant.parse(it.getString("effectiveAt")) <= now }
                ?.getJSONObject("content") ?: return unavailable()
            val freshness = entry.getJSONObject("freshness")
            val updated = if (freshness.getString("availability") == "fresh") null else freshness.time("updatedAt")
            val complete = freshness.getBoolean("coverageComplete")
            val destination = entry.destination()
            when (purpose) {
                Purpose.Courses -> {
                    val date = entry.getString("date")
                    val primary = entry.optJSONObject("primary")?.course(date)
                    val upcoming = entry.rows("upcoming") { course(date) }
                    WidgetDisplay(
                        locale, zone, destination, primary, upcoming,
                        if (primary != null) null else if (complete) "widget.noClasses" else "widget.openToUpdate",
                        updated,
                    )
                }
                Purpose.Assignments -> {
                    val rows = entry.rows("rows") { assignment() }
                    WidgetDisplay(
                        locale, zone, destination, rows.firstOrNull(), rows.drop(1),
                        if (rows.isNotEmpty()) null else if (complete) "widget.noAssignments" else "widget.openToUpdate",
                        updated,
                    )
                }
            }
        }.getOrElse { unavailable() }

    private fun unavailable() = WidgetDisplay(null, tokyo, sync, message = "widget.openToUpdate")

    private fun JSONObject.course(day: String) = WidgetRow(
        getString("title"), optional("room"), getString("color"), time("startsAt"), time("endsAt"),
        getString("date") != day, false, destination(),
    )

    private fun JSONObject.assignment() = WidgetRow(
        getString("title"), optional("courseTitle"), optional("color"), time("dueAt"), null,
        true, getBoolean("overdue"), destination(),
    )

    private fun JSONObject.time(key: String) = optional(key)?.let(Instant::parse)

    private fun JSONObject.optional(key: String): String? =
        if (isNull(key)) null else getString(key).ifEmpty { null }

    private inline fun JSONObject.rows(key: String, row: JSONObject.() -> WidgetRow): List<WidgetRow> {
        val array = getJSONArray(key)
        return List(array.length()) { index -> array.getJSONObject(index).row() }
    }

    private fun JSONObject.destination(): Uri {
        val destination = getJSONObject("destination")
        val kind = destination.getString("kind")
        val uri = Uri.Builder().scheme("isctcompanion").authority("widget").appendPath(kind)
        when (kind) {
            "course" -> uri.appendQueryParameter("key", destination.getString("key"))
                .appendQueryParameter("date", destination.getString("date"))
            "assignment" -> uri.appendQueryParameter("id", destination.getString("id"))
            "timetable", "assignments", "sync" -> Unit
            else -> error("Unknown widget destination")
        }
        return uri.build()
    }
}
