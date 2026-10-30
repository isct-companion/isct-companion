package app.isctcompanion.design

import androidx.compose.foundation.background
import androidx.compose.foundation.border
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
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.Link
import androidx.compose.material.icons.automirrored.rounded.MenuBook
import androidx.compose.material.icons.automirrored.rounded.OpenInNew
import androidx.compose.material.icons.rounded.PlayCircleOutline
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.PushPin
import androidx.compose.material.icons.rounded.Visibility
import androidx.compose.material.icons.rounded.VisibilityOff
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.DeadlineItem
import app.isctcompanion.core.ExamConflict
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
    Tag(
        if (item.submitted) item.grade ?: t("common.submitted") else Format.countdown(item.dueAt, now, overdueStyle),
        color = ink, soft = fill,
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

/** Core's `ExamConflict` on an exam row: the published exam changed after the
 * user corrected it, or left the timetable. Resolving it stays on desktop. */
@Composable
fun ExamConflictLabel(conflict: ExamConflict) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        Icon(Icons.Rounded.ErrorOutline, null, tint = Theme.colors.warn, modifier = Modifier.size(14.dp))
        Text(
            if (conflict.removed) t("exam.noLongerInThePublished") else t("common.timetableChanged"),
            style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.SemiBold, color = Theme.colors.warn,
            maxLines = 1, overflow = TextOverflow.Ellipsis,
        )
    }
}

/** How a [Tag] is drawn (slotledger `approach/chip-variants`). */
enum class TagVariant {
    /** The role's soft fill and its colour as text: the default. */
    Soft,
    /** A line in the role's colour, such as a quiz. */
    Outline,
    /** The role's colour as the fill: an exam only. */
    Solid,
}

/** The one small label (`今日`, `資料`, `採点済`, a deadline, 試験). A role
 * is [color] with its [soft] fill: neutral (muted / surface2), accent, ok,
 * warn, danger, or a course colour. [ink] is the text of an outline tag,
 * [onSolid] of a solid one. Never a bare count. `counting` rolls the figures
 * as they change (`CountingText`). */
@Composable
fun Tag(
    text: String,
    color: Color = Theme.colors.accent,
    soft: Color = Theme.colors.accentSoft,
    variant: TagVariant = TagVariant.Soft,
    ink: Color = color,
    onSolid: Color = Theme.onSolid,
    counting: Boolean = false,
) {
    val shape = RoundedCornerShape(Theme.radiusChip)
    val box = Modifier
        .clip(shape)
        .then(
            when (variant) {
                TagVariant.Soft -> Modifier.background(soft)
                TagVariant.Outline -> Modifier.border(1.dp, color, shape)
                TagVariant.Solid -> Modifier.background(color)
            },
        )
        .padding(horizontal = 6.dp, vertical = 2.dp)
    val tone = when (variant) {
        TagVariant.Soft -> color
        TagVariant.Outline -> ink
        TagVariant.Solid -> onSolid
    }
    val style = MaterialTheme.typography.labelSmall.tabular()
    if (counting) {
        CountingText(text, box, style = style, color = tone, fontWeight = FontWeight.SemiBold)
    } else {
        Text(text, style = style, fontWeight = FontWeight.SemiBold, color = tone, maxLines = 1, modifier = box)
    }
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
        horizontalArrangement = Arrangement.spacedBy(Theme.rowGap),
        modifier = Modifier.fillMaxWidth().padding(horizontal = Theme.rowX, vertical = Theme.rowY),
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
            // Files put in its upload card wait for the person to hand them
            // in; Core counts them until LMS shows the work submitted.
            LocalModel.current.waiting[item.assignmentId]?.let { waiting ->
                Text(
                    t("components.countFilesToHandIn", "count" to waiting),
                    style = MaterialTheme.typography.labelMedium,
                    fontWeight = FontWeight.SemiBold,
                    color = Theme.colors.accent,
                    maxLines = 1,
                )
            }
        }
        DeadlineChip(item, now, overdueStyle)
    }
}

/** Opens the assignment's page, read from LMS by Core; long press offers
 * quick actions, among them the LMS submission page. */
@Composable
fun DeadlineLink(item: DeadlineItem, chevron: Boolean = false, hidden: Boolean = false, content: @Composable () -> Unit) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val scope = rememberCoroutineScope()
    var menu by remember { mutableStateOf(false) }
    Box(
        Modifier.combinedClickable(
            onClick = { navigator.openAssignment(item.assignmentId) },
            onLongClick = { menu = true },
        ),
    ) {
        if (chevron) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.weight(1f)) { content() }
                RowChevron(Modifier.padding(end = 16.dp))
            }
        } else {
            content()
        }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            item.courseId?.let { course ->
                MenuItem(t("common.openCourse"), Icons.AutoMirrored.Rounded.MenuBook) {
                    menu = false
                    navigator.openCourse(course)
                }
            }
            item.sourceUrl?.let { url ->
                MenuItem(t("common.submitInLMS"), Icons.AutoMirrored.Rounded.OpenInNew) {
                    menu = false
                    navigator.openPage(WebRouter.PushedPage(SchoolSite.SCIENCE_LMS, url, item.title))
                }
            }
            if (hidden) {
                MenuItem(t("common.restore"), Icons.Rounded.Visibility) {
                    menu = false
                    scope.launch { model.setHidden(item, false) }
                }
            } else {
                MenuItem(if (item.pinned) t("common.unpin") else t("common.pin"), Icons.Rounded.PushPin) {
                    menu = false
                    scope.launch { model.setPinned(item, !item.pinned) }
                }
                MenuItem(t("common.hide"), Icons.Rounded.VisibilityOff, Theme.colors.danger) {
                    menu = false
                    scope.launch { model.setHidden(item, true) }
                }
            }
        }
    }
}

