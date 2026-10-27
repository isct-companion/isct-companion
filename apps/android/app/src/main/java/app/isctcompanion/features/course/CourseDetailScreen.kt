package app.isctcompanion.features.course

import app.isctcompanion.design.SkeletonRows
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.PagerState
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.FolderOpen
import app.isctcompanion.design.EmptyState
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Campaign
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.Language
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.AssignmentDetail
import app.isctcompanion.core.CourseContent
import app.isctcompanion.core.CourseDetail
import app.isctcompanion.core.CourseKey
import app.isctcompanion.core.ExamView
import app.isctcompanion.core.LessonContent
import app.isctcompanion.core.LessonItem
import app.isctcompanion.core.LessonItemKind
import app.isctcompanion.core.SchoolSite
import app.isctcompanion.core.SyncScope
import app.isctcompanion.core.SyncView
import app.isctcompanion.core.WebRouter
import app.isctcompanion.core.text
import app.isctcompanion.design.Card
import app.isctcompanion.design.ExamConflictLabel
import app.isctcompanion.design.Format
import app.isctcompanion.design.GuideTip
import app.isctcompanion.design.HtmlText
import app.isctcompanion.design.MaterialButton
import app.isctcompanion.design.PageSubtitle
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import app.isctcompanion.design.tabular
import kotlinx.coroutines.launch
import org.json.JSONObject

/** The course page as the desktop Lecture Desk lays it out: the course, then
 * its lessons (General, 1, 2, …) one at a time, each with its own text (授業
 * 計画), its files, the pages, labels and announcements shown in place, and
 * the assignments set in it, which open in place. Content comes from Core's
 * `CourseContent`; links and the files folder from `CourseDetail`. */
@Composable
fun CourseDetailScreen(courseId: String) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    var detail by remember { mutableStateOf<CourseDetail?>(null) }
    var content by remember { mutableStateOf<CourseContent?>(null) }
    var assignments by remember { mutableStateOf<List<AssignmentDetail>>(emptyList()) }
    LaunchedEffect(courseId, model.contentRevision) {
        val loaded = model.courseDetail(courseId) ?: return@LaunchedEffect
        detail = loaded
        content = model.courseContent(courseId)
        assignments = loaded.assignmentIds.mapNotNull { model.assignmentDetail(it) }
    }
    // Runs once the LMS id is known: Core's planner moves this course's
    // reads ahead while it is on screen.
    val lmsCourseId = detail?.lmsCourseId
    LaunchedEffect(lmsCourseId) { lmsCourseId?.let { model.open(SyncView.Course(it)) } }
    val course = model.records.courses[courseId] ?: JSONObject()
    val title = course.text("title").orEmpty()
    TabPage(
        title,
        navigationIcon = {
            IconButton(onClick = { navigator.back() }) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, t("Back")) }
        },
        onRefresh = { lmsCourseId?.let { model.refresh(SyncScope.Course(it)) } },
    ) {
        Column(
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp)
                .padding(bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                PageSubtitle(subtitle(course))
                for (exam in detail?.exams.orEmpty().filter { it.kind == "exam" || it.kind == "quiz" }) {
                    Text(
                        examLine(exam), style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium,
                        color = Theme.colors.warn,
                        textDecoration = if (exam.status == "cancelled") TextDecoration.LineThrough else null,
                    )
                    exam.conflict?.let { ExamConflictLabel(it) }
                }
                Links(detail, title)
            }
            val lessons = content?.lessons.orEmpty()
            if (lessons.isNotEmpty()) {
                Lessons(courseId, content!!, lessons, assignments, detail?.files) { reread -> content = reread }
            } else if (detail != null) {
                EmptyState(Icons.Rounded.FolderOpen, t("No materials yet"))
            } else {
                // Nothing read yet: rows in the shape of the materials to come.
                SkeletonRows(4)
            }
        }
    }
}

/** The lesson chips and the pages under them, one lesson per page; Core
 * picks where to open (the marked lesson, else the last with material). */
