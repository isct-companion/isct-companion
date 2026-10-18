package app.isctcompanion.features.course

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.Language
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.CourseDetail
import app.isctcompanion.core.DeadlineItem
import app.isctcompanion.core.MaterialItem
import app.isctcompanion.core.SchoolSite
import app.isctcompanion.core.WebRouter
import app.isctcompanion.core.number
import app.isctcompanion.core.roomMapQuery
import app.isctcompanion.core.text
import app.isctcompanion.design.Card
import app.isctcompanion.design.DeadlineLink
import app.isctcompanion.design.DeadlineRow
import app.isctcompanion.design.Format
import app.isctcompanion.design.MapButton
import app.isctcompanion.design.MaterialRow
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.PageSubtitle
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.Theme
import app.isctcompanion.design.courseColor
import app.isctcompanion.design.tabular
import java.time.Instant
import org.json.JSONObject

private const val LATEST_MATERIALS = 3

private data class Meeting(val day: Int, val start: String, val end: String, val room: String)

/** Course detail: room card with map, open assignments, latest materials and
 * the course's T2SCHOLA and syllabus pages. Ids and links come from Core's
 * `CourseDetail`; rows are looked up in the snapshot to render them. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CourseDetailScreen(courseId: String) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    var detail by remember { mutableStateOf<CourseDetail?>(null) }
    LaunchedEffect(courseId, model.contentRevision) { detail = model.courseDetail(courseId) }
    val course = model.records.courses[courseId] ?: JSONObject()
    val tint = courseColor(model.colors[courseId])
    val title = course.text("title").orEmpty()
    val meeting = meetings(course).firstOrNull()
    val now = Instant.now()
    TabPage(
        title,
        navigationIcon = {
            IconButton(onClick = { navigator.back() }) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, "戻る") }
        },
    ) {
        Column(
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp)
                .padding(bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            PageSubtitle(subtitle(course, meeting))
            meeting?.let { RoomCard(it, tint) }
            val assignments = assignments(courseId)
            if (assignments.isNotEmpty()) {
                SectionLabel("課題")
                Card {
                    assignments.forEachIndexed { index, item ->
                        if (index > 0) RowDivider()
                        DeadlineLink(item) { DeadlineRow(item, now, showsCourse = false) }
                    }
                }
            }
            val materials = materials(courseId, detail)
            if (materials.isNotEmpty()) {
                SectionLabel("資料")
                Card {
                    materials.forEachIndexed { index, item ->
                        if (index > 0) RowDivider()
                        MaterialRow(item)
                    }
                }
            }
            Links(detail, title)
        }
    }
}

private fun meetings(course: JSONObject): List<Meeting> {
    val rows = course.optJSONArray("meetings") ?: return emptyList()
    return (0 until rows.length()).mapNotNull { index ->
        val row = rows.optJSONObject(index) ?: return@mapNotNull null
        val start = row.text("startTime") ?: return@mapNotNull null
        val end = row.text("endTime") ?: return@mapNotNull null
        val day = row.optJSONArray("days")?.optInt(0) ?: 0
        Meeting(day, start, end, row.text("room").orEmpty())
    }
}

@Composable
private fun subtitle(course: JSONObject, meeting: Meeting?): String {
    val model = LocalModel.current
    val days = listOf("日", "月", "火", "水", "木", "金", "土")
    val parts = listOfNotNull(course.text("code")).toMutableList()
    if (meeting != null) {
        val start = model.periods.firstOrNull { it.startTime == meeting.start }?.periods?.firstOrNull()
        val end = model.periods.firstOrNull { it.endTime == meeting.end }?.periods?.lastOrNull()
        val periods = if (start != null && end != null) Format.periods(byteArrayOf(start, end)) else null
        parts.add("${days[meeting.day.coerceIn(0, 6)]} ${periods ?: "${meeting.start}–${meeting.end}"}")
    }
    course.number("credits")?.takeIf { it > 0 }?.let {
        parts.add("${if (it % 1.0 == 0.0) it.toInt().toString() else it.toString()} 単位")
    }
    return parts.joinToString(" · ")
}

@Composable
private fun RoomCard(meeting: Meeting, tint: Color) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Theme.radiusPanel))
            .background(tint.copy(alpha = Theme.tintSoft))
            .padding(18.dp),
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(
                meeting.room.ifEmpty { "教室未定" }, style = MaterialTheme.typography.headlineMedium,
                fontWeight = FontWeight.Bold, color = Theme.colors.ink,
            )
            Text("${meeting.start} – ${meeting.end}", style = MaterialTheme.typography.bodyMedium.tabular(), color = tint)
        }
        roomMapQuery(meeting.room)?.let { MapButton(it, "地図", tint) }
    }
}

/** This course's open work from Core's task timeline, in its order. */
@Composable
private fun assignments(courseId: String): List<DeadlineItem> {
    val today = LocalModel.current.board?.today ?: return emptyList()
    return (today.pinned + today.dueThisWeek + today.later + today.pastRecent).filter { it.courseId == courseId }
}

/** The newest materials: Core orders by section, so the last ids are newest. */
@Composable
private fun materials(courseId: String, detail: CourseDetail?): List<MaterialItem> {
    val records = LocalModel.current.records
    return detail?.materialIds.orEmpty().takeLast(LATEST_MATERIALS).reversed().mapNotNull { id ->
        val row = records.materials[id] ?: return@mapNotNull null
        MaterialItem(
            materialId = id, courseId = courseId, courseTitle = null, color = null,
            title = row.text("title").orEmpty(), kind = row.text("type") ?: "link",
            section = (row.number("sectionNumber") ?: row.number("session"))?.toLong(),
            sectionTitle = row.text("sectionTitle"),
            url = row.text("resourceUrl") ?: row.text("url"),
            updatedAt = row.text("updatedAt"), addedToday = false,
        )
    }
}

@Composable
private fun Links(detail: CourseDetail?, title: String) {
    val navigator = LocalNavigator.current
    val lms = detail?.lmsUrl
    val syllabus = detail?.syllabusUrl
    if (lms == null && syllabus == null) return
    Card(Modifier.padding(top = 8.dp)) {
        if (lms != null) {
            LinkRow("T2SCHOLA で開く", Icons.Outlined.Language) {
                navigator.openPage(WebRouter.PushedPage(SchoolSite.SCIENCE_LMS, lms, title))
            }
        }
        if (lms != null && syllabus != null) RowDivider()
        if (syllabus != null) {
            LinkRow("シラバス", Icons.Outlined.Description) {
                navigator.openPage(WebRouter.PushedPage(SchoolSite.SYLLABUS, syllabus, title))
            }
        }
    }
}

@Composable
private fun LinkRow(title: String, icon: ImageVector, onClick: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 14.dp),
    ) {
        Icon(icon, null, tint = Theme.colors.muted, modifier = Modifier.size(18.dp))
        Text(title, color = Theme.colors.ink, modifier = Modifier.weight(1f))
        Icon(Icons.Outlined.ChevronRight, null, tint = Theme.colors.muted, modifier = Modifier.size(18.dp))
    }
}