/** A card row that pushes a page ends in this, as iOS's `CardLink` does;
 * hero cards, which open as a whole, carry none. */
@Composable
fun RowChevron(modifier: Modifier = Modifier) {
    Icon(Icons.Rounded.ChevronRight, null, tint = Theme.colors.muted, modifier = modifier.size(18.dp))
}

@Composable
private fun MenuItem(text: String, icon: ImageVector, tint: Color = Theme.colors.ink, onClick: () -> Unit) {
    DropdownMenuItem(
        text = { Text(text, color = tint) },
        leadingIcon = { Icon(icon, null, tint = tint) },
        onClick = onClick,
    )
}

/** A material listed outside its lesson (今日); Core opens it through the
 * course's folder. */
@Composable
fun MaterialRow(item: MaterialItem) {
    val opener = rememberMaterialOpener(item.title, item.url, null, item.courseId)
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        modifier = Modifier
            .fillMaxWidth()
            .clickable(enabled = !opener.opening) { opener.open() }
            .padding(horizontal = 16.dp, vertical = 10.dp),
    ) {
        Icon(
            when (item.kind) {
                "pdf" -> Icons.Rounded.Description
                "video" -> Icons.Rounded.PlayCircleOutline
                else -> Icons.Rounded.Link
            },
            null, tint = Theme.colors.muted, modifier = Modifier.size(18.dp),
        )
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(item.title, color = Theme.colors.ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
            item.sectionLabel?.let {
                Text(it, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted, maxLines = 1)
            }
        }
        if (opener.opening) {
            CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
        } else if (item.addedToday) {
            Tag(t("common.addedToday"))
        }
    }
}

/** A button's level (slotledger `approach/button-hierarchy`). */
enum class ButtonLevel {
    /** Accent fill: at most one on a screen. */
    Primary,
    /** Accent-soft fill, such as 今日 on the timetable. */
    Secondary,
    /** Accent text alone. */
    Text,
}

/** The app's capsule button. [danger] puts the danger role in the accent's
 * place for a destructive action. */
@Composable
fun ActionButton(
    title: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    level: ButtonLevel = ButtonLevel.Primary,
    icon: ImageVector? = null,
    danger: Boolean = false,
    enabled: Boolean = true,
) {
    val colors = Theme.colors
    val role = if (danger) colors.danger else colors.accent
    val (fill, ink) = when (level) {
        ButtonLevel.Primary -> role to (if (danger) colors.surface else colors.accentInk)
        ButtonLevel.Secondary -> (if (danger) colors.dangerSoft else colors.accentSoft) to role
        ButtonLevel.Text -> Color.Transparent to role
    }
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp, Alignment.CenterHorizontally),
        modifier = modifier
            .clip(CircleShape)
            .background(fill)
            .clickable(enabled = enabled, onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 10.dp)
            .alpha(if (enabled) 1f else 0.35f),
    ) {
        icon?.let { Icon(it, null, tint = ink, modifier = Modifier.size(18.dp)) }
        Text(title, color = ink, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/** An empty list or page: a muted system icon, one line, and at most one
 * action (slotledger `approach/empty-state`). */
@Composable
fun EmptyState(icon: ImageVector, text: String, modifier: Modifier = Modifier, action: Pair<String, () -> Unit>? = null) {
    Column(
        modifier.fillMaxWidth().padding(vertical = 32.dp, horizontal = Theme.gutter),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Icon(icon, null, tint = Theme.colors.muted, modifier = Modifier.size(40.dp))
        Text(text, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted, textAlign = TextAlign.Center)
        action?.let { (title, run) -> ActionButton(title, run, level = ButtonLevel.Secondary) }
    }
}

/** A problem that stops the page: one line on the danger tint at the top,
 * with Retry when the page can try again (slotledger
 * `approach/feedback-banner-toast`). */
@Composable
fun ProblemBanner(text: String, onRetry: (() -> Unit)? = null) {
    val model = LocalModel.current
    Row(
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Theme.radiusSurface))
            .background(Theme.colors.dangerSoft)
            .padding(start = 14.dp, top = 4.dp, bottom = 4.dp),
    ) {
        Icon(Icons.Rounded.ErrorOutline, null, tint = Theme.colors.danger, modifier = Modifier.padding(top = 10.dp))
        Text(
            text, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.ink,
            modifier = Modifier.weight(1f).padding(vertical = 10.dp),
        )
        onRetry?.let { retry ->
            TextButton(onClick = retry) { Text(t("common.retry"), fontWeight = FontWeight.SemiBold, color = Theme.colors.danger) }
        }
        IconButton(onClick = { model.dismissProblem() }) {
            Icon(Icons.Rounded.Close, t("common.close"), tint = Theme.colors.muted, modifier = Modifier.size(18.dp))
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
