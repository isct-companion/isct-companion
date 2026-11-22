package app.isctcompanion.features.timetable

import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Error
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.Occurrence
import app.isctcompanion.core.OccurrenceKind
import app.isctcompanion.core.ScheduleView
import app.isctcompanion.core.SessionMark
import app.isctcompanion.core.TimetableAxis
import app.isctcompanion.design.Tag
import app.isctcompanion.design.TagVariant
import app.isctcompanion.design.Theme
import app.isctcompanion.design.courseColor
import app.isctcompanion.design.courseMix
import app.isctcompanion.design.courseZoomSource
import app.isctcompanion.design.t
import kotlinx.coroutines.launch

/** Gap between blocks that share a time side by side. */
internal val laneGap = 2.dp

@Composable
internal fun Block(
    block: Occurrence,
    week: ScheduleView?,
    axis: TimetableAxis,
    rowHeight: Dp,
    columnWidth: Dp,
    current: Boolean,
) {
    val navigator = LocalNavigator.current
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    var menu by remember { mutableStateOf(false) }
    // Core's grid: a block timed inside periods fills the periods it
    // touches, and overlapping blocks sit side by side.
    val grid = block.grid
    val top = rowHeight * axis.offset(grid?.startTime ?: block.startTime).toFloat()
    val bottom = rowHeight * axis.offset(grid?.endTime ?: block.endTime).toFloat()
    val lanes = maxOf(grid?.lanes?.toInt() ?: 1, 1)
    val lane = grid?.lane?.toInt() ?: 0
    val laneWidth = (columnWidth - laneGap * 2 - laneGap * (lanes - 1)) / lanes
    val planned = block.planned == true
    val cancelled = block.cancelled == true
    val exam = block.kind == OccurrenceKind.EXAM && !cancelled
    val quiz = exam && block.quiz == true
    val tint = courseColor(block.color)
    val surface = Theme.colors.surface
    val shape = RoundedCornerShape(Theme.radiusBlock)
    val id = block.courseId
    // Core's current mark, checked in the long-press menu; none when the
    // block takes no mark.
    val state = block.mark
    val ink = when {
        cancelled -> Theme.colors.muted
        else -> courseMix(tint, Theme.tintInk, Theme.colors.ink)
    }
    val fill = when {
        cancelled -> courseMix(Theme.colors.muted, 0.10f, surface)
        planned -> surface
        exam && !quiz -> courseMix(tint, Theme.tintExam, surface)
        else -> courseMix(tint, Theme.tintFill, surface)
    }
    Box(
        Modifier
            .offset(x = laneGap + (laneWidth + laneGap) * lane, y = top + 2.dp)
            .width(laneWidth),
    ) {
        Column(
            Modifier
                .fillMaxWidth()
                .height(maxOf(bottom - top - 4.dp, 24.dp))
                .courseZoomSource(block.key.takeIf { id != null })
                .clip(shape)
                .background(fill)
                .then(if (cancelled) Modifier.stripes(Theme.colors.line) else Modifier)
                .then(
                    // A ring means the class in progress, nothing else
                    // (slotledger `approach/timetable-exam-emphasis`).
                    when {
                        current && !cancelled -> Modifier.border(2.dp, tint, shape)
                        planned -> Modifier.border(1.5.dp, courseMix(tint, 0.45f, surface), shape)
                        else -> Modifier
                    },
                )
                .combinedClickable(
                    enabled = id != null || !planned,
                    onClick = { id?.let { navigator.openCourse(it, block.key) } },
                    onLongClick = if (state == null) null else ({
                        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                        menu = true
                    }),
                )
                .padding(Theme.space2),
        ) {
            val style = MaterialTheme.typography.labelSmall
            // An exam or quiz names itself first, so even a short block says what it is.
            if (exam) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(3.dp)) {
                    ExamTag(quiz, tint)
                    if (block.sourceChanged == true) {
                        Icon(Icons.Rounded.Error, t("common.timetableChanged"), tint = Theme.colors.warn, modifier = Modifier.size(12.dp))
                    }
                }
            }
            Text(
                block.title, style = style, fontWeight = FontWeight.SemiBold, color = ink,
                textDecoration = if (cancelled) TextDecoration.LineThrough else null,
                maxLines = if (lanes > 1) Int.MAX_VALUE else 4, overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f, fill = false),
            )
            // Side by side there is room for the title only.
            if (lanes == 1) {
                Spacer(Modifier.weight(1f))
                // 計画, 休講, the times of a block drawn over whole periods,
                // or the first room; every room stays on the course page.
                val moved = grid != null && (grid.startTime != block.startTime || grid.endTime != block.endTime)
                val line = when {
                    planned -> t("timetable.planned")
                    cancelled -> t("common.noClass")
                    moved -> "${block.startTime.removePrefix("0")}–${block.endTime.removePrefix("0")}"
                    else -> firstRoom(block.room)
                }
                Text(
                    line, style = style, fontWeight = FontWeight.SemiBold, color = ink,
                    maxLines = 1, overflow = TextOverflow.Ellipsis,
                )
            }
        }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            Text(
                t("timetable.thisLesson"), style = MaterialTheme.typography.labelMedium, color = Theme.colors.muted,
                modifier = Modifier.padding(horizontal = Theme.space6, vertical = Theme.space3),
            )
            HorizontalDivider()
            // Choosing the checked mark again sends `class`; Core decides
            // whether that clears the mark.
            for ((mark, value, label) in listOf(
                Triple(SessionMark.EXAM, "exam", t("common.markAsExam")),
                Triple(SessionMark.QUIZ, "quiz", t("common.markAsQuiz")),
                Triple(SessionMark.CANCELLED, "cancelled", t("common.markAsCancelled")),
            )) {
                DropdownMenuItem(
                    text = { Text(label) },
                    trailingIcon = { if (mark == state) Icon(Icons.Rounded.Check, null, tint = Theme.colors.accent) },
                    onClick = {
                        menu = false
                        val committed = model.markSession(block.key, if (mark == state) "class" else value)
                        scope.launch {
                            committed.join()
                            haptics.performHapticFeedback(HapticFeedbackType.Confirm)
                        }
                    },
                )
            }
        }
    }
}

/** 試験 above an exam block's title, solid in the course colour; a quiz's
 * 小テスト is outlined. */
@Composable
private fun ExamTag(quiz: Boolean, tint: Color) {
    if (quiz) {
        Tag(t("common.quiz"), color = tint, variant = TagVariant.Outline, ink = courseMix(tint, Theme.tintInk, Theme.colors.ink))
    } else {
        Tag(t("common.exam"), color = tint, variant = TagVariant.Solid)
    }
}

/** The first of several rooms joined by commas; every room stays on the course page. */
internal fun firstRoom(room: String): String =
    room.split(',', '、', '，').firstOrNull()?.trim() ?: room

/** Diagonal stripes over a class that does not take place. */
private fun Modifier.stripes(color: Color): Modifier = drawBehind {
    val step = 8.dp.toPx()
    var x = -size.height
    while (x < size.width) {
        drawLine(color, Offset(x, size.height), Offset(x + size.height, 0f), strokeWidth = 3.dp.toPx())
        x += step
    }
}
