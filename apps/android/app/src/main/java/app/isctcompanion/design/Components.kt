package app.isctcompanion.design

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.Link
import androidx.compose.material.icons.automirrored.outlined.MenuBook
import androidx.compose.material.icons.outlined.Place
import androidx.compose.material.icons.outlined.PlayCircleOutline
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.LargeTopAppBar
import androidx.compose.material3.Scaffold
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.DeadlineItem
import app.isctcompanion.core.MaterialItem
import app.isctcompanion.core.SchoolSite
import app.isctcompanion.core.WebRouter
import java.time.Instant
import kotlinx.coroutines.launch

private val tabular = androidx.compose.ui.text.TextStyle(fontFeatureSettings = "tnum")

/** Text style with tabular figures, for times and countdowns. */
@Composable
fun androidx.compose.ui.text.TextStyle.tabular() = merge(tabular)

/** Deadline chip: one per row, coloured by Core's tone. */
@Composable
fun DeadlineChip(item: DeadlineItem, now: Instant, overdueStyle: Format.OverdueStyle = Format.OverdueStyle.Label) {
    val (ink, fill) = toneColors(if (item.submitted) "done" else item.tone)
    Chip(
        if (item.submitted) item.grade ?: "提出済" else Format.countdown(item.dueAt, now, overdueStyle),
        ink, fill,
    )
}

@Composable
fun toneColors(tone: String): Pair<Color, Color> = with(Theme.colors) {
    when (tone) {
        "past", "urgent" -> danger to dangerSoft
        "soon" -> warn to warnSoft
        "done" -> ok to okSoft
        else -> muted to surface2
    }
}

/** Small text chip (`今日`, `資料`, `採点済`). */
@Composable
fun TagChip(text: String, ink: Color = Theme.colors.accent, fill: Color = Theme.colors.accentSoft) = Chip(text, ink, fill)

@Composable
private fun Chip(text: String, ink: Color, fill: Color) {
    Text(
        text,
        style = MaterialTheme.typography.labelMedium.tabular(),
        fontWeight = FontWeight.SemiBold,
        color = ink,
        maxLines = 1,
        modifier = Modifier
            .clip(RoundedCornerShape(Theme.radiusChip))
            .background(fill)
            .padding(horizontal = 7.dp, vertical = 3.dp),
    )
}

/** Assignment row: course square, title, due · course, one chip. */
@Composable
fun DeadlineRow(
    item: DeadlineItem,
    now: Instant,
    overdueStyle: Format.OverdueStyle = Format.OverdueStyle.Label,
    showsCourse: Boolean = true,
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp),
    ) {
        CourseSquare(item.color)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(
                item.title,
                style = MaterialTheme.typography.bodyLarge,
                color = if (item.submitted) Theme.colors.muted else Theme.colors.ink,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            // The deadline leads: a long course name truncates, the date must not.
            val due = Format.due(item.dueAt, now)
            Text(
                if (showsCourse && item.courseTitle != null) "$due · ${item.courseTitle}" else due,
                style = MaterialTheme.typography.bodyMedium,
                color = Theme.colors.muted,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        DeadlineChip(item, now, overdueStyle)
    }
}

/** Opens the assignment's submission page; long press offers quick actions. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun DeadlineLink(item: DeadlineItem, content: @Composable () -> Unit) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val scope = rememberCoroutineScope()
    var menu by remember { mutableStateOf(false) }
    Box(
        Modifier.combinedClickable(
            onClick = { navigator.openPage(WebRouter.PushedPage(SchoolSite.SCIENCE_LMS, item.sourceUrl, item.title)) },
            onLongClick = { menu = true },
        ),
    ) {
        content()
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            item.courseId?.let { course ->
                MenuItem("授業を開く", Icons.AutoMirrored.Outlined.MenuBook) {
                    menu = false
                    navigator.openCourse(course)
                }
            }
            MenuItem(if (item.pinned) "ピン留めを外す" else "ピン留め", Icons.Outlined.PushPin) {
                menu = false
                scope.launch { model.setPinned(item, !item.pinned) }
            }
            MenuItem("非表示", Icons.Outlined.VisibilityOff, Theme.colors.danger) {
                menu = false
                scope.launch { model.setHidden(item, true) }
            }
        }
    }
}

@Composable
private fun MenuItem(text: String, icon: ImageVector, tint: Color = Theme.colors.ink, onClick: () -> Unit) {
    DropdownMenuItem(
        text = { Text(text, color = tint) },
        leadingIcon = { Icon(icon, null, tint = tint) },
        onClick = onClick,
    )
}

@Composable
fun MaterialRow(item: MaterialItem) {
    val context = LocalContext.current
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        modifier = Modifier
            .fillMaxWidth()
            .clickable {
                item.url?.let { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(it))) }
            }
            .padding(horizontal = 16.dp, vertical = 10.dp),
    ) {
        Icon(
            when (item.kind) {
                "pdf" -> Icons.Outlined.Description
                "video" -> Icons.Outlined.PlayCircleOutline
                else -> Icons.Outlined.Link
            },
            null, tint = Theme.colors.muted, modifier = Modifier.size(18.dp),
        )
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(item.title, color = Theme.colors.ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
            // LMS section titles usually carry their own number; fall back to it.
            (item.sectionTitle ?: item.section?.let { "第${it}回" })?.let {
                Text(it, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted, maxLines = 1)
            }
        }
        if (item.addedToday) TagChip("今日追加")
    }
}

/** Opens the system maps app at `query`. */
@Composable
fun MapButton(query: String, title: String, tint: Color) {
    val context = LocalContext.current
    PillButton(title, Icons.Outlined.Place, tint) {
        context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse("geo:0,0?q=" + Uri.encode(query))))
    }
}

