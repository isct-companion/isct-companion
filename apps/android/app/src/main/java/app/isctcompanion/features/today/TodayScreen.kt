package app.isctcompanion.features.today

import androidx.compose.animation.core.spring
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.Animatable
import kotlin.math.abs
import app.isctcompanion.features.assignments.todayAssignments
import app.isctcompanion.features.timetable.firstRoom
import app.isctcompanion.design.t
import app.isctcompanion.core.OccurrenceKind
import androidx.compose.ui.zIndex
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.gestures.verticalDrag
import androidx.compose.foundation.gestures.awaitVerticalTouchSlopOrCancellation
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.OpenInNew
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.DeadlineItem
import app.isctcompanion.core.ExamView
import app.isctcompanion.core.MaterialItem
import app.isctcompanion.core.RecentItem
import app.isctcompanion.core.SchoolSite
import app.isctcompanion.core.SyncView
import app.isctcompanion.core.TodayClass
import app.isctcompanion.core.TodayPhase
import app.isctcompanion.core.TodayView
import app.isctcompanion.core.WebRouter
import app.isctcompanion.design.Card
import app.isctcompanion.design.CourseSquare
import app.isctcompanion.design.DeadlineChip
import app.isctcompanion.design.DeadlineLink
import app.isctcompanion.design.DeadlineRow
import app.isctcompanion.design.Format
import app.isctcompanion.design.PageSubtitle
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.MaterialRow
import app.isctcompanion.design.PillButton
import app.isctcompanion.design.ProblemBanner
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.FirstLoadCard
import app.isctcompanion.design.SyncProblemBanner
import app.isctcompanion.design.SyncRingButton
import app.isctcompanion.design.activeRuns
import app.isctcompanion.design.problem
import app.isctcompanion.design.TagChip
import app.isctcompanion.design.Theme
import app.isctcompanion.design.CountingText
import app.isctcompanion.design.courseColor
import app.isctcompanion.design.courseZoomSource
import app.isctcompanion.design.nowTint
import app.isctcompanion.design.tabular
import java.time.Instant
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** A clock for countdowns, ticking every `seconds`. */
@Composable
fun rememberNow(seconds: Long): Instant {
    val now by produceState(Instant.now()) {
        while (true) {
            delay(seconds * 1_000)
            value = Instant.now()
        }
    }
    return now
}

/** 今日: what Core chose for this moment (the class in progress or tonight's
 * deadline), the stack of the next classes, then the quarter's assignments.
 * The shell renders `TodayView` and `AssignmentBoard` in order and never
 * sorts or filters. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TodayScreen() {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val now = rememberNow(30)
    LaunchedEffect(Unit) { model.open(SyncView.Today) }
    // Core's upcoming exams for the next two weeks.
    var exams by remember { mutableStateOf<List<ExamView>>(emptyList()) }
    LaunchedEffect(model.contentRevision, model.today?.date) { exams = model.upcomingExams() }
    TabPage(
        t("Today"),
        actions = { SyncRingButton() },
        onRefresh = { model.refreshFromSchool() },
    ) {
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp)) {
            item(key = "today") {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    PageSubtitle(subtitle(model.today))
                    val problem = model.syncSnapshot?.problem
                    if (model.problem != null) {
                        ProblemBanner(model.problem!!)
                    } else if (problem != null) {
                        SyncProblemBanner(problem)
                    }
                    if (model.needsAccount) ConnectCard { navigator.openSettings() }
                    model.syncSnapshot?.let { snapshot ->
                        if ((snapshot.activeRuns.isNotEmpty() || snapshot.resetting) &&
                            model.records.courses.isEmpty() && model.records.assignments.isEmpty()
                        ) {
                            FirstLoadCard(snapshot)
                        }
                    }
                    model.today?.let { TodayContent(it, now, exams) }
                }
            }
            if (model.today != null) todayAssignments(model.board, now) { navigator.openRecord() }
        }
    }
}

private fun subtitle(today: TodayView?): String {
    today ?: return ""
    val parts = mutableListOf(Format.dayTitle(today.date))
    if (today.noClasses) {
        parts.add(today.calendarNotice ?: t("Holiday"))
    } else {
        today.teachingWeek?.let { parts.add(t("{{quarter}} week {{week}}", "quarter" to Format.quarter(today.term), "week" to it)) }
    }
    return parts.joinToString(" · ")
}

@Composable
private fun TodayContent(today: TodayView, now: Instant, exams: List<ExamView>) {
    when (today.phase) {
        TodayPhase.BEFORE_CLASSES, TodayPhase.BETWEEN_CLASSES -> {
            ClassStack(today.upcoming, now)
            UpcomingExams(exams)
        }
        TodayPhase.IN_CLASS -> {
            today.hero?.let { CurrentClassHero(it, now) }
            if (today.classMaterials.isNotEmpty() || today.classAssignments.isNotEmpty()) {
                SectionLabel(t("This class"))
                ThisClassCard(today.classMaterials, today.classAssignments, now)
            }
            ClassStack(today.upcoming, now)
        }
        TodayPhase.AFTER_CLASSES -> {
            today.deadlineHero?.let { TonightHero(it, now) }
            ClassStack(today.upcoming, now)
            UpcomingExams(exams)
        }
        TodayPhase.NO_CLASSES -> {
            UpcomingExams(exams)
            if (today.recent.isNotEmpty()) {
                SectionLabel(t("New"))
                RecentCard(today.recent)
            }
            ClassStack(today.upcoming, now)
        }
    }
}

// Class stack

private val stackCardHeight = 150.dp
/** How far each card behind shows below the one in front. */
private val stackStep = 16.dp
/** How much narrower each card behind is. */
private const val stackShrink = 0.07f

