package app.isctcompanion.core

import org.json.JSONArray
import org.json.JSONObject

/** Rows of the `records(fields)` view query, indexed by id. Core read
 * models return ids (course detail, timetable blocks); screens look the rows
 * up here to render them. No decision is made on these rows. */
class WorkspaceRecords(workspace: String? = null) {
    val courses: Map<String, JSONObject>
    val assignments: Map<String, JSONObject>
    /** The complete settings object; forms edit a copy and replace it. */
    val settings: JSONObject
    val reminderLeadOptions: ReminderLeadOptions?

    init {
        val root = workspace?.let { runCatching { JSONObject(it) }.getOrNull() } ?: JSONObject()
        courses = index(root.optJSONArray("courses"))
        assignments = index(root.optJSONArray("assignments"))
        settings = root.optJSONObject("settings") ?: JSONObject()
        reminderLeadOptions = root.optJSONObject("reminderLeadOptions")?.let { options ->
            val assignments = options.getJSONObject("assignments")
            ReminderLeadOptions(
                classes = options.getJSONArray("classes").minutes().map(Int::toUInt),
                exams = options.getJSONArray("exams").minutes().map(Int::toUInt),
                assignments = AssignmentLeadOptions(
                    presets = assignments.getJSONArray("presets").minutes().map(Int::toUInt),
                    minSelected = assignments.getInt("minSelected").toUInt(),
                    maxSelected = assignments.getInt("maxSelected").toUInt(),
                ),
            )
        }
    }

    private fun index(rows: JSONArray?): Map<String, JSONObject> = buildMap {
        for (i in 0 until (rows?.length() ?: 0)) {
            val row = rows?.optJSONObject(i) ?: continue
            row.text("id")?.let { put(it, row) }
        }
    }
}

/** A non-empty string field. */
fun JSONObject.text(key: String): String? =
    if (!has(key) || isNull(key)) null else optString(key).ifEmpty { null }

internal fun JSONArray.minutes(): List<Int> = List(length(), ::getInt)
