package app.isctcompanion.core

import org.json.JSONArray
import org.json.JSONObject

/** Rows of the authoritative workspace snapshot, indexed by id. Core read
 * models return ids (course detail, timetable blocks); screens look the rows
 * up here to render them. No decision is made on these rows. */
class WorkspaceRecords(workspace: String? = null, schoolRecords: String? = null) {
    val courses: Map<String, JSONObject>
    val materials: Map<String, JSONObject>
    val assignments: Map<String, JSONObject>
    /** The complete settings object; forms edit a copy and replace it. */
    val settings: JSONObject
    /** `provider:scope` → when Core last finished that import, from the stored runs. */
    val lastRuns: Map<String, String>

    init {
        val root = workspace?.let { runCatching { JSONObject(it) }.getOrNull() } ?: JSONObject()
        courses = index(root.optJSONArray("courses"))
        materials = index(root.optJSONArray("materials"))
        assignments = index(root.optJSONArray("assignments"))
        settings = root.optJSONObject("settings") ?: JSONObject()
        val runs = schoolRecords?.let { runCatching { JSONObject(it) }.getOrNull() }?.optJSONObject("lastRuns")
        lastRuns = buildMap { runs?.keys()?.forEach { key -> runs.text(key)?.let { put(key, it) } } }
    }

    /** When the profile's school data was last read, picked the way the desktop
     * footer does (`usePullState.ts`): LMS runs for Science Tokyo, any 教務Web
     * run for Tokyo Tech. */
    fun lastRun(profile: SchoolAuthProfile): String? = lastRuns
        .filterKeys { if (profile == SchoolAuthProfile.SCIENCE) it == "lms:all" || it == "lms:summary" else it.startsWith("kyomu:") }
        .values.maxOrNull()

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