/** The next class and up to two after it (Core's `upcoming`), stacked like
 * grouped notifications: swipe up for the next card, down to go back. The
 * stack keeps one height, so the page never moves while it turns, and it
 * consumes every vertical drag that starts on it: past either end the front
 * card only stretches, so the drag never reaches the list's scrolling or its
 * pull to refresh. */
@Composable
private fun ClassStack(items: List<TodayClass>, now: Instant) {
    if (items.isEmpty()) return
    var front by remember(items.firstOrNull()?.startsAt) { mutableIntStateOf(0) }
    // Where the stack stands, in cards: `front` at rest, between two while a
    // drag brings the next or previous card in.
    val position = remember(items.firstOrNull()?.startsAt) { Animatable(0f) }
    var stretch by remember { mutableFloatStateOf(0f) }
    val index = front.coerceAtMost(items.lastIndex)
    val current by rememberUpdatedState(index)
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    val turn = with(LocalDensity.current) { 60.dp.toPx() }
    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.weight(1f)) { SectionLabel(stackTitle(items[index], index == 0, now)) }
        if (items.size > 1) {
            Text(
                "${index + 1} / ${items.size}", style = MaterialTheme.typography.bodySmall.tabular(),
                color = Theme.colors.muted, modifier = Modifier.padding(end = 4.dp),
            )
        }
    }
    Box(
        Modifier
            .fillMaxWidth()
            .height(stackCardHeight + stackStep * minOf(items.size - 1, 2))
            .pointerInput(items.size) {
                awaitEachGesture {
                    val down = awaitFirstDown(requireUnconsumed = false)
                    var travel = 0f
                    val start = awaitVerticalTouchSlopOrCancellation(down.id) { change, over ->
                        change.consume()
                        travel = over
                    } ?: return@awaitEachGesture
                    verticalDrag(start.id) { change ->
                        travel += change.positionChange().y
                        change.consume()
                        val open = if (travel < 0) current < items.lastIndex else current > 0
                        if (open) {
                            stretch = 0f
                            val progress = (-travel / turn).coerceIn(-1f, 1f)
                            scope.launch { position.snapTo(current + progress) }
                        } else {
                            // Past an end the card stretches a little and springs back.
                            stretch = travel * 0.2f
                        }
                    }
                    val next = when {
                        travel < -turn / 2 -> (current + 1).coerceAtMost(items.lastIndex)
                        travel > turn / 2 -> (current - 1).coerceAtLeast(0)
                        else -> current
                    }
                    stretch = 0f
                    if (next != current) haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                    front = next
                    scope.launch { position.animateTo(next.toFloat(), spring(stiffness = Spring.StiffnessMediumLow)) }
                }
            },
    ) {
        val height = with(LocalDensity.current) { stackCardHeight.toPx() }
        val step = with(LocalDensity.current) { stackStep.toPx() }
        items.forEachIndexed { at, item ->
            val depth = at - position.value
            Box(
                Modifier
                    .zIndex(items.size - abs(depth))
                    .graphicsLayer {
                        translationY = when {
                            depth < 0 -> depth * height / 3
                            else -> depth * step
                        } + if (at == index) stretch else 0f
                        val scale = if (depth <= 0) 1f else 1f - depth * stackShrink
                        scaleX = scale
                        scaleY = scale
                        transformOrigin = TransformOrigin(0.5f, 0f)
                        alpha = when {
                            depth <= -1 -> 0f
                            depth < 0 -> 1 + depth
                            else -> 1f
                        }
                    },
            ) {
                StackCard(item, now, leads = at == 0, enabled = at == index, zoom = "stack-$at")
                if (depth > 0) {
                    // Cards behind dim a little per level, so the stack reads as layers.
                    Box(
                        Modifier
                            .matchParentSize()
                            .clip(RoundedCornerShape(Theme.radiusPanel))
                            .background(Color.Black.copy(alpha = (0.04f * depth).coerceAtMost(0.1f))),
                    )
                }
            }
        }
    }
}

