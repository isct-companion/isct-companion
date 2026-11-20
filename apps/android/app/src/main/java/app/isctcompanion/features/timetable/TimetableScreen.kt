package app.isctcompanion.features.timetable

import app.isctcompanion.shared.rememberNow
import app.isctcompanion.design.Skeleton
import app.isctcompanion.design.Loading
import app.isctcompanion.design.Motion
import app.isctcompanion.design.LocalReducedMotion
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
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
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.TextAutoSize
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CalendarMonth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.DatedAgenda
import app.isctcompanion.core.Occurrence
import app.isctcompanion.core.OccurrenceKind
import app.isctcompanion.core.ScheduleView
import app.isctcompanion.core.SyncView
import app.isctcompanion.core.TimetableAxis
import app.isctcompanion.core.TimetableYear
import app.isctcompanion.core.TodayPhase
import app.isctcompanion.design.ActionButton
import app.isctcompanion.design.ButtonLevel
import app.isctcompanion.design.Format
import app.isctcompanion.design.GuideTip
import app.isctcompanion.design.Theme
import app.isctcompanion.design.CountingText
import app.isctcompanion.design.courseMix
import app.isctcompanion.design.t
import app.isctcompanion.design.tabular
import java.time.DayOfWeek
import java.time.LocalDate
import kotlin.math.abs
import kotlin.math.floor
import kotlin.math.roundToInt
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch

private val minimumRow = 30.dp
/** Space between two weeks while they pass: past the page's side margin, so
 * a neighbour never peeks in at rest. */
private val weekGap = 24.dp
/** Weekday letters and date circles above the columns, plus their gap. */
private val dayHeaderHeight = 54.dp
private val periodWidth = 26.dp

/** 時間割: a dated week. Blocks come from Core's `ScheduleView` for
 * Monday–Friday, whatever quarter the week is in; the week chip opens a picker
 * that names each week's quarter (`TimetableYear`). A long press marks one
 * session as an exam, a quiz or 休講 (choosing the checked mark again makes
 * it a plain class); the header exports the quarter as an .ics. Adding and
 * planning courses stay on desktop. */
