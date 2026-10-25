package app.isctcompanion.features.timetable

import android.content.Context
import android.content.Intent
import android.widget.Toast
import androidx.compose.animation.core.animateFloatAsState
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
import androidx.compose.material.icons.filled.Error
import androidx.compose.material.icons.outlined.CalendarMonth
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.EditCalendar
import androidx.compose.material.icons.outlined.IosShare
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
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
import androidx.compose.ui.unit.sp
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
import app.isctcompanion.design.Format
import app.isctcompanion.design.GuideTip
import app.isctcompanion.design.Theme
import app.isctcompanion.design.CountingText
import app.isctcompanion.design.courseColor
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
    var drag by remember { mutableFloatStateOf(0f) }
    val offset by animateFloatAsState(drag, label = "week drag")
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
        Header(monday, chip(year, monday), term, openPicker = { picker = true }) { monday = thisWeek() }
        GuideTip("timetable-marks", "Swipe to change weeks", "Long-press a class to mark it as an exam or cancelled.")
        // Period rows share the height the screen leaves, so the week fits
        // without scrolling (with a floor for very short windows).
        BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
            val length = axis?.length ?: 12.5
            val rowHeight = maxOf(minimumRow, floor(((maxHeight - dayHeaderHeight).value / length)).dp)
            Grid(
                monday, week, axis, rowHeight,
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
    if (picker) {
        WeekPicker(year, monday, dismiss = { picker = false }) { picked ->
            monday = picked
            picker = false
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
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            modifier = Modifier
                .clip(CircleShape)
                .background(Theme.colors.accentSoft)
                .clickable(onClick = openPicker)
                .padding(horizontal = 10.dp, vertical = 5.dp)
                .semantics { contentDescription = t("Choose a week") },
        ) {
            Icon(Icons.Outlined.CalendarMonth, null, tint = Theme.colors.accent, modifier = Modifier.size(16.dp))
            Text(
                chip, style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.SemiBold, color = Theme.colors.accent,
                maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
        }
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
            FilledTonalButton(
                onClick = toToday, enabled = !atThisWeek,
                colors = ButtonDefaults.filledTonalButtonColors(
                    containerColor = Theme.colors.accentSoft, contentColor = Theme.colors.accent,
                    disabledContainerColor = Theme.colors.accentSoft.copy(alpha = 0.35f),
                    disabledContentColor = Theme.colors.accent.copy(alpha = 0.35f),
                ),
                modifier = Modifier.semantics { contentDescription = t("Back to this week") },
            ) { Text(t("Today"), fontWeight = FontWeight.SemiBold) }
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
        IconButton(onClick = { menu = true }) { Icon(Icons.Outlined.IosShare, t("Export ICS"), tint = Theme.colors.accent) }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            Text(
                t("Export ICS") + " · " + Format.quarter(term), style = MaterialTheme.typography.labelMedium,
                color = Theme.colors.muted, modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
            )
            DropdownMenuItem(
                text = { Text(t("Courses only")) }, leadingIcon = { Icon(Icons.Outlined.CalendarMonth, null) },
                onClick = { export(CalendarExportScope.COURSES) },
            )
            DropdownMenuItem(
                text = { Text(t("Courses + personal plans")) }, leadingIcon = { Icon(Icons.Outlined.EditCalendar, null) },
                onClick = { export(CalendarExportScope.COURSES_AND_PLANS) },
            )
        }
    }
}

