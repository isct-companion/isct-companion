package app.isctcompanion.features.timetable

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.DatedAgenda
import app.isctcompanion.core.Occurrence
import app.isctcompanion.core.OccurrenceKind
import app.isctcompanion.core.ScheduleView
import app.isctcompanion.core.TeachingPeriod
import app.isctcompanion.core.TodayPhase
import app.isctcompanion.design.Format
import app.isctcompanion.design.PageSubtitle
import app.isctcompanion.design.TabPage
import androidx.compose.material3.IconButton
import app.isctcompanion.design.Theme
import app.isctcompanion.design.courseColor
import app.isctcompanion.design.tabular
import java.time.DayOfWeek
import java.time.LocalDate
import kotlin.math.abs
import kotlin.math.roundToInt

private val rowHeight = 64.dp
private const val PERIOD_COUNT = 10

/** 時間割: the week grid, read-only. Blocks come from Core's `ScheduleView`
 * for Monday–Friday; adding and planning courses stay on desktop. */
@Composable
fun TimetableScreen() {
    val model = LocalModel.current
    var monday by rememberSaveable { mutableStateOf(monday(Format.today())) }
    var week by androidx.compose.runtime.remember { mutableStateOf<ScheduleView?>(null) }
    var drag by androidx.compose.runtime.remember { mutableFloatStateOf(0f) }
    val offset by animateFloatAsState(drag, label = "week drag")
    LaunchedEffect(monday, model.contentRevision) { week = model.week(monday) }
    val swipe = with(LocalDensity.current) { 60.dp.toPx() }

    TabPage(
        "時間割",
        actions = {
            IconButton(onClick = { monday = monday.minusWeeks(1) }) {
                Icon(Icons.AutoMirrored.Outlined.KeyboardArrowLeft, "前の週")
            }
            IconButton(onClick = { monday = monday.plusWeeks(1) }) {
                Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, "次の週")
            }
        },
    ) {
    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 12.dp)
            .padding(bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        PageSubtitle(subtitle(monday, model.today?.term, model.today?.teachingWeek))
        Grid(
            monday, week, model.periods,
            Modifier
                .offset { IntOffset(offset.roundToInt(), 0) }
                .pointerInput(Unit) {
                    var total = 0f
                    detectHorizontalDragGestures(
                        onDragStart = { total = 0f },
                        onDragEnd = {
                            if (abs(total) > swipe) monday = if (total < 0) monday.plusWeeks(1) else monday.minusWeeks(1)
                            total = 0f
                            drag = 0f
                        },
                        onDragCancel = {
                            total = 0f
                            drag = 0f
                        },
                    ) { change, amount ->
                        change.consume()
                        total += amount
                        drag = total / 3
                    }
                },
        )
    }
    }
}

private fun subtitle(monday: LocalDate, term: String?, teachingWeek: UInt?): String {
    val friday = monday.plusDays(4)
    val range = "${monday.monthValue}/${monday.dayOfMonth} – ${friday.monthValue}/${friday.dayOfMonth}"
    term ?: return range
    if (monday == monday(Format.today()) && teachingWeek != null) {
        return "${Format.quarter(term)} 第${teachingWeek}週 · $range"
    }
    return "${Format.quarter(term)} · $range"
}

/** Saturday and Sunday look ahead to the coming week. */
fun monday(date: LocalDate): LocalDate = when (date.dayOfWeek) {
    DayOfWeek.SATURDAY -> date.plusDays(2)
    DayOfWeek.SUNDAY -> date.plusDays(1)
    else -> date.minusDays((date.dayOfWeek.value - 1).toLong())
}

@Composable
private fun Grid(monday: LocalDate, week: ScheduleView?, periods: List<TeachingPeriod>, modifier: Modifier) {
    val model = LocalModel.current
    val days = (0L until 5L).map { monday.plusDays(it) }
    val height = rowHeight * PERIOD_COUNT
    val today = Format.today()
    val current = model.today?.takeIf { it.phase == TodayPhase.IN_CLASS }?.hero?.occurrence?.key
    Column(modifier, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            Spacer(Modifier.width(30.dp))
            for (day in days) DayHeader(day, day == today, Modifier.weight(1f))
        }
        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            PeriodColumn(periods, Modifier.width(30.dp).height(height))
            for (day in days) {
                val agenda = week?.agendas?.firstOrNull { it.date == Format.civil(day) }
                DayColumn(agenda, day == today, week, periods, current, Modifier.weight(1f).height(height))
            }
        }
    }
}

@Composable
private fun DayHeader(day: LocalDate, today: Boolean, modifier: Modifier) {
    Column(modifier, horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Text(Format.weekday(day), style = MaterialTheme.typography.labelMedium, color = Theme.colors.muted)
        Box(
            Modifier.size(30.dp).clip(CircleShape).background(if (today) Theme.colors.accent else Color.Transparent),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                "${day.dayOfMonth}", style = MaterialTheme.typography.titleMedium.tabular(), fontWeight = FontWeight.SemiBold,
                color = if (today) Color.White else Theme.colors.ink,
            )
        }
    }
}

