package app.isctcompanion.features.timetable

import app.isctcompanion.design.Skeleton
import app.isctcompanion.design.Loading
import app.isctcompanion.design.Motion
import app.isctcompanion.design.LocalReducedMotion
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import android.content.Context
import android.content.Intent
import android.widget.Toast
import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Error
import androidx.compose.material.icons.rounded.CalendarMonth
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.EditCalendar
import androidx.compose.material.icons.rounded.IosShare
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.CalendarExportScope
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
import app.isctcompanion.design.Tag
import app.isctcompanion.design.TagVariant
import app.isctcompanion.design.Theme
import app.isctcompanion.design.CountingText
import app.isctcompanion.design.courseColor
import app.isctcompanion.design.courseMix
import app.isctcompanion.design.courseZoomSource
import app.isctcompanion.design.t
import app.isctcompanion.design.tabular
import java.io.File
import java.time.DayOfWeek
import java.time.Instant
import java.time.LocalDate
import kotlin.math.abs
import kotlin.math.floor
import kotlin.math.roundToInt
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private val minimumRow = 30.dp
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
    var monday by rememberSaveable { mutableStateOf(monday(Format.today())) }
    var week by remember { mutableStateOf(model.cachedWeek(monday)) }
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
    // A week read with its neighbours draws at once; the effect below still
    // checks it against the current revision.
    LaunchedEffect(monday) { model.cachedWeek(monday)?.let { week = it } }
    LaunchedEffect(monday, model.contentRevision) {
        // A newer revision or week cancels this read (the query rethrows), so
        // its empty answer never replaces what is shown.
        val loaded = model.week(monday)
        week = loaded
        model.timetableAxis(loaded)?.let { axis = it }
        val number = model.academicYear(monday) ?: return@LaunchedEffect
        if (year?.year != number || year?.quarters?.isEmpty() == true) model.timetableYear(number)?.let { year = it }
    }
    val swipe = with(LocalDensity.current) { 60.dp.toPx() }
    var width by remember { mutableFloatStateOf(1f) }
    // The shown week slides out towards the side it leaves by and the new one
    // comes in from the other; a release that falls short settles back.
    fun go(to: LocalDate, slow: Boolean) = scope.launch {
        if (to == monday) {
            shift.animateTo(0f, Motion.slowSpring())
            return@launch
        }
        if (reduced) {
            shift.snapTo(0f)
            fade.animateTo(0f, Motion.fast())
            monday = to
            fade.animateTo(1f, Motion.fast())
            return@launch
        }
        val direction = if (to.isAfter(monday)) -1f else 1f
        val spec = if (slow) Motion.slow<Float>() else Motion.base<Float>()
        shift.animateTo(direction * width, spec)
        monday = to
        shift.snapTo(-direction * width)
        shift.animateTo(0f, spec)
    }
    // The shown week's quarter, or this quarter between quarters.
    val term = year?.weeks?.firstOrNull { it.monday == Format.civil(monday) }?.term ?: week?.term ?: model.today?.term

    Column(
        Modifier
            .fillMaxSize()
            .background(Theme.colors.bg)
            .windowInsetsPadding(WindowInsets.statusBars)
            .padding(horizontal = 12.dp)
            .padding(bottom = 8.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Header(monday, chip(year, monday), term, openPicker = { picker = true }) { go(thisWeek(), slow = false) }
        GuideTip("timetable-marks", "Swipe to change weeks", "Long-press a class to mark it as an exam or cancelled.")
        // Period rows share the height the screen leaves, so the week fits
        // without scrolling (with a floor for very short windows).
        BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
            val length = axis?.length ?: 12.5
            val rowHeight = maxOf(minimumRow, floor(((maxHeight - dayHeaderHeight).value / length)).dp)
            val columns = constraints.maxWidth.toFloat()
            SideEffect { width = columns }
            Grid(
                monday, week, axis, rowHeight,
                Modifier
                    .offset { IntOffset(shift.value.roundToInt(), 0) }
                    .graphicsLayer { alpha = fade.value }
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
            )
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
    Column(Modifier.padding(top = 12.dp, start = 4.dp, end = 4.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        // The way into the week picker, a secondary button.
        ActionButton(
            chip, openPicker,
            Modifier.semantics { contentDescription = t("Choose a week") },
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
                t("Today"), toToday,
                Modifier.semantics { contentDescription = t("Back to this week") },
                level = ButtonLevel.Secondary, enabled = !atThisWeek,
            )
        }
    }
}

/** The shown quarter as an .ics, written by Core as the desktop's ICSを書き出す,
 * handed to the share sheet (Calendar, Drive, Mail). */
@Composable
private fun ExportButton(term: String) {
    val model = LocalModel.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var menu by remember { mutableStateOf(false) }
    fun export(scope0: CalendarExportScope) = scope.launch {
        menu = false
        runCatching { share(context, term, scope0, model.quarterIcs(term, scope0)) }.onFailure {
            Toast.makeText(context, t("Export failed: {{reason}}", "reason" to AppModel.describe(it)), Toast.LENGTH_LONG).show()
        }
    }
    Box {
        IconButton(onClick = { menu = true }) { Icon(Icons.Rounded.IosShare, t("Export ICS"), tint = Theme.colors.accent) }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            Text(
                t("Export ICS") + " · " + Format.quarter(term), style = MaterialTheme.typography.labelMedium,
                color = Theme.colors.muted, modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
            )
            DropdownMenuItem(
                text = { Text(t("Courses only")) }, leadingIcon = { Icon(Icons.Rounded.CalendarMonth, null) },
                onClick = { export(CalendarExportScope.COURSES) },
            )
            DropdownMenuItem(
                text = { Text(t("Courses + weekly events")) }, leadingIcon = { Icon(Icons.Rounded.EditCalendar, null) },
                onClick = { export(CalendarExportScope.COURSES_AND_EVENTS) },
            )
        }
    }
}

