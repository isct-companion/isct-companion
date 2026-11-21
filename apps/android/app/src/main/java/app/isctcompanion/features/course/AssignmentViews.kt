package app.isctcompanion.features.course

import app.isctcompanion.design.BackButton
import app.isctcompanion.design.Motion
import app.isctcompanion.design.LocalReducedMotion
import androidx.compose.animation.fadeOut
import androidx.compose.animation.fadeIn
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.expandVertically
import app.isctcompanion.features.handin.HandInCard

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animateFloatAsState
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
import androidx.compose.material.icons.automirrored.rounded.OpenInNew
import androidx.compose.material.icons.rounded.ExpandMore
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.AssignmentDetail
import app.isctcompanion.core.DeadlineState
import app.isctcompanion.design.AssignmentDeadlineTag
import java.time.Duration
import kotlinx.coroutines.delay
import app.isctcompanion.core.HandInAvailability
import app.isctcompanion.core.AssignmentState as CoreAssignmentState
import app.isctcompanion.core.LessonItemKind
import app.isctcompanion.core.SchoolSite
import app.isctcompanion.core.WebRouter
import app.isctcompanion.core.text
import app.isctcompanion.design.Card
import app.isctcompanion.design.Format
import app.isctcompanion.design.HtmlText
import app.isctcompanion.design.MaterialButton
import app.isctcompanion.design.ActionButton
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.Tag
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import java.time.Instant

/** An assignment pushed from 今日 or 課題: what the course page shows when it
 * opens one in place, as a page of its own. */
@Composable
fun AssignmentScreen(assignmentId: String) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    var assignment by remember { mutableStateOf<AssignmentDetail?>(null) }
    LaunchedEffect(assignmentId, model.contentRevision) {
        model.assignmentDetail(assignmentId)?.let { assignment = it }
    }
    val deadline = assignment?.deadline
    LaunchedEffect(assignmentId, deadline?.at) {
        if (deadline == null || deadline.state != DeadlineState.REMAINING) return@LaunchedEffect
        val due = Format.date(deadline.at) ?: return@LaunchedEffect
        do {
            delay(maxOf(1L, Duration.between(Instant.now(), due).toMillis()))
        } while (Instant.now().isBefore(due))
        model.assignmentDetail(assignmentId)?.let { assignment = it }
    }
    TabPage(
        assignment?.title.orEmpty(),
        navigationIcon = {
            BackButton { navigator.back() }
        },
    ) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = Theme.gutter).padding(bottom = Theme.space10),
            verticalArrangement = Arrangement.spacedBy(Theme.space7),
        ) {
            assignment?.let { detail ->
                Text(
                    model.records.courses[detail.courseId]?.text("title").orEmpty(),
                    style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold, color = Theme.colors.accent,
                    modifier = Modifier.clickable { navigator.openCourse(detail.courseId) },
                )
                AssignmentBody(detail)
                if (detail.handInAvailability == HandInAvailability.PREPARE_DRAFT) HandInCard(detail)
            }
        }
    }
}

/** One assignment inside a lesson: state first, the teacher's text on tap. */
@Composable
fun AssignmentCard(assignment: AssignmentDetail, expanded: Boolean, onExpand: (Boolean) -> Unit) {
    val reduced = LocalReducedMotion.current
    val turn by animateFloatAsState(if (expanded) 180f else 0f, Motion.base(), label = "chevron")
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Theme.radiusSurface))
            .background(Theme.colors.surface)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(Theme.space6),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(Theme.space5),
            modifier = Modifier.fillMaxWidth().clickable { onExpand(!expanded) },
        ) {
            Text(assignment.title, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink, modifier = Modifier.weight(1f))
            AssignmentState(assignment)
            Icon(Icons.Rounded.ExpandMore, null, tint = Theme.colors.muted, modifier = Modifier.size(18.dp).rotate(turn))
        }
        AnimatedVisibility(
            expanded,
            enter = if (reduced) fadeIn(Motion.fast()) else expandVertically(Motion.base()) + fadeIn(Motion.base()),
            exit = if (reduced) fadeOut(Motion.fast()) else shrinkVertically(Motion.base()) + fadeOut(Motion.base()),
        ) { AssignmentBody(assignment, showsState = false) }
    }
}

/** Deadline, the LMS text, its files and the way to the submission page. */
@Composable
fun AssignmentBody(assignment: AssignmentDetail, showsState: Boolean = true) {
    val navigator = LocalNavigator.current
    Column(verticalArrangement = Arrangement.spacedBy(Theme.space7)) {
        val due = assignment.dueAt?.let { t("course.dueDate", "date" to Format.due(it)) }
        if (showsState) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Theme.space4)) {
                AssignmentState(assignment)
                due?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted) }
            }
        } else {
            due?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted) }
        }
        if (assignment.bodyHtml.isNotEmpty()) {
            HtmlText(assignment.bodyHtml, assignment.courseId, originalHtml = assignment.originalBodyHtml)
        } else if (assignment.details.isNotEmpty()) {
            Text(assignment.details, color = Theme.colors.ink)
        }
        if (assignment.attachments.isNotEmpty()) {
            Card {
                assignment.attachments.forEachIndexed { index, file ->
                    if (index > 0) RowDivider()
                    MaterialButton(file.name, LessonItemKind.FILE, file.url, assignment.courseId)
                }
            }
        }
        assignment.sourceUrl?.takeIf { assignment.handInAvailability != HandInAvailability.UNAVAILABLE }?.let { url ->
            ActionButton(
                t(if (assignment.handInAvailability == HandInAvailability.OPEN_LMS) "common.openInLMS" else "common.submitInLMS"),
                { navigator.openPage(WebRouter.PushedPage(SchoolSite.SCIENCE_LMS, url, assignment.title)) },
                icon = Icons.AutoMirrored.Rounded.OpenInNew,
            )
        }
    }
}

/** 提出済 / the grade, or the countdown, in the deadline chip's colours. */
@Composable
private fun AssignmentState(assignment: AssignmentDetail) {
    val done = assignment.state == CoreAssignmentState.SUBMITTED || assignment.state == CoreAssignmentState.GRADED
    AssignmentDeadlineTag(assignment.deadline, done, assignment.grade)
}