/** The card's day, said once above the stack: 次の授業 for the first class
 * still to come today, then today, tomorrow or the date. */
@Composable
private fun stackTitle(item: TodayClass, first: Boolean, now: Instant): String {
    val exam = item.occurrence.kind == OccurrenceKind.EXAM
    val date = Format.civilDate(item.occurrence.date)
    val today = Format.today(now)
    return when (date) {
        today -> if (first) t(if (exam) "Next exam" else "Next class") else t("Later today")
        today.plusDays(1) -> t(if (exam) "Tomorrow’s exam" else "Tomorrow’s class")
        else -> Format.dayTitle(item.occurrence.date)
    }
}

/** One class in the stack: periods, start time and room in large figures, and
 * how long until it starts while that is today. */
@Composable
private fun StackCard(item: TodayClass, now: Instant, leads: Boolean, enabled: Boolean, zoom: String) {
    val tint = courseColor(item.occurrence.color)
    val navigator = LocalNavigator.current
    val id = item.occurrence.courseId
    val exam = item.occurrence.kind == OccurrenceKind.EXAM
    val starts = Format.date(item.startsAt)
    val today = starts?.let { Format.tokyo(it).toLocalDate() == Format.today(now) } == true
    Box(
        Modifier
            .fillMaxWidth()
            .height(stackCardHeight)
            .courseZoomSource(zoom.takeIf { enabled && id != null })
            .shadow(6.dp, RoundedCornerShape(Theme.radiusPanel))
            .clip(RoundedCornerShape(Theme.radiusPanel))
            .background(Theme.colors.surface),
    ) {
        Hero(
            Modifier.fillMaxHeight().nowTint(tint, Theme.colors, RoundedCornerShape(Theme.radiusPanel)),
            onClick = id?.takeIf { enabled }?.let { { navigator.openCourse(it, zoom) } },
        ) {
            HeroLine(listOfNotNull(if (exam) t("Exam") else null, Format.periods(item.periods)).joinToString(" · "), if (exam) Theme.colors.warn else tint) {
                if (leads && today && starts != null) {
                    val minutes = maxOf(Format.minutes(starts, now), 0)
                    TagChip(
                        if (minutes < 60) t("{{count}} min left", "count" to minutes) else Format.time(starts),
                        ink = if (exam) Theme.colors.warn else tint, fill = Theme.colors.surface, counting = true,
                    )
                }
            }
            Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(
                    item.occurrence.startTime, fontSize = 40.sp, fontWeight = FontWeight.Bold,
                    style = MaterialTheme.typography.displaySmall.tabular(), color = Theme.colors.ink, maxLines = 1,
                )
                Text(
                    item.occurrence.room, style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.Bold,
                    color = Theme.colors.ink, maxLines = 1, overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.padding(bottom = 4.dp),
                )
            }
            Text(
                item.occurrence.title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold,
                color = Theme.colors.ink, maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

// Heroes

/** A hero card; [surface] is its background, the 'now' tint for a class. */
@Composable
private fun Hero(
    surface: Modifier,
    onClick: (() -> Unit)? = null,
    content: @Composable () -> Unit,
) {
    Column(
        surface
            .fillMaxWidth()
            .clip(RoundedCornerShape(Theme.radiusPanel))
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
            .padding(18.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) { content() }
}

@Composable
private fun HeroLine(text: String, tint: Color, trailing: @Composable () -> Unit = {}) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(
            text, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold, color = tint,
            modifier = Modifier.weight(1f),
        )
        trailing()
    }
}

private val bigFigure = 48.sp

/** The current class card's zoom id; one hero shows at a time. */
private const val HERO_ZOOM = "hero"

/** In class: minutes left, progress and end time. */
@Composable
private fun CurrentClassHero(item: TodayClass, now: Instant) {
    val tint = courseColor(item.occurrence.color)
    val starts = Format.date(item.startsAt) ?: now
    val ends = Format.date(item.endsAt) ?: now
    val total = maxOf(ends.toEpochMilli() - starts.toEpochMilli(), 1L)
    val progress = ((now.toEpochMilli() - starts.toEpochMilli()).toFloat() / total).coerceIn(0f, 1f)
    val navigator = LocalNavigator.current
    val id = item.occurrence.courseId
    Hero(
        Modifier
            .courseZoomSource(HERO_ZOOM.takeIf { id != null })
            .nowTint(tint, Theme.colors, RoundedCornerShape(Theme.radiusPanel)),
        onClick = id?.let { { navigator.openCourse(it, HERO_ZOOM) } },
    ) {
        // The first room only, so the line never wraps; every room is on the
        // course page.
        HeroLine(
            listOfNotNull(
                if (item.occurrence.kind == OccurrenceKind.EXAM) t("Exam in progress") else t("In class"),
                Format.periods(item.periods), firstRoom(item.occurrence.room),
            ).filter { it.isNotEmpty() }.joinToString(" · "),
            tint,
        ) {
            Text(
                t("Until {{time}}", "time" to item.occurrence.endTime), style = MaterialTheme.typography.bodyMedium.tabular(),
                fontWeight = FontWeight.SemiBold, color = tint,
            )
        }
        Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            // The figure sits where the language puts the count: あと 45 分,
            // 45 min left, 还剩 45 分钟.
            val words = t("{{count}} min left").split("{{count}}").map { it.trim() }
            words.first().takeIf { it.isNotEmpty() }?.let {
                Text(it, style = MaterialTheme.typography.titleLarge, color = Theme.colors.ink, modifier = Modifier.padding(bottom = 8.dp))
            }
            CountingText(
                "${maxOf(Format.minutes(ends, now), 0)}", fontSize = bigFigure, fontWeight = FontWeight.Bold,
                style = MaterialTheme.typography.displayMedium.tabular(), color = Theme.colors.ink,
            )
            words.getOrNull(1)?.takeIf { it.isNotEmpty() }?.let {
                Text(it, style = MaterialTheme.typography.titleLarge, color = Theme.colors.ink, modifier = Modifier.padding(bottom = 8.dp))
            }
        }
        LinearProgressIndicator(
            progress = { progress }, color = tint, trackColor = tint.copy(alpha = 0.2f),
            modifier = Modifier.fillMaxWidth(),
        )
        CourseTitle(item.occurrence.courseId != null, item.occurrence.title)
    }
}

/** After classes: tonight's nearest deadline on the danger tint. */
@Composable
private fun TonightHero(item: DeadlineItem, now: Instant) {
    val navigator = LocalNavigator.current
    Hero(Modifier.background(Theme.colors.dangerSoft, RoundedCornerShape(Theme.radiusPanel))) {
        HeroLine(t("Due tonight"), Theme.colors.danger) { DeadlineChip(item, now) }
        Text(
            Format.date(item.dueAt)?.let(Format::time) ?: Format.due(item.dueAt, now), fontSize = bigFigure, fontWeight = FontWeight.Bold,
            style = MaterialTheme.typography.displayMedium.tabular(), color = Theme.colors.ink,
        )
        Text(item.title, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
        item.courseTitle?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted) }
        item.sourceUrl?.let { url ->
            PillButton(t("Open submission page"), Icons.AutoMirrored.Outlined.OpenInNew, Theme.colors.danger) {
                navigator.openPage(WebRouter.PushedPage(SchoolSite.SCIENCE_LMS, url, item.title))
            }
        }
    }
}