@Composable
fun TimetableScreen() {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    var monday by rememberSaveable { mutableStateOf(monday(Format.today())) }
    val week = model.timetableWeeks[monday]
    val weekRevision = model.timetableWeeks.revision
    var axis by remember { mutableStateOf(model.lastTimetableAxis) }
    var year by remember { mutableStateOf<TimetableYear?>(null) }
    var picker by remember { mutableStateOf(false) }
    // The grid's horizontal travel while weeks change, and its fade when the
    // system asks for less motion (slotledger `approach/motion-week-swipe`).
    val shift = remember { Animatable(0f) }
    val fade = remember { Animatable(1f) }
    val reduced = LocalReducedMotion.current
    val scope = rememberCoroutineScope()
    LaunchedEffect(Unit) { model.open(SyncView.Timetable) }
    LaunchedEffect(navigator.timetableDate) {
        val date = navigator.timetableDate ?: return@LaunchedEffect
        monday = monday(LocalDate.parse(date))
        navigator.timetableDate = null
    }
    val requestedMonday = monday
    LaunchedEffect(requestedMonday, weekRevision, model.today?.term) {
        if (monday != requestedMonday || model.timetableWeeks.revision != weekRevision) return@LaunchedEffect
        val loaded = model.week(requestedMonday) ?: return@LaunchedEffect
        val shape = model.timetableAxis(loaded)
        currentCoroutineContext().ensureActive()
        if (monday != requestedMonday || model.timetableWeeks.revision != weekRevision) return@LaunchedEffect
        if (shape != null) {
            axis = shape
            model.lastTimetableAxis = shape
        }
        val number = model.academicYear(requestedMonday) ?: return@LaunchedEffect
        if (year?.year != number || year?.quarters?.isEmpty() == true) {
            val calendar = model.timetableYear(number)
            currentCoroutineContext().ensureActive()
            if (monday == requestedMonday && model.timetableWeeks.revision == weekRevision && calendar != null) {
                year = calendar
            }
        }
    }
    val swipe = with(LocalDensity.current) { 60.dp.toPx() }
    var width by remember { mutableFloatStateOf(1f) }
    // The week and its blocks change together. Set apart (the blocks a frame
    // later, from the effect above), the grid slid in empty and its cards
    // popped in where they stand.
    fun show(to: LocalDate) {
        monday = to
    }
    val gap = with(LocalDensity.current) { weekGap.toPx() }
    // The weeks lie side by side and follow the finger; a swipe glides the
    // strip on to the neighbouring week, which then becomes the middle one
    // without anything moving. A jump further (the picker, 今日) fades.
    fun go(to: LocalDate, slow: Boolean) = scope.launch {
        if (to == monday) {
            shift.animateTo(0f, Motion.slowSpring())
            return@launch
        }
        val adjacent = to == monday.plusWeeks(1) || to == monday.minusWeeks(1)
        if (reduced || !adjacent) {
            shift.animateTo(0f, Motion.fast())
            fade.animateTo(0f, Motion.fast())
            show(to)
            fade.animateTo(1f, Motion.fast())
            return@launch
        }
        val direction = if (to.isAfter(monday)) -1f else 1f
        val spec = if (slow) Motion.slow<Float>() else Motion.base<Float>()
        shift.animateTo(direction * (width + gap), spec)
        show(to)
        shift.snapTo(0f)
    }
    // The shown week's quarter, or this quarter between quarters.
    val term = year?.weeks?.firstOrNull { it.monday == Format.civil(monday) }?.term ?: week?.term ?: model.today?.term

    Column(
        Modifier
            .fillMaxSize()
            // Passing weeks reach the page's edge but never draw over the
            // rail beside it on a tablet.
            .clipToBounds()
            .background(Theme.colors.bg)
            .windowInsetsPadding(WindowInsets.statusBars)
            .padding(horizontal = Theme.space6)
            .padding(bottom = Theme.space4),
        verticalArrangement = Arrangement.spacedBy(Theme.space6),
    ) {
        Header(monday, chip(year, monday), term, openPicker = { picker = true }) { go(thisWeek(), slow = false) }
        GuideTip("timetable-marks", "guide.swipeToChangeWeeks", "timetable.longPressAClassTo")
        // Period rows share the height the screen leaves, so the week fits
        // without scrolling; past the floor (a tablet on its side, split
        // screen) the grid scrolls instead of cutting off the last periods.
        // The swipe sits on the frame around the weeks, so the blocks inside
        // still take taps and long presses.
        BoxWithConstraints(
            Modifier.weight(1f).fillMaxWidth()
                .pointerInput(Unit) {
                        val tracker = VelocityTracker()
                        var total = 0f
                        detectHorizontalDragGestures(
                            onDragStart = {
                                total = 0f
                                tracker.resetTracking()
                            },
                            onDragEnd = {
                                // Past 60 dp, or a fling, turns the week.
                                val fling = tracker.calculateVelocity().x
                                val turn = abs(total) > swipe || abs(fling) > 1_000f
                                val forward = if (abs(total) > swipe) total < 0 else fling < 0
                                go(if (!turn) monday else if (forward) monday.plusWeeks(1) else monday.minusWeeks(1), slow = true)
                                total = 0f
                            },
                            onDragCancel = {
                                total = 0f
                                go(monday, slow = true)
                            },
                        ) { change, amount ->
                            change.consume()
                            tracker.addPosition(change.uptimeMillis, change.position)
                            total += amount
                            // The grid follows the finger; with less motion it stays put.
                            if (!reduced) scope.launch { shift.snapTo(total) }
                        }
                    },
        ) {
            val length = axis?.length ?: 12.5
            val rowHeight = maxOf(minimumRow, floor(((maxHeight - dayHeaderHeight).value / length)).dp)
            val columns = constraints.maxWidth.toFloat()
            SideEffect { width = columns }
            // The previous, shown and next weeks, one page apart. The
            // neighbours draw from the weeks already read around this one (a
            // skeleton otherwise), so a swipe shows the next week coming in.
            for (slot in -1..1) {
                val start = monday.plusWeeks(slot.toLong())
                key(slot) {
                    Grid(
                        start, model.timetableWeeks[start], axis, rowHeight,
                        Modifier
                            .offset { IntOffset((slot * (width + gap) + shift.value).roundToInt(), 0) }
                            .graphicsLayer { alpha = fade.value }
                            .verticalScroll(rememberScrollState()),
                    )
                }
            }
        }
    }
    if (picker) {
        WeekPicker(year, monday, dismiss = { picker = false }) { picked ->
            picker = false
            go(picked, slow = false)
        }
    }
}

private val Occurrence.isBlock: Boolean
    get() = kind == OccurrenceKind.COURSE || kind == OccurrenceKind.EXAM

/** The week chip on its own row, free to take the full width; below it the
 * date range, the quarter export and 今日. Weeks change by swiping the grid. */
