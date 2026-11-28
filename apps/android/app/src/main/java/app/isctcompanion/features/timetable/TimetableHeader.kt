package app.isctcompanion.features.timetable

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CalendarMonth
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.sp
import app.isctcompanion.core.TimetableWeek
import app.isctcompanion.core.TimetableYear
import app.isctcompanion.design.ActionButton
import app.isctcompanion.design.ButtonLevel
import app.isctcompanion.design.CountingText
import app.isctcompanion.design.Format
import app.isctcompanion.design.Localizer
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import app.isctcompanion.design.tabular
import java.time.LocalDate

/** The week chip, the quarter export and 今日 above the week's date range,
 * which rolls on one line across the full width. Weeks change by swiping
 * the grid; the chip and the range open the week picker. */
@Composable
internal fun TimetableHeader(
    monday: LocalDate,
    year: TimetableYear?,
    term: String?,
    openPicker: () -> Unit,
    toToday: () -> Unit,
) {
    val atThisWeek = monday == thisWeek()
    // The chip's labels across the shown year, widest by ActionButton's own label style.
    val labelStyle = LocalTextStyle.current.copy(fontWeight = FontWeight.SemiBold)
    val measurer = rememberTextMeasurer()
    val widest = remember(year, Localizer.language, labelStyle, measurer) {
        (year?.weeks.orEmpty().map { weekChip(year, it) } + weekChip(year, null))
            .distinct()
            .maxBy { measurer.measure(it, labelStyle).size.width }
    }
    Column(
        Modifier.fillMaxWidth().padding(top = Theme.space6, start = Theme.space2, end = Theme.space2),
        verticalArrangement = Arrangement.spacedBy(Theme.space3),
    ) {
        Controls(
            widest = {
                Row(Modifier.clearAndSetSemantics {}, horizontalArrangement = Arrangement.spacedBy(Theme.space4)) {
                    WeekChip(widest) {}
                    Actions(term, atThisWeek = false) {}
                }
            },
            chip = { WeekChip(weekChip(year, year?.weeks?.firstOrNull { it.monday == Format.civil(monday) }), openPicker) },
            actions = { Actions(term, atThisWeek, toToday) },
        )
        BoxWithConstraints(Modifier.fillMaxWidth()) {
            val typography = MaterialTheme.typography
            val style = typography.headlineLarge.tabular().copy(
                fontWeight = FontWeight.Bold,
                lineHeight = TextUnit.Unspecified,
            )
            val range = weekDateRange(monday)
            CountingText(
                range,
                Modifier
                    .clickable(role = Role.Button, onClickLabel = t("timetable.chooseAWeek"), onClick = openPicker)
                    // Read as one range, not figure by rolling figure.
                    .clearAndSetSemantics { contentDescription = range },
                style = style,
                color = Theme.colors.ink,
                fontSize = rollingSize(style, constraints.maxWidth, typography.bodySmall.fontSize),
                countsDown = false,
            )
        }
    }
}

/** The chip and the actions share a row while the shown year's widest chip
 * fits beside them; otherwise the actions sit under the chip. [widest] is
 * measured and never placed, so the choice follows the window and the font
 * size, not the week on screen: a longer exam-week label does not reshape
 * the header, or the grid's rows, mid-swipe. */
@Composable
private fun Controls(widest: @Composable () -> Unit, chip: @Composable () -> Unit, actions: @Composable () -> Unit) {
    val gap = Theme.space4
    val rowGap = Theme.space2
    Layout(contents = listOf(widest, chip, actions)) { (reference, label, tools), constraints ->
        val loose = constraints.copy(minWidth = 0, minHeight = 0)
        val width = constraints.maxWidth
        val oneRow = reference.single().maxIntrinsicWidth(Constraints.Infinity) <= width
        val trailing = tools.single().measure(loose)
        val leading = label.single().measure(
            if (oneRow) loose.copy(maxWidth = (width - gap.roundToPx() - trailing.width).coerceAtLeast(0)) else loose,
        )
        if (oneRow) {
            val height = maxOf(leading.height, trailing.height)
            layout(width, height) {
                leading.place(0, (height - leading.height) / 2)
                trailing.place(width - trailing.width, (height - trailing.height) / 2)
            }
        } else {
            val below = leading.height + rowGap.roundToPx()
            layout(width, below + trailing.height) {
                leading.place(0, 0)
                trailing.place(0, below)
            }
        }
    }
}

/** The way into the week picker, a secondary button. */
@Composable
private fun WeekChip(label: String, openPicker: () -> Unit) {
    ActionButton(
        label, openPicker,
        Modifier.semantics { contentDescription = t("timetable.chooseAWeek") },
        level = ButtonLevel.Secondary,
        icon = Icons.Rounded.CalendarMonth,
    )
}

@Composable
private fun Actions(term: String?, atThisWeek: Boolean, toToday: () -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        ExportButton(term)
        // 今日 rests while this week is shown, which doubles as "you are here".
        ActionButton(
            t("common.today"), toToday,
            Modifier.semantics { contentDescription = t("timetable.backToThisWeek") },
            level = ButtonLevel.Secondary,
            enabled = !atThisWeek,
        )
    }
}

/** The largest size, from [style]'s down to [floor], at which the range fits
 * [width] at its widest moment: each rolling figure keeps its own slot and,
 * mid-roll, takes the wider of its two figures, so the longest form of the
 * range is sized with its widest figure in every slot. Every week shares
 * that size, so neither a month change nor a roll cuts the end off. */
@Composable
private fun rollingSize(style: TextStyle, width: Int, floor: TextUnit): TextUnit {
    val measurer = rememberTextMeasurer()
    return remember(style, width, floor, measurer) {
        val longest = weekDateRange(LocalDate.of(2026, 12, 14))
        val figures = longest.toSet().map(Char::toString)
        generateSequence(style.fontSize.value) { it - 1f }
            .takeWhile { it >= floor.value }
            .map { it.sp }
            .firstOrNull { size ->
                figures.maxOf { measurer.measure(it, style.copy(fontSize = size)).size.width } * longest.length <= width
            }
            ?: floor
    }
}

/** "2026年度 · 3Q 第 2 週", from Core's week list; between quarters, 休み. */
private fun weekChip(year: TimetableYear?, week: TimetableWeek?): String {
    year ?: return t("common.timetable")
    return listOfNotNull(t("timetable.academicYear", "year" to year.year), week?.let { weekLabel(it, year) }).joinToString(" · ")
}

internal fun weekDateRange(monday: LocalDate): String {
    val friday = monday.plusDays(4)
    return "${monday.monthValue}/${monday.dayOfMonth} – ${friday.monthValue}/${friday.dayOfMonth}"
}