@Composable
private fun Lessons(
    courseId: String,
    content: CourseContent,
    lessons: List<LessonContent>,
    assignments: List<AssignmentDetail>,
    files: CourseKey?,
    reread: (CourseContent) -> Unit,
) {
    val model = LocalModel.current
    val haptics = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()
    val opening = lessons.indexOfFirst { it.section == content.currentSection }.coerceAtLeast(0)
    val pager = rememberPagerState(opening) { lessons.size }
    var expanded by remember { mutableStateOf(emptySet<String>()) }
    GuideTip("lesson-marks", "Today's lesson", "Long-press a lesson to make it the lesson of today's class.")
    LessonChips(lessons, pager, content) { marked ->
        val classAt = content.classAt ?: return@LessonChips
        val sectionId = marked.sectionId ?: return@LessonChips
        scope.launch {
            if (!model.recordLesson(courseId, classAt, sectionId)) return@launch
            haptics.performHapticFeedback(HapticFeedbackType.Confirm)
            model.courseContent(courseId)?.let(reread)
        }
    }
    HorizontalPager(pager, verticalAlignment = Alignment.Top, pageSpacing = 16.dp) { index ->
        val lesson = lessons[index]
        LessonPage(
            lesson,
            assignments.filter { (it.sectionNumber ?: 0) == lesson.section },
            files,
            expanded,
        ) { id, open -> expanded = if (open) expanded + id else expanded - id }
    }
}

/** General, 1, 2, … as on the desktop lesson rail; the chosen one stays in
 * view. A dot marks where the course is (Core's `currentSection`); a long
 * press marks a lesson as the one for the coming (or today's) class. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun LessonChips(lessons: List<LessonContent>, pager: PagerState, content: CourseContent, mark: (LessonContent) -> Unit) {
    val scope = rememberCoroutineScope()
    val row = rememberLazyListState()
    LaunchedEffect(pager.currentPage) { row.animateScrollToItem(maxOf(pager.currentPage - 2, 0)) }
    // `11/27` in Tokyo time.
    val classDay = Format.date(content.classAt)?.let { Format.tokyo(it) }?.let { "${it.monthValue}/${it.dayOfMonth}" }
    LazyRow(state = row, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        itemsIndexed(lessons, key = { _, lesson -> lesson.section }) { index, lesson ->
            val selected = index == pager.currentPage
            var menu by remember { mutableStateOf(false) }
            Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Box {
                    Text(
                        chipLabel(lesson),
                        style = MaterialTheme.typography.bodyMedium.tabular(), fontWeight = FontWeight.SemiBold,
                        color = if (selected) Theme.colors.accentInk else Theme.colors.ink,
                        modifier = Modifier
                            .clip(CircleShape)
                            .background(if (selected) Theme.colors.accent else Theme.colors.surface)
                            .combinedClickable(
                                onClick = { scope.launch { pager.animateScrollToPage(index) } },
                                onLongClick = { if (classDay != null && lesson.sectionId != null) menu = true },
                            )
                            .defaultMinSize(minWidth = 34.dp)
                            .padding(horizontal = 10.dp, vertical = 7.dp),
                    )
                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                        DropdownMenuItem(
                            text = { Text(t("Make this the {{day}} class", "day" to classDay.orEmpty())) },
                            leadingIcon = { Icon(Icons.Rounded.CheckCircle, null) },
                            onClick = {
                                menu = false
                                mark(lesson)
                            },
                        )
                    }
                }
                Box(
                    Modifier.size(5.dp).clip(CircleShape)
                        .background(if (lesson.section == content.currentSection) Theme.colors.accent else Color.Transparent),
                )
            }
        }
    }
}

/** The lesson number when the course numbers its lessons, else a short form
 * of the section title. */
private fun chipLabel(lesson: LessonContent): String {
    lesson.number?.let { return "$it" }
    if (lesson.section == 0) return t("General")
    val title = lesson.title ?: "${lesson.section}"
    return if (title.length > 8) title.take(7) + "…" else title
}

/** One lesson's whole content: its title, its text (授業計画), its items in
 * LMS order, and the assignments set in it. */
@Composable
private fun LessonPage(
    lesson: LessonContent,
    work: List<AssignmentDetail>,
    files: CourseKey?,
    expanded: Set<String>,
    expand: (String, Boolean) -> Unit,
) {
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(14.dp)) {
        lesson.title?.takeIf { it.isNotEmpty() }?.let {
            Text(it, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
        }
        if (lesson.summaryHtml.isNotEmpty()) Surface { HtmlText(lesson.summaryHtml) }
        for (group in groups(lesson.items)) {
            val item = group.first()
            if (group.size == 1 && item.bodyHtml != null) {
                InlineItem(item)
            } else {
                Card {
                    group.forEachIndexed { index, row ->
                        if (index > 0) RowDivider()
                        MaterialButton(row.title, kind(row.kind), row.source, files)
                    }
                }
            }
        }
        if (work.isNotEmpty()) {
            Text(
                t("Assignments"), style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.SemiBold,
                color = Theme.colors.muted, modifier = Modifier.padding(horizontal = 16.dp),
            )
            for (assignment in work) {
                AssignmentCard(assignment, assignment.assignmentId in expanded) { expand(assignment.assignmentId, it) }
            }
        }
        if (lesson.items.isEmpty() && lesson.summaryHtml.isEmpty() && work.isEmpty()) {
            EmptyState(Icons.Rounded.Description, t("No materials for this lesson"))
        }
    }
}