@Composable
private fun Header(monday: LocalDate, chip: String, term: String?, openPicker: () -> Unit, toToday: () -> Unit) {
    Column(Modifier.padding(top = Theme.space6, start = Theme.space2, end = Theme.space2), verticalArrangement = Arrangement.spacedBy(Theme.space3)) {
        // The way into the week picker, a secondary button.
        ActionButton(
            chip, openPicker,
            Modifier.semantics { contentDescription = t("timetable.chooseAWeek") },
            level = ButtonLevel.Secondary, icon = Icons.Rounded.CalendarMonth,
        )
        Row(verticalAlignment = Alignment.CenterVertically) {
            val friday = monday.plusDays(4)
            Box(Modifier.weight(1f)) {
                CountingText(
                    "${monday.monthValue}/${monday.dayOfMonth} – ${friday.monthValue}/${friday.dayOfMonth}",
                    Modifier.clickable(onClick = openPicker),
                    style = MaterialTheme.typography.headlineLarge.tabular(), fontWeight = FontWeight.Bold,
                    color = Theme.colors.ink, countsDown = false,
                )
            }
            term?.let { ExportButton(it) }
            // 今日 rests while this week is shown, which doubles as "you are here".
            val atThisWeek = monday == thisWeek()
            ActionButton(
                t("common.today"), toToday,
                Modifier.semantics { contentDescription = t("timetable.backToThisWeek") },
                level = ButtonLevel.Secondary, enabled = !atThisWeek,
            )
        }
    }
}

/** The shown quarter as an .ics, written by Core as the desktop's ICSを書き出す,
 * handed to the share sheet (Calendar, Drive, Mail). */

/** "2026年度 · 3Q 第 2 週", from Core's week list; between quarters, 休み. */
private fun chip(year: TimetableYear?, monday: LocalDate): String {
    year ?: return t("common.timetable")
    val label = year.weeks.firstOrNull { it.monday == Format.civil(monday) }?.let { weekLabel(it, year) }
    return listOfNotNull(t("timetable.academicYear", "year" to year.year), label).joinToString(" · ")
}

internal fun thisWeek() = monday(Format.today())

/** Saturday and Sunday look ahead to the coming week. */
fun monday(date: LocalDate): LocalDate = when (date.dayOfWeek) {
    DayOfWeek.SATURDAY -> date.plusDays(2)
    DayOfWeek.SUNDAY -> date.plusDays(1)
    else -> date.minusDays((date.dayOfWeek.value - 1).toLong())
}

@Composable
private fun Grid(monday: LocalDate, week: ScheduleView?, axis: TimetableAxis?, rowHeight: Dp, modifier: Modifier) {
    val model = LocalModel.current
    val days = (0L until 5L).map { monday.plusDays(it) }
    val height = rowHeight * (axis?.length ?: 12.5).toFloat()
    val today = Format.today()
    val current = model.today?.takeIf { it.phase == TodayPhase.IN_CLASS }?.hero?.occurrence?.key
    Column(modifier, verticalArrangement = Arrangement.spacedBy(Theme.space3)) {
        Row(horizontalArrangement = Arrangement.spacedBy(3.dp)) {
            Spacer(Modifier.width(periodWidth))
            for (day in days) DayHeader(day, day == today, Modifier.weight(1f))
        }
        Row(horizontalArrangement = Arrangement.spacedBy(3.dp)) {
            PeriodColumn(axis, rowHeight, Modifier.width(periodWidth).height(height))
            for (day in days) {
                val agenda = week?.agendas?.firstOrNull { it.date == Format.civil(day) }
                DayColumn(agenda, day == today, week, axis, rowHeight, current, Modifier.weight(1f).height(height))
            }
        }
    }
}

@Composable
private fun DayHeader(day: LocalDate, today: Boolean, modifier: Modifier) {
    Column(modifier, horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(Theme.space1)) {
        Text(Format.weekday(day), style = MaterialTheme.typography.labelMedium, color = Theme.colors.muted)
        Box(
            Modifier.size(30.dp).clip(CircleShape).background(if (today) Theme.colors.accent else Color.Transparent),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                "${day.dayOfMonth}", style = MaterialTheme.typography.titleMedium.tabular(), fontWeight = FontWeight.SemiBold,
                color = if (today) Theme.colors.accentInk else Theme.colors.ink,
            )
        }
    }
}

/** Period numbers at their rows; each block's start time under its first
 * period and its end time under its last. */