/** White label on a filled capsule. */
@Composable
fun PillButton(title: String, icon: ImageVector?, fill: Color, onClick: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        modifier = Modifier
            .clip(CircleShape)
            .background(fill)
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 10.dp),
    ) {
        icon?.let { Icon(it, null, tint = Color.White, modifier = Modifier.size(18.dp)) }
        Text(title, color = Color.White, fontWeight = FontWeight.SemiBold)
    }
}

@Composable
fun ProblemBanner(text: String) {
    val model = LocalModel.current
    Row(
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Theme.radiusSurface))
            .background(Theme.colors.dangerSoft)
            .padding(start = 14.dp, top = 4.dp, bottom = 4.dp),
    ) {
        Icon(Icons.Outlined.ErrorOutline, null, tint = Theme.colors.danger, modifier = Modifier.padding(top = 10.dp))
        Text(
            text, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.ink,
            modifier = Modifier.weight(1f).padding(vertical = 10.dp),
        )
        IconButton(onClick = { model.dismissProblem() }) {
            Icon(Icons.Outlined.Close, "閉じる", tint = Theme.colors.muted, modifier = Modifier.size(18.dp))
        }
    }
}

/** A tab's page (or a pushed one, with `navigationIcon`): Material's collapsing large title over scrolling content,
 * pull to refresh when `onRefresh` is given. The content scrolls itself. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TabPage(
    title: String,
    navigationIcon: @Composable () -> Unit = {},
    actions: @Composable RowScope.() -> Unit = {},
    onRefresh: (suspend () -> Unit)? = null,
    content: @Composable () -> Unit,
) {
    val scroll = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
    val scope = rememberCoroutineScope()
    var refreshing by remember { mutableStateOf(false) }
    Scaffold(
        modifier = Modifier.nestedScroll(scroll.nestedScrollConnection),
        containerColor = Theme.colors.bg,
        contentWindowInsets = WindowInsets(0, 0, 0, 0),
        topBar = {
            LargeTopAppBar(
                title = { Text(title, fontWeight = FontWeight.Bold, maxLines = 2, overflow = TextOverflow.Ellipsis) },
                navigationIcon = navigationIcon,
                actions = actions,
                scrollBehavior = scroll,
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = Theme.colors.bg, scrolledContainerColor = Theme.colors.bg,
                ),
            )
        },
    ) { padding ->
        Box(Modifier.padding(padding)) {
            if (onRefresh == null) {
                content()
            } else {
                PullToRefreshBox(
                    isRefreshing = refreshing,
                    onRefresh = {
                        scope.launch {
                            refreshing = true
                            onRefresh()
                            refreshing = false
                        }
                    },
                ) { content() }
            }
        }
    }
}

/** The muted line under a page title: date, quarter, week. */
@Composable
fun PageSubtitle(text: String) {
    Text(
        text, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted,
    )
}
