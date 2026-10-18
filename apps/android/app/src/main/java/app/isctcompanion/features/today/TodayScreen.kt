package app.isctcompanion.features.today

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.OpenInNew
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.DeadlineItem
import app.isctcompanion.core.MaterialItem
import app.isctcompanion.core.RecentItem
import app.isctcompanion.core.SchoolSite
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
import app.isctcompanion.design.MapButton
import app.isctcompanion.design.MaterialRow
import app.isctcompanion.design.PillButton
import app.isctcompanion.design.ProblemBanner
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.TagChip
import app.isctcompanion.design.Theme
import app.isctcompanion.design.courseColor
import app.isctcompanion.design.tabular
import app.isctcompanion.features.settings.SettingsSheet
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

/** 今日: one hero chosen by Core for this moment, then at most two short
 * sections. The shell renders `TodayView` in order and never sorts or filters. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TodayScreen() {
    val model = LocalModel.current
    var showsSettings by remember { mutableStateOf(false) }
    val now = rememberNow(30)
    TabPage(
        "今日",
        actions = {
            if (model.isPulling) {
                CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                Spacer(Modifier.width(12.dp))
            }
            IconButton(onClick = { showsSettings = true }) { Icon(Icons.Outlined.Settings, "設定") }
        },
        onRefresh = { model.pull() },
    ) {
        Column(
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp)
                .padding(bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            PageSubtitle(subtitle(model.today))
            model.problem?.let { ProblemBanner(it) }
            if (model.needsAccount) ConnectCard { showsSettings = true }
            model.today?.let { TodayContent(it, now) }
        }
    }
    if (showsSettings) SettingsSheet { showsSettings = false }
}

private fun subtitle(today: TodayView?): String {
    today ?: return ""
    val parts = mutableListOf(Format.dayTitle(today.date))
    if (today.noClasses) {
        parts.add(today.calendarNotice ?: "休日")
    } else {
        today.teachingWeek?.let { parts.add("${Format.quarter(today.term)} 第${it}週") }
    }
    return parts.joinToString(" · ")
}

@Composable
private fun TodayContent(today: TodayView, now: Instant) {
    when (today.phase) {
        TodayPhase.BEFORE_CLASSES, TodayPhase.BETWEEN_CLASSES -> {
            today.hero?.let { NextClassHero(it, now) }
            if (today.later.isNotEmpty()) {
                SectionLabel("このあと")
                ClassList(today.later)
            }
            DeadlineSection("期限", today.deadlines, now)
        }
        TodayPhase.IN_CLASS -> {
            today.hero?.let { CurrentClassHero(it, now) }
            if (today.classMaterials.isNotEmpty() || today.classAssignments.isNotEmpty()) {
                SectionLabel("この授業")
                ThisClassCard(today.classMaterials, today.classAssignments, now)
            }
            val next = today.later.firstOrNull()
            if (next != null && today.hero != null) {
                SectionLabel("次")
                ClassList(listOf(next), breakAfter = today.hero)
            }
            DeadlineSection("期限", today.deadlines, now)
        }
        TodayPhase.AFTER_CLASSES -> {
            today.deadlineHero?.let { TonightHero(it, now) }
            today.nextClass?.let { next ->
                val tomorrow = Format.civilDate(next.occurrence.date) == Format.today(now).plusDays(1)
                SectionLabel(if (tomorrow) "明日の授業" else "次の授業")
                NextDayCard(next)
            }
            DeadlineSection("期限", today.deadlines, now)
        }
        TodayPhase.NO_CLASSES -> {
            DeadlineSection("期限", today.deadlines, now)
            if (today.recent.isNotEmpty()) {
                SectionLabel("新着")
                RecentCard(today.recent)
            }
            today.nextClass?.let {
                SectionLabel("次の授業")
                NextDayCard(it)
            }
        }
    }
}

// Heroes

@Composable
private fun Hero(fill: Color, content: @Composable () -> Unit) {
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Theme.radiusPanel))
            .background(fill)
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

/** Before or between classes: start time and room in large figures. */
@Composable
private fun NextClassHero(item: TodayClass, now: Instant) {
    val tint = courseColor(item.occurrence.color)
    Hero(tint.copy(alpha = Theme.tintSoft)) {
        HeroLine(listOfNotNull("次の授業", Format.periods(item.periods)).joinToString(" · "), tint) {
            Format.date(item.startsAt)?.let {
                TagChip("あと ${maxOf(Format.minutes(it, now), 0)} 分", ink = tint, fill = Theme.colors.surface)
            }
        }
        Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(
                item.occurrence.startTime, fontSize = bigFigure, fontWeight = FontWeight.Bold,
                style = MaterialTheme.typography.displayMedium.tabular(), color = Theme.colors.ink, maxLines = 1,
            )
            Text(
                item.occurrence.room, style = MaterialTheme.typography.headlineMedium, fontWeight = FontWeight.Bold,
                color = Theme.colors.ink, maxLines = 1, overflow = TextOverflow.Ellipsis,
                modifier = Modifier.padding(bottom = 6.dp),
            )
        }
        CourseLink(item.occurrence.courseId, item.occurrence.title)
        item.mapQuery?.let {
            MapButton(it, "${item.occurrence.room.split("-").first()} 棟への地図", tint)
        }
    }
}