@Composable
private fun PeriodColumn(axis: TimetableAxis?, rowHeight: Dp, modifier: Modifier) {
    Box(modifier) {
        for (row in axis?.rows.orEmpty()) {
            val periods = row.periods ?: continue
            if (periods.size != 2) continue
            val first = periods[0].toInt()
            val last = maxOf(first, periods[1].toInt())
            for (period in first..last) {
                Column(
                    Modifier.fillMaxWidth().offset(y = rowHeight * (row.top + (period - first)).toFloat()),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text("$period", style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
                    if (period == first) PeriodTime(row.startTime)
                    if (period == last) PeriodTime(row.endTime)
                }
            }
        }
    }
}

@Composable
private fun PeriodTime(time: String) {
    // A time of five characters is wider than the column: it shrinks to
    // fit, as iOS's minimumScaleFactor(0.8). The floor is in dp because the
    // column is: a floor in sp would grow past it with the system font size.
    val style = MaterialTheme.typography.labelSmall.tabular()
    Text(
        time.removePrefix("0"), style = style,
        color = Theme.colors.muted, maxLines = 1,
        autoSize = TextAutoSize.StepBased(
            minFontSize = with(LocalDensity.current) { (style.fontSize.value * 0.8f).dp.toSp() },
            maxFontSize = style.fontSize,
            stepSize = 0.25.sp,
        ),
    )
}

@Composable
private fun DayColumn(
    agenda: DatedAgenda?,
    today: Boolean,
    week: ScheduleView?,
    axis: TimetableAxis?,
    rowHeight: Dp,
    current: String?,
    modifier: Modifier,
) {
    val shape = RoundedCornerShape(Theme.radiusColumn)
    val holiday = agenda?.noClasses == true
    val blocks = agenda?.occurrences.orEmpty().filter { it.isBlock }
    // A day without classes is a flat grey column; stripes belong to the
    // classes that do not take place (slotledger `approach/timetable-cancelled-stripes`).
    val background = if (holiday) courseMix(Theme.colors.muted, 0.08f, Theme.colors.surface) else Theme.colors.surface
    BoxWithConstraints(modifier.clip(shape).background(background)) {
        val width = maxWidth
        axis?.let { RowRules(it, rowHeight) }
        if (axis != null) {
            // The first read of a week draws placeholder blocks, then fades in.
            Loading(week != null, placeholder = { DaySkeleton(axis, rowHeight) }) {
                Box(Modifier.fillMaxSize()) {
                    for (block in blocks) key(block.key) {
                        Block(block, holiday, week, axis, rowHeight, width, block.key == current)
                    }
                }
            }
            if (today) NowLine(axis, rowHeight)
        }
    }
}

/** Placeholder blocks over every other teaching block while a week loads. */
@Composable
private fun DaySkeleton(axis: TimetableAxis, rowHeight: Dp) {
    Box(Modifier.fillMaxSize()) {
        axis.rows.filter { it.periods != null }.filterIndexed { index, _ -> index % 2 == 0 }.forEach { row ->
            Skeleton(
                Modifier
                    .padding(horizontal = laneGap)
                    .offset(y = rowHeight * row.top.toFloat() + 2.dp)
                    .fillMaxWidth()
                    .height(rowHeight * (row.bottom - row.top).toFloat() - 4.dp),
            )
        }
    }
}

/** Gap between blocks that share a time side by side. */

/** One rule after each teaching block (1–2, 3–4, …), never inside one, so a
 * two-period class is not cut in half; the lunch band is shaded. */
@Composable
private fun RowRules(axis: TimetableAxis, rowHeight: Dp) {
    val line = Theme.colors.line
    val band = Theme.colors.surface2
    Canvas(Modifier.fillMaxSize()) {
        val row = rowHeight.toPx()
        for (each in axis.rows) {
            val top = (each.top * row).toFloat()
            val bottom = (each.bottom * row).toFloat()
            if (each.periods == null) {
                drawRect(band, Offset(0f, top), androidx.compose.ui.geometry.Size(size.width, bottom - top))
            } else if (bottom < size.height - 1) {
                drawLine(line, Offset(0f, bottom), Offset(size.width, bottom), strokeWidth = 1.dp.toPx())
            }
        }
    }
}

/** The current time across today's column, moving with the clock. */
@Composable
private fun NowLine(axis: TimetableAxis, rowHeight: Dp) {
    val now = rememberNow(30)
    val offset = axis.now(now) ?: return
    val danger = Theme.colors.danger
    Canvas(Modifier.fillMaxSize()) {
        val y = (offset * rowHeight.toPx()).toFloat()
        drawCircle(danger, 4.dp.toPx(), Offset(4.dp.toPx(), y))
        drawLine(danger, Offset(0f, y), Offset(size.width, y), strokeWidth = 2.dp.toPx())
    }
}