@Composable
private fun PeriodColumn(periods: List<TeachingPeriod>, modifier: Modifier) {
    Column(modifier) {
        for (period in 1..PERIOD_COUNT) {
            Column(Modifier.height(rowHeight).fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
                Text("$period", style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
                startTime(periods, period)?.let {
                    Text(it, fontSize = 9.sp, style = MaterialTheme.typography.labelSmall.tabular(), color = Theme.colors.muted)
                }
            }
        }
    }
}

@Composable
private fun DayColumn(
    agenda: DatedAgenda?,
    today: Boolean,
    week: ScheduleView?,
    periods: List<TeachingPeriod>,
    current: String?,
    modifier: Modifier,
) {
    val shape = RoundedCornerShape(12.dp)
    val holiday = agenda?.noClasses == true
    val blocks = agenda?.occurrences.orEmpty().filter { it.kind == OccurrenceKind.COURSE || it.kind == OccurrenceKind.EXAM }
    Box(
        modifier
            .clip(shape)
            .background(Theme.colors.surface)
            .then(if (today) Modifier.border(1.5.dp, Theme.colors.accent.copy(alpha = 0.5f), shape) else Modifier),
    ) {
        PeriodLines(periods, Theme.colors.line)
        if (holiday) Hatch(Theme.colors.line)
        for (block in blocks) Block(block, holiday, week, periods, block.key == current)
    }
}

@Composable
private fun Block(block: Occurrence, holiday: Boolean, week: ScheduleView?, periods: List<TeachingPeriod>, current: Boolean) {
    val navigator = LocalNavigator.current
    val top = rowHeight * slot(periods, block.startTime)
    val bottom = rowHeight * slot(periods, block.endTime)
    val planned = block.courseId?.let { week?.enrolledCourseIds?.contains(it) == false } ?: false
    val faded = holiday || block.cancelled == true
    val tint = courseColor(block.color)
    val shape = RoundedCornerShape(8.dp)
    val id = block.courseId
    Column(
        Modifier
            .padding(horizontal = 2.dp)
            .offset(y = top + 2.dp)
            .fillMaxWidth()
            .height(maxOf(bottom - top - 4.dp, 24.dp))
            .clip(shape)
            .background(if (planned) Theme.colors.surface else tint.copy(alpha = if (faded) 0.08f else Theme.tintFill))
            .then(
                when {
                    current -> Modifier.border(2.dp, tint, shape)
                    planned -> Modifier.border(1.dp, Theme.colors.line, shape)
                    else -> Modifier
                },
            )
            .then(if (id != null) Modifier.clickable { navigator.openCourse(id) } else Modifier)
            .padding(5.dp),
    ) {
        val ink = if (faded) Theme.colors.muted else Theme.colors.ink
        Text(
            block.title, style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.SemiBold, color = ink,
            maxLines = 3, overflow = TextOverflow.Ellipsis,
        )
        Spacer(Modifier.weight(1f))
        Text(
            if (planned) "計画" else block.room, style = MaterialTheme.typography.labelSmall, fontWeight = FontWeight.SemiBold,
            color = ink, maxLines = 1, overflow = TextOverflow.Ellipsis,
        )
    }
}

/** One rule after each teaching slot (1–2, 3–4, …), never inside a slot, so
 * a two-period class is not cut in half. */
@Composable
private fun PeriodLines(periods: List<TeachingPeriod>, color: Color) {
    val ends = periods.mapNotNull { it.periods.lastOrNull()?.toInt() }.filter { it in 1 until PERIOD_COUNT }
    Canvas(Modifier.fillMaxSize()) {
        for (end in ends) {
            val y = rowHeight.toPx() * end
            drawLine(color, Offset(0f, y), Offset(size.width, y), strokeWidth = 1.dp.toPx())
        }
    }
}

/** Diagonal hatch for a day without classes. */
@Composable
private fun Hatch(color: Color) {
    Canvas(Modifier.fillMaxSize()) {
        var x = -size.height
        while (x < size.width) {
            drawLine(color, Offset(x, size.height), Offset(x + size.height, 0f), strokeWidth = 4.dp.toPx())
            x += 10.dp.toPx()
        }
    }
}

// Period geometry

/** Fractional row (0-based) of a clock time, interpolated over the
 * teaching-period table so blocks line up with their periods. */
private fun slot(periods: List<TeachingPeriod>, time: String): Float {
    val minutes = minutes(time)
    var row = 0f
    for (period in periods) {
        val start = minutes(period.startTime)
        val end = minutes(period.endTime)
        val first = (period.periods.firstOrNull()?.toInt() ?: 1) - 1f
        val span = ((period.periods.lastOrNull()?.toInt() ?: 1) - (period.periods.firstOrNull()?.toInt() ?: 1) + 1).toFloat()
        if (minutes < start) return row
        if (minutes <= end) return first + span * (minutes - start) / maxOf(end - start, 1)
        row = first + span
    }
    return minOf(row, PERIOD_COUNT.toFloat())
}

private fun startTime(periods: List<TeachingPeriod>, period: Int): String? =
    periods.firstOrNull { it.periods.firstOrNull()?.toInt() == period }?.startTime?.removePrefix("0")

private fun minutes(time: String): Int {
    val parts = time.split(":").mapNotNull { it.toIntOrNull() }
    return if (parts.size == 2) parts[0] * 60 + parts[1] else 0
}