/** In class: minutes left, progress and end time. */
@Composable
private fun CurrentClassHero(item: TodayClass, now: Instant) {
    val tint = courseColor(item.occurrence.color)
    val starts = Format.date(item.startsAt) ?: now
    val ends = Format.date(item.endsAt) ?: now
    val total = maxOf(ends.toEpochMilli() - starts.toEpochMilli(), 1L)
    val progress = ((now.toEpochMilli() - starts.toEpochMilli()).toFloat() / total).coerceIn(0f, 1f)
    Hero(tint.copy(alpha = Theme.tintSoft)) {
        HeroLine(
            listOfNotNull("授業中", Format.periods(item.periods), item.occurrence.room)
                .filter { it.isNotEmpty() }.joinToString(" · "),
            tint,
        ) {
            Text(
                "${item.occurrence.endTime} まで", style = MaterialTheme.typography.bodyMedium.tabular(),
                fontWeight = FontWeight.SemiBold, color = tint,
            )
        }
        Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Text("あと", style = MaterialTheme.typography.titleLarge, color = Theme.colors.ink, modifier = Modifier.padding(bottom = 8.dp))
            Text(
                "${maxOf(Format.minutes(ends, now), 0)}", fontSize = bigFigure, fontWeight = FontWeight.Bold,
                style = MaterialTheme.typography.displayMedium.tabular(), color = Theme.colors.ink,
            )
            Text("分", style = MaterialTheme.typography.titleLarge, color = Theme.colors.ink, modifier = Modifier.padding(bottom = 8.dp))
        }
        LinearProgressIndicator(
            progress = { progress }, color = tint, trackColor = tint.copy(alpha = 0.2f),
            modifier = Modifier.fillMaxWidth(),
        )
        CourseLink(item.occurrence.courseId, item.occurrence.title)
    }
}

/** After classes: tonight's nearest deadline on the danger tint. */
@Composable
private fun TonightHero(item: DeadlineItem, now: Instant) {
    val navigator = LocalNavigator.current
    Hero(Theme.colors.dangerSoft) {
        HeroLine("今夜の期限", Theme.colors.danger) { DeadlineChip(item, now) }
        Text(
            Format.due(item.dueAt, now).replace("今日 ", ""), fontSize = bigFigure, fontWeight = FontWeight.Bold,
            style = MaterialTheme.typography.displayMedium.tabular(), color = Theme.colors.ink,
        )
        Text(item.title, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
        item.courseTitle?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted) }
        item.sourceUrl?.let { url ->
            PillButton("提出ページを開く", Icons.AutoMirrored.Outlined.OpenInNew, Theme.colors.danger) {
                navigator.openPage(WebRouter.PushedPage(SchoolSite.SCIENCE_LMS, url, item.title))
            }
        }
    }
}

// Sections

@Composable
private fun CourseLink(id: String?, title: String) {
    val navigator = LocalNavigator.current
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        modifier = if (id != null) Modifier.clickable { navigator.openCourse(id) } else Modifier,
    ) {
        Text(
            title, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold,
            color = Theme.colors.ink, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false),
        )
        if (id != null) Icon(Icons.Outlined.ChevronRight, null, tint = Theme.colors.muted, modifier = Modifier.size(18.dp))
    }
}

