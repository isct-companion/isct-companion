package app.isctcompanion.features.timetable

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowRight
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.core.TimetableWeek
import app.isctcompanion.core.TimetableYear
import app.isctcompanion.design.Format
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import app.isctcompanion.design.tabular
import java.time.LocalDate
import kotlinx.coroutines.launch

/** A week's quarter label from Core's week list; nothing while the academic
 * calendar is unknown: a week is only 休み when Core knows the quarters. */
fun weekLabel(week: TimetableWeek, year: TimetableYear): String? {
    if (year.quarters.isEmpty()) return null
    val term = week.term ?: return t("timetable.breakBetweenQuarters")
    val name = listOfNotNull(Format.quarter(term), week.week?.let { t("timetable.weekNumber", "week" to it) }).joinToString(" ")
    return if (week.exams) "$name · ${t("common.exams")}" else name
}

/** The timetable's time picker: quarter chips that jump to a quarter's first
 * teaching week, and the academic year's weeks, each labelled with its
 * quarter. Core decides every label (`TimetableYear`); this only lays them out. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WeekPicker(year: TimetableYear?, monday: LocalDate, dismiss: () -> Unit, pick: (LocalDate) -> Unit) {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    var stepped by remember { mutableStateOf<TimetableYear?>(null) }
    val shown = stepped ?: year
    val selected = Format.civil(monday)
    fun step(by: Int) = scope.launch {
        val current = shown?.year ?: return@launch
        model.timetableYear(current + by)?.let { stepped = it }
    }
    ModalBottomSheet(onDismissRequest = dismiss, containerColor = Theme.colors.bg) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth().padding(horizontal = Theme.space4)) {
            IconButton(onClick = { step(-1) }) { Icon(Icons.AutoMirrored.Rounded.KeyboardArrowLeft, t("timetable.previousYear")) }
            IconButton(onClick = { step(1) }) { Icon(Icons.AutoMirrored.Rounded.KeyboardArrowRight, t("timetable.nextYear")) }
            Text(
                shown?.let { t("timetable.academicYear", "year" to it.year) } ?: t("timetable.chooseAWeek"),
                style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink,
                modifier = Modifier.weight(1f),
            )
            TextButton(onClick = { pick(thisWeek()) }) { Text(t("timetable.backToToday")) }
        }
        val quarterOfShown = shown?.weeks?.firstOrNull { it.monday == selected }?.term
        LazyRow(contentPadding = PaddingValues(horizontal = Theme.space8), horizontalArrangement = Arrangement.spacedBy(Theme.space4)) {
            items(shown?.quarters.orEmpty(), key = { it.term }) { quarter ->
                FilterChip(
                    selected = quarter.term == quarterOfShown,
                    onClick = { Format.civilDate(quarter.firstWeek)?.let(pick) },
                    label = { Text(Format.quarter(quarter.term)) },
                )
            }
        }
        val weeks = shown?.weeks.orEmpty()
        val list = rememberLazyListState()
        // Opens on the shown week.
        LaunchedEffect(shown) {
            val index = rows(weeks).indexOfFirst { (it as? PickerRow.Week)?.week?.monday == selected }
            if (index > 0) list.scrollToItem(maxOf(index - 3, 0))
        }
        LazyColumn(state = list, contentPadding = PaddingValues(bottom = Theme.space10)) {
            items(rows(weeks)) { row ->
                when (row) {
                    is PickerRow.Month -> Text(
                        row.title, style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.SemiBold, color = Theme.colors.muted,
                        modifier = Modifier.padding(start = Theme.space9, top = Theme.space8, bottom = Theme.space2),
                    )
                    is PickerRow.Week -> shown?.let { WeekRow(row.week, it, row.week.monday == selected) { Format.civilDate(row.week.monday)?.let(pick) } }
                }
            }
        }
    }
}

@Composable
private fun WeekRow(week: TimetableWeek, year: TimetableYear, selected: Boolean, onClick: () -> Unit) {
    val tone = when {
        week.exams -> Theme.colors.warn
        week.term == null -> Theme.colors.muted
        else -> Theme.colors.accent
    }
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Theme.space6),
        modifier = Modifier
            .fillMaxWidth()
            .background(if (week.term == null) Theme.colors.surface2 else Theme.colors.surface)
            .clickable(onClick = onClick)
            .padding(horizontal = Theme.space9, vertical = Theme.space5),
    ) {
        Text(
            weekLabel(week, year).orEmpty(), style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.SemiBold,
            color = tone, modifier = Modifier.width(110.dp),
        )
        Text(
            Format.civilDate(week.monday)?.let(::weekDateRange) ?: week.monday,
            style = MaterialTheme.typography.bodyLarge.tabular(),
            color = if (selected) Theme.colors.accentInk else Theme.colors.ink,
            modifier = Modifier
                .clip(CircleShape)
                .background(if (selected) Theme.colors.accent else Color.Transparent)
                .padding(horizontal = Theme.space5, vertical = Theme.space2),
        )
    }
}

private sealed interface PickerRow {
    data class Month(val title: String) : PickerRow
    data class Week(val week: TimetableWeek) : PickerRow
}

/** Weeks grouped under the month their Monday falls in. */
private fun rows(weeks: List<TimetableWeek>): List<PickerRow> {
    val rows = mutableListOf<PickerRow>()
    var month: String? = null
    for (week in weeks) {
        val title = Format.civilDate(week.monday)?.let { Format.written(it, "yMMMM") } ?: week.monday
        if (title != month) {
            rows.add(PickerRow.Month(title))
            month = title
        }
        rows.add(PickerRow.Week(week))
    }
    return rows
}