// Sections

/** The hero's course title; the chevron says the whole card opens it. */
@Composable
private fun CourseTitle(opens: Boolean, title: String) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(
            title, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold,
            color = Theme.colors.ink, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false),
        )
        if (opens) Icon(Icons.Outlined.ChevronRight, null, tint = Theme.colors.muted, modifier = Modifier.size(18.dp))
    }
}

/** In class: the newest material and open work of this course. */
@Composable
private fun ThisClassCard(materials: List<MaterialItem>, assignments: List<DeadlineItem>, now: Instant) {
    Card {
        materials.forEachIndexed { index, item ->
            if (index > 0) RowDivider()
            MaterialRow(item)
        }
        assignments.forEachIndexed { index, item ->
            if (index > 0 || materials.isNotEmpty()) RowDivider()
            DeadlineLink(item) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp),
                ) {
                    Icon(Icons.Outlined.Description, null, tint = Theme.colors.muted, modifier = Modifier.size(18.dp))
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        Text(item.title, color = Theme.colors.ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
                        Text(Format.due(item.dueAt, now), style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted)
                    }
                    DeadlineChip(item, now)
                }
            }
        }
    }
}

/** Exams in the next two weeks, as Core lists them; a row opens its course. */
@Composable
private fun UpcomingExams(exams: List<ExamView>) {
    if (exams.isEmpty()) return
    val navigator = LocalNavigator.current
    SectionLabel(t("Exam"))
    Card {
        exams.forEachIndexed { index, exam ->
            if (index > 0) RowDivider()
            val id = exam.courseId
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                modifier = Modifier
                    .fillMaxWidth()
                    .then(if (id != null) Modifier.clickable { navigator.openCourse(id) } else Modifier)
                    .padding(horizontal = 16.dp, vertical = 10.dp),
            ) {
                CourseSquare(exam.color)
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(exam.title.ifEmpty { exam.code }, color = Theme.colors.ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(examDetail(exam), style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted, maxLines = 1)
                }
            }
        }
    }
}