/** Rows of classes: time, course square, title and periods, room. */
@Composable
private fun ClassList(items: List<TodayClass>, breakAfter: TodayClass? = null) {
    val navigator = LocalNavigator.current
    Card {
        items.forEachIndexed { index, item ->
            if (index > 0) RowDivider(inset = 76.dp)
            val id = item.occurrence.courseId
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                modifier = Modifier
                    .fillMaxWidth()
                    .then(if (id != null) Modifier.clickable { navigator.openCourse(id) } else Modifier)
                    .padding(horizontal = 16.dp, vertical = 12.dp),
            ) {
                Text(
                    item.occurrence.startTime, style = MaterialTheme.typography.titleMedium.tabular(),
                    fontWeight = FontWeight.SemiBold, color = Theme.colors.ink, modifier = Modifier.width(48.dp),
                )
                CourseSquare(item.occurrence.color)
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(item.occurrence.title, color = Theme.colors.ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    detail(item, breakAfter)?.let {
                        Text(it, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted)
                    }
                }
                Text(item.occurrence.room, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
            }
        }
    }
}

private fun detail(item: TodayClass, breakAfter: TodayClass?): String? {
    val parts = listOfNotNull(Format.periods(item.periods)).toMutableList()
    val ends = Format.date(breakAfter?.endsAt)
    val starts = Format.date(item.startsAt)
    if (ends != null && starts != null) parts.add("休憩 ${(starts.epochSecond - ends.epochSecond) / 60} 分")
    return parts.takeIf { it.isNotEmpty() }?.joinToString(" · ")
}

@Composable
private fun DeadlineSection(title: String, items: List<DeadlineItem>, now: Instant) {
    if (items.isEmpty()) return
    SectionLabel(title)
    Card {
        items.forEachIndexed { index, item ->
            if (index > 0) RowDivider()
            DeadlineLink(item) { DeadlineRow(item, now) }
        }
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

/** After classes or on a day off: the next class day's first class. */
@Composable
private fun NextDayCard(item: TodayClass) {
    val navigator = LocalNavigator.current
    val date = Format.civilDate(item.occurrence.date)
    val id = item.occurrence.courseId
    Card {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
            modifier = Modifier
                .fillMaxWidth()
                .then(if (id != null) Modifier.clickable { navigator.openCourse(id) } else Modifier)
                .padding(14.dp),
        ) {
            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
                modifier = Modifier
                    .size(width = 52.dp, height = 56.dp)
                    .clip(RoundedCornerShape(12.dp))
                    .background(Theme.colors.surface2),
            ) {
                Text(
                    date?.let { Format.weekday(it) }.orEmpty(), style = MaterialTheme.typography.labelMedium,
                    fontWeight = FontWeight.SemiBold, color = Theme.colors.muted,
                )
                Text(
                    date?.dayOfMonth?.toString().orEmpty(), style = MaterialTheme.typography.titleLarge.tabular(),
                    fontWeight = FontWeight.Bold, color = Theme.colors.ink,
                )
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.Bottom) {
                    Text(
                        item.occurrence.startTime, style = MaterialTheme.typography.titleLarge.tabular(),
                        fontWeight = FontWeight.Bold, color = Theme.colors.ink,
                    )
                    Text(
                        item.occurrence.room, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold,
                        color = Theme.colors.ink, maxLines = 1, overflow = TextOverflow.Ellipsis,
                    )
                }
                Text(item.occurrence.title, color = Theme.colors.ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            Icon(Icons.Outlined.ChevronRight, null, tint = Theme.colors.muted, modifier = Modifier.size(18.dp))
        }
    }
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
                ) { TagChip("採点済", ink = Theme.colors.ok, fill = Theme.colors.okSoft) }
                is RecentItem.Material -> RecentRow(item.v1.color, item.v1.title, item.v1.courseTitle, item.v1.courseId) {
                    TagChip("資料")
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
        Text("学校アカウントを接続", style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold, color = Theme.colors.ink)
        Text(
            "時間割・課題・資料を学校のサイトから読み込みます。パスワードはこの端末のキーストアで暗号化して保存され、学校のサイトにだけ送られます。",
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
            Text("Science Tokyo で接続", color = Color.White, fontWeight = FontWeight.SemiBold)
        }
    }
}