private suspend fun share(context: Context, term: String, scope: CalendarExportScope, text: String) {
    val suffix = if (scope == CalendarExportScope.COURSES_AND_PLANS) "-with-plans" else ""
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
                color = if (today) Color.White else Theme.colors.ink,
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
                            row.startTime.removePrefix("0"), fontSize = 9.sp, style = MaterialTheme.typography.labelSmall.tabular(),
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
    val shape = RoundedCornerShape(12.dp)
    val holiday = agenda?.noClasses == true
    val blocks = agenda?.occurrences.orEmpty().filter { it.isBlock }
    Box(
        modifier
            .clip(shape)
            .background(Theme.colors.surface)
            .then(if (today) Modifier.border(1.5.dp, Theme.colors.accent.copy(alpha = 0.5f), shape) else Modifier),
    ) {
        axis?.let { RowRules(it, rowHeight) }
        if (holiday) Hatch(Theme.colors.line)
        if (axis != null) {
            for (block in blocks) Block(block, holiday, week, axis, rowHeight, block.key == current)
            if (today) NowLine(axis, rowHeight)
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun Block(block: Occurrence, holiday: Boolean, week: ScheduleView?, axis: TimetableAxis, rowHeight: Dp, current: Boolean) {
    val navigator = LocalNavigator.current
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    var menu by remember { mutableStateOf(false) }
    val top = rowHeight * axis.offset(block.startTime).toFloat()
    val bottom = rowHeight * axis.offset(block.endTime).toFloat()
    val planned = block.planned == true
    val cancelled = block.cancelled == true
    val exam = block.kind == OccurrenceKind.EXAM && !cancelled
    val quiz = exam && block.quiz == true
    val faded = holiday || cancelled
    val tint = courseColor(block.color)
    val shape = RoundedCornerShape(8.dp)
    val id = block.courseId
    // The current mark, checked in the long-press menu.
    val state = when {
        cancelled -> "cancelled"
        block.kind != OccurrenceKind.EXAM -> "class"
        block.quiz == true -> "quiz"
        else -> "exam"
    }
    Box(Modifier.padding(horizontal = 2.dp).offset(y = top + 2.dp).fillMaxWidth()) {
        Column(
            Modifier
                .fillMaxWidth()
                .height(maxOf(bottom - top - 4.dp, 24.dp))
                .courseZoomSource(block.key.takeIf { id != null })
                .clip(shape)
                // An exam is the strongest fill, a quiz a little weaker;
                // neither is striped, which means cancelled.
                .background(
                    if (planned) Theme.colors.surface
                    else tint.copy(alpha = when {
                        faded -> 0.08f
                        quiz -> Theme.tintQuiz
                        exam -> Theme.tintExam
                        else -> Theme.tintFill
                    }),
                )
                .then(
                    // The class in progress and an exam ring in the course
                    // colour, which outranks a quiz's lighter ring.
                    when {
                        current || (exam && !quiz) -> Modifier.border(2.dp, tint, shape)
                        quiz -> Modifier.border(1.5.dp, tint.copy(alpha = Theme.tintOutline), shape)
                        planned -> Modifier.border(1.dp, Theme.colors.line, shape)
                        else -> Modifier
                    },
                )
                .combinedClickable(
                    enabled = id != null || !planned,
                    onClick = { id?.let { navigator.openCourse(it, block.key) } },
                    // Planned courses have no sessions of their own to mark.
                    onLongClick = if (planned) null else ({ menu = true }),
                )
                .padding(4.dp),
        ) {
            val ink = if (faded) Theme.colors.muted else Theme.colors.ink
            Text(
                block.title, style = MaterialTheme.typography.labelSmall, fontWeight = FontWeight.SemiBold, color = ink,
                textDecoration = if (cancelled) TextDecoration.LineThrough else null,
                maxLines = 4, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false),
            )
            Spacer(Modifier.weight(1f))
            // 計画, 休講, or the first room (an exam or quiz tags it first, with
            // a warning when the published exam changed).
            val room = firstRoom(block.room)
            val line = @Composable { text: String ->
                Text(
                    text, style = MaterialTheme.typography.labelSmall, fontWeight = FontWeight.SemiBold, color = ink,
                    maxLines = 1, overflow = TextOverflow.Ellipsis,
                )
            }
            when {
                planned -> line(t("Planned"))
                cancelled -> line(t("No class"))
                exam -> Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(3.dp)) {
                    ExamTag(quiz, tint)
                    if (block.sourceChanged == true) {
                        Icon(Icons.Filled.Error, t("Timetable changed"), tint = Theme.colors.warn, modifier = Modifier.size(12.dp))
                    }
                    line(room)
                }
                else -> line(room)
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
                    trailingIcon = { if (value == state) Icon(Icons.Outlined.Check, null, tint = Theme.colors.accent) },
                    onClick = {
                        menu = false
                        scope.launch { model.markSession(block.key, if (value == state) "class" else value) }
                    },
                )
            }
        }
    }
}

/** 試験 before an exam block's room, solid in the course colour; a quiz's
 * 小テスト is outlined (design/README.md). */
@Composable
private fun ExamTag(quiz: Boolean, tint: Color) {
    val shape = RoundedCornerShape(50)
    Text(
        if (quiz) t("Quiz") else t("Exam"),
        style = MaterialTheme.typography.labelSmall, fontWeight = FontWeight.Bold, maxLines = 1,
        // The course palette is mid-tone in both themes.
        color = if (quiz) tint else Color.White,
        modifier = Modifier
            .then(if (quiz) Modifier.border(1.dp, tint, shape) else Modifier.background(tint, shape))
            .padding(horizontal = 4.dp),
    )
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