/** `10月26日（月） · 3–4限 · W531`, or the time when no period is given. */
private fun examDetail(exam: ExamView): String {
    val time = Format.periods(exam.periods) ?: exam.startTime?.let { start -> listOfNotNull(start, exam.endTime).joinToString("–") }
    return listOfNotNull(Format.dayTitle(exam.date), time, exam.room).filter { it.isNotEmpty() }.joinToString(" · ")
}

/** Day off: grades and materials that changed recently. */
@Composable
private fun RecentCard(items: List<RecentItem>) {
    Card {
        items.forEachIndexed { index, item ->
            if (index > 0) RowDivider()
            when (item) {
                is RecentItem.Grade -> RecentRow(
                    item.v1.color, listOfNotNull(item.v1.title, item.v1.grade).joinToString(" · "), item.v1.courseTitle,
                    item.v1.courseId,
                ) { TagChip(t("Graded"), ink = Theme.colors.ok, fill = Theme.colors.okSoft) }
                is RecentItem.Material -> RecentRow(item.v1.color, item.v1.title, item.v1.courseTitle, item.v1.courseId) {
                    TagChip(t("Material"))
                }
            }
        }
    }
}

@Composable
private fun RecentRow(color: String?, title: String, detail: String?, courseId: String?, tag: @Composable () -> Unit) {
    val navigator = LocalNavigator.current
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        modifier = Modifier
            .fillMaxWidth()
            .then(if (courseId != null) Modifier.clickable { navigator.openCourse(courseId) } else Modifier)
            .padding(horizontal = 16.dp, vertical = 10.dp),
    ) {
        CourseSquare(color)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(title, color = Theme.colors.ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
            detail?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted, maxLines = 1) }
        }
        tag()
    }
}

/** First launch: the real empty Today with one sign-in card. */
@Composable
private fun ConnectCard(onConnect: () -> Unit) {
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Theme.radiusPanel))
            .background(Theme.colors.surface)
            .padding(18.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(t("Connect school accounts"), style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold, color = Theme.colors.ink)
        Text(
            t("Reads your timetable, assignments and materials from the school sites. Your password is encrypted in this device’s keystore and sent only to the school sites."),
            style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted,
        )
        Box(
            Modifier
                .fillMaxWidth()
                .clip(CircleShape)
                .background(Theme.colors.accent)
                .clickable(onClick = onConnect)
                .padding(vertical = 12.dp),
            contentAlignment = Alignment.Center,
        ) {
            Text(t("Connect with Science Tokyo"), color = Color.White, fontWeight = FontWeight.SemiBold)
        }
    }
}
