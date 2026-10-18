package app.isctcompanion.core

import org.json.JSONArray
import org.json.JSONObject

/** Rows of the authoritative workspace snapshot, indexed by id. Core read
 * models return ids (course detail, timetable blocks); screens look the rows
 * up here to render them. No decision is made on these rows. */
class WorkspaceRecords(json: String? = null) {
    val courses: Map<String, JSONObject>
    val materials: Map<String, JSONObject>
    val assignments: Map<String, JSONObject>
    /** The complete settings object; forms edit a copy and replace it. */
    val settings: JSONObject

    init {
        val root = json?.let { runCatching { JSONObject(it) }.getOrNull() } ?: JSONObject()
        courses = index(root.optJSONArray("courses"))
        materials = index(root.optJSONArray("materials"))
        assignments = index(root.optJSONArray("assignments"))
        settings = root.optJSONObject("settings") ?: JSONObject()
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

fun JSONObject.number(key: String): Double? =
    if (has(key) && !isNull(key)) optDouble(key).takeUnless { it.isNaN() } else null
