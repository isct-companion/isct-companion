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
    /** `dataset/key` → when Core last read that key in full, from its stored freshness. */
    val fetchedAt: Map<String, String>

    init {
        val root = workspace?.let { runCatching { JSONObject(it) }.getOrNull() } ?: JSONObject()
        courses = index(root.optJSONArray("courses"))
        materials = index(root.optJSONArray("materials"))
        assignments = index(root.optJSONArray("assignments"))
        settings = root.optJSONObject("settings") ?: JSONObject()
        val freshness = schoolRecords?.let { runCatching { JSONObject(it) }.getOrNull() }?.optJSONObject("freshness")
        fetchedAt = buildMap {
            freshness?.keys()?.forEach { key -> freshness.optJSONObject(key)?.text("fetchedAt")?.let { put(key, it) } }
        }
    }

    /** When the profile's school data was last read, picked the way Core's
     * school status does: the course list and deadlines for Science Tokyo,
     * any 教務Web page for Tokyo Tech. */
    fun lastRun(profile: SchoolAuthProfile): String? = fetchedAt
        .filterKeys {
            if (profile == SchoolAuthProfile.SCIENCE) it == "lms.enrolment/all" || it == "lms.deadlines/all"
            else it.startsWith("kyomu.page/")
        }
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