/** Items in LMS order: text shown in place stands on its own; runs of files
 * and links share one card. */
private fun groups(items: List<LessonItem>): List<List<LessonItem>> {
    val groups = mutableListOf<MutableList<LessonItem>>()
    for (item in items) {
        if (groups.isEmpty() || item.bodyHtml != null || groups.last().last().bodyHtml != null) {
            groups.add(mutableListOf(item))
        } else {
            groups.last().add(item)
        }
    }
    return groups
}

private fun kind(kind: LessonItemKind): String = when (kind) {
    LessonItemKind.FILE -> "resource"
    LessonItemKind.FOLDER -> "folder"
    LessonItemKind.LINK, LessonItemKind.OTHER -> "link"
    LessonItemKind.PAGE -> "page"
    LessonItemKind.LABEL -> "label"
    LessonItemKind.FORUM -> "forum"
}

/** A page, label or announcement shown in place, as on the desktop. */
@Composable
private fun InlineItem(item: LessonItem) {
    Surface {
        when (item.kind) {
            LessonItemKind.PAGE -> Text(item.title, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
            LessonItemKind.FORUM -> Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Icon(Icons.Rounded.Campaign, null, tint = Theme.colors.ink, modifier = Modifier.size(18.dp))
                Text(item.title, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
            }
            else -> Unit
        }
        item.bodyHtml?.let { HtmlText(it) }
    }
}

@Composable
private fun Surface(content: @Composable () -> Unit) {
    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(Theme.radiusSurface)).background(Theme.colors.surface).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) { content() }
}

/** `CSC.T273 · 月 3–4限 · W5-101`, `CSC.T273 · Mon Periods 3–4 · W5-101` */
@Composable
private fun subtitle(course: JSONObject): String {
    val model = LocalModel.current
    val parts = listOfNotNull(course.text("code")).toMutableList()
    val meeting = course.optJSONArray("meetings")?.optJSONObject(0)
    val start = meeting?.text("startTime")
    val end = meeting?.text("endTime")
    if (meeting != null && start != null && end != null) {
        val first = model.periods.firstOrNull { it.startTime == start }?.periods?.firstOrNull()
        val last = model.periods.firstOrNull { it.endTime == end }?.periods?.lastOrNull()
        val periods = if (first != null && last != null) Format.periods(byteArrayOf(first, last)) else null
        parts.add("${Format.weekday(meeting.optJSONArray("days")?.optInt(0) ?: 0)} ${periods ?: "$start–$end"}")
        meeting.text("room")?.takeIf { it.isNotEmpty() }?.let { parts.add(it) }
    }
    return parts.joinToString(" · ")
}

/** `試験 · 11/27 5–6限 · W9-202`, one line per exam or quiz of this course. */
private fun examLine(exam: ExamView): String {
    val day = Format.civilDate(exam.date)
    var time = day?.let { "${it.monthValue}/${it.dayOfMonth}" } ?: exam.date
    val periods = Format.periods(exam.periods)
    val start = exam.startTime
    val end = exam.endTime
    if (periods != null) time += " $periods" else if (start != null && end != null) time += " $start–$end"
    return (listOf(if (exam.kind == "quiz") t("Quiz") else t("Exam"), time) + listOfNotNull(exam.room.ifEmpty { null })).joinToString(" · ")
}

@Composable
private fun Links(detail: CourseDetail?, title: String) {
    val navigator = LocalNavigator.current
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        detail?.lmsUrl?.let { url ->
            LinkChip("LMS", Icons.Rounded.Language) { navigator.openPage(WebRouter.PushedPage(SchoolSite.SCIENCE_LMS, url, title)) }
        }
        detail?.syllabusUrl?.let { url ->
            LinkChip(t("Syllabus"), Icons.Rounded.Description) { navigator.openPage(WebRouter.PushedPage(SchoolSite.SYLLABUS, url, title)) }
        }
    }
}

@Composable
private fun LinkChip(title: String, icon: ImageVector, onClick: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        modifier = Modifier
            .clip(CircleShape)
            .background(Theme.colors.accentSoft)
            .clickable(onClick = onClick)
            .padding(horizontal = 10.dp, vertical = 6.dp),
    ) {
        Icon(icon, null, tint = Theme.colors.accent, modifier = Modifier.size(14.dp))
        Text(title, style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.SemiBold, color = Theme.colors.accent)
    }
}