private suspend fun share(context: Context, term: String, scope: CalendarExportScope, text: String) {
    val suffix = if (scope == CalendarExportScope.COURSES_AND_EVENTS) "-with-events" else ""
    val file = withContext(Dispatchers.IO) {
        File(context.cacheDir, "exports").apply { mkdirs() }.resolve("IsctCompanion-$term$suffix.ics").apply { writeText(text) }
    }
    val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
    val send = Intent(Intent.ACTION_SEND).setType("text/calendar").putExtra(Intent.EXTRA_STREAM, uri)
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    context.startActivity(Intent.createChooser(send, file.name))
}

/** "2026年度 · 3Q 第 2 週", from Core's week list; between quarters, 休み. */
private fun chip(year: TimetableYear?, monday: LocalDate): String {
    year ?: return t("Timetable")
    val label = year.weeks.firstOrNull { it.monday == Format.civil(monday) }?.let { weekLabel(it, year) }
    return listOfNotNull(t("AY {{year}}", "year" to year.year), label).joinToString(" · ")
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
    Column(modifier, verticalArrangement = Arrangement.spacedBy(6.dp)) {
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
    Column(modifier, horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(2.dp)) {
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

/** Period numbers at their rows; each block's start time under its first. */
@Composable
private fun PeriodColumn(axis: TimetableAxis?, rowHeight: Dp, modifier: Modifier) {
    Box(modifier) {
        for (row in axis?.rows.orEmpty()) {
            val periods = row.periods ?: continue
            if (periods.size != 2) continue
            val first = periods[0].toInt()
            for (period in first..maxOf(first, periods[1].toInt())) {
                Column(
                    Modifier.fillMaxWidth().offset(y = rowHeight * (row.top + (period - first)).toFloat()),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text("$period", style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
                    if (period == first) {
                        Text(
                            row.startTime.removePrefix("0"), style = MaterialTheme.typography.labelSmall.tabular(),
                            color = Theme.colors.muted, maxLines = 1,
                        )
                    }
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
                    for (block in blocks) Block(block, holiday, week, axis, rowHeight, width, block.key == current)
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
private val laneGap = 2.dp

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun Block(
    block: Occurrence,
    holiday: Boolean,
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
    val cancelled = block.cancelled == true || holiday
    val exam = block.kind == OccurrenceKind.EXAM && !cancelled
    val quiz = exam && block.quiz == true
    val tint = courseColor(block.color)
    val surface = Theme.colors.surface
    val shape = RoundedCornerShape(Theme.radiusBlock)
    val id = block.courseId
    // The current mark, checked in the long-press menu.
    val state = when {
        block.cancelled == true -> "cancelled"
        block.kind != OccurrenceKind.EXAM -> "class"
        block.quiz == true -> "quiz"
        else -> "exam"
    }
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
                    // Planned courses have no sessions of their own to mark.
                    onLongClick = if (planned) null else ({
                        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                        menu = true
                    }),
                )
                .padding(4.dp),
        ) {
            val style = MaterialTheme.typography.labelSmall
            // An exam or quiz names itself first, so even a short block says what it is.
            if (exam) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(3.dp)) {
                    ExamTag(quiz, tint)
                    if (block.sourceChanged == true) {
                        Icon(Icons.Rounded.Error, t("Timetable changed"), tint = Theme.colors.warn, modifier = Modifier.size(12.dp))
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
                    planned -> t("Planned")
                    cancelled -> t("No class")
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
                t("This lesson"), style = MaterialTheme.typography.labelMedium, color = Theme.colors.muted,
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
            )
            HorizontalDivider()
            // Choosing the checked mark again sends `class`; Core decides
            // whether that clears the mark.
            for ((value, label) in listOf("exam" to t("Mark as exam"), "quiz" to t("Mark as quiz"), "cancelled" to t("Mark as cancelled"))) {
                DropdownMenuItem(
                    text = { Text(label) },
                    trailingIcon = { if (value == state) Icon(Icons.Rounded.Check, null, tint = Theme.colors.accent) },
                    onClick = {
                        menu = false
                        scope.launch {
                            model.markSession(block.key, if (value == state) "class" else value)
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
        Tag(t("Quiz"), color = tint, variant = TagVariant.Outline, ink = courseMix(tint, Theme.tintInk, Theme.colors.ink))
    } else {
        Tag(t("Exam"), color = tint, variant = TagVariant.Solid)
    }
}

/** The first of several rooms joined by commas; every room stays on the course page. */
internal fun firstRoom(room: String): String =
    room.split(',', '、', '，').firstOrNull()?.trim() ?: room

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
    var now by remember { mutableStateOf(Instant.now()) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(30_000)
            now = Instant.now()
        }
    }
    val offset = axis.now(now) ?: return
    val danger = Theme.colors.danger
    Canvas(Modifier.fillMaxSize()) {
        val y = (offset * rowHeight.toPx()).toFloat()
        drawCircle(danger, 4.dp.toPx(), Offset(4.dp.toPx(), y))
        drawLine(danger, Offset(0f, y), Offset(size.width, y), strokeWidth = 2.dp.toPx())
    }
}

/** Diagonal stripes over a class that does not take place. */
private fun Modifier.stripes(color: Color): Modifier = drawBehind {
    val step = 8.dp.toPx()
    var x = -size.height
    while (x < size.width) {
        drawLine(color, Offset(x, size.height), Offset(x + size.height, 0f), strokeWidth = 3.dp.toPx())
        x += step
    }
}
