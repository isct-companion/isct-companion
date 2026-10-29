package app.isctcompanion.features.assignments

import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import app.isctcompanion.core.AssignmentBoard
import app.isctcompanion.core.SyncView
import app.isctcompanion.design.t
import androidx.compose.material3.TextButton
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.Spacer
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CheckCircle
import app.isctcompanion.design.EmptyState
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.PushPin
import androidx.compose.material.icons.rounded.Visibility
import androidx.compose.material.icons.rounded.VisibilityOff
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.rememberSwipeToDismissBoxState
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
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.DeadlineGroups
import app.isctcompanion.core.DeadlineItem
import app.isctcompanion.design.DeadlineLink
import app.isctcompanion.design.DeadlineRow
import app.isctcompanion.design.Format
import app.isctcompanion.design.GuideTip
import app.isctcompanion.design.PageSubtitle
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.ProblemBanner
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.Theme
import app.isctcompanion.features.today.rememberNow
import java.time.Instant
import kotlinx.coroutines.launch

/** 今日's 課題: what is left to do this quarter, in the groups of Core's task
 * timeline (`AssignmentBoard.today`), as items of 今日's list. Swipe right
 * pins, swipe left hides; every assignment, submitted and hidden included,
 * sits behind すべて. */
fun LazyListScope.todayAssignments(board: AssignmentBoard?, now: Instant, openRecord: () -> Unit) {
    board ?: return
    item(key = "assignments-header") {
        Row(
            verticalAlignment = Alignment.Bottom,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            modifier = Modifier.fillMaxWidth().padding(top = 20.dp, start = 4.dp),
        ) {
            Text(t("common.assignments"), style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold, color = Theme.colors.ink)
            Text(
                t("assignments.quarterUnsubmittedOnly", "quarter" to Format.quarter(board.term)),
                style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted,
                modifier = Modifier.weight(1f).padding(bottom = 3.dp),
            )
            TextButton(onClick = openRecord) { Text(t("assignments.seeAll")) }
        }
    }
    val today = board.today
    if (!today.isEmpty()) {
        item(key = "assignments-tip") {
            GuideTip("assignment-swipe", "guide.pinOrHide", "guide.swipeRightToPinAn", Modifier.padding(top = 8.dp))
        }
    }
    group(t("common.pinned"), today.pinned, now, Format.OverdueStyle.Elapsed)
    group(t("common.thisWeek"), today.dueThisWeek, now, Format.OverdueStyle.Elapsed)
    group(t("assignments.nextWeekAndLater"), today.later, now, Format.OverdueStyle.Elapsed)
    group(t("common.overdue"), today.pastRecent, now, Format.OverdueStyle.Elapsed)
    if (today.isEmpty()) {
        item(key = "assignments-empty") {
            EmptyState(Icons.Rounded.CheckCircle, t("assignments.noUnsubmittedAssignments"))
        }
    }
}

private fun DeadlineGroups.isEmpty() =
    pinned.isEmpty() && dueThisWeek.isEmpty() && later.isEmpty() && undated.isEmpty() && pastRecent.isEmpty()

/** One titled card of rows; open rows pin and hide by swiping. */
private fun LazyListScope.group(
    title: String,
    items: List<DeadlineItem>,
    now: Instant,
    overdueStyle: Format.OverdueStyle = Format.OverdueStyle.Label,
    restores: Boolean = false,
) {
    if (items.isEmpty()) return
    item(key = "label-$title") { Box(Modifier.padding(top = 12.dp, bottom = 6.dp)) { SectionLabel(title) } }
    items.forEachIndexed { index, row ->
        item(key = "$title-${row.assignmentId}") {
            val shape = RoundedCornerShape(
                topStart = if (index == 0) Theme.radiusSurface else 0.dp,
                topEnd = if (index == 0) Theme.radiusSurface else 0.dp,
                bottomStart = if (index == items.lastIndex) Theme.radiusSurface else 0.dp,
                bottomEnd = if (index == items.lastIndex) Theme.radiusSurface else 0.dp,
            )
            Column(Modifier.clip(shape).background(Theme.colors.surface)) {
                if (index > 0) RowDivider()
                if (restores) RestoreRow(row, now) else SwipeRow(row, now, overdueStyle)
            }
        }
    }
}

@Composable
private fun GroupCard(content: @Composable () -> Unit) {
    Box(Modifier.fillMaxWidth().clip(RoundedCornerShape(Theme.radiusSurface)).background(Theme.colors.surface)) { content() }
}

/** Swipe right: pin (or unpin); swipe left: hide. The row always springs back;
 * Core's refreshed board moves it. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SwipeRow(item: DeadlineItem, now: Instant, overdueStyle: Format.OverdueStyle) {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    // Row swipes leave the left 20 dp to the system back gesture (slotledger `approach/mobile-gestures`).
    val edge = with(LocalDensity.current) { 20.dp.toPx() }
    var fromEdge by remember { mutableStateOf(false) }
    val state = rememberSwipeToDismissBoxState(
        confirmValueChange = { value ->
            when (value) {
                SwipeToDismissBoxValue.StartToEnd -> scope.launch {
                    model.setPinned(item, !item.pinned)
                    haptics.performHapticFeedback(HapticFeedbackType.Confirm)
                }
                SwipeToDismissBoxValue.EndToStart -> scope.launch {
                    model.setHidden(item, true)
                    haptics.performHapticFeedback(HapticFeedbackType.Confirm)
                }
                SwipeToDismissBoxValue.Settled -> Unit
            }
            false
        },
    )
    SwipeToDismissBox(
        state = state,
        gesturesEnabled = !fromEdge,
        modifier = Modifier.pointerInput(Unit) {
            awaitPointerEventScope {
                while (true) {
                    val event = awaitPointerEvent(PointerEventPass.Initial)
                    val down = event.changes.firstOrNull { it.pressed && !it.previousPressed } ?: continue
                    fromEdge = down.position.x < edge
                }
            }
        },
        backgroundContent = {
            when (state.dismissDirection) {
                SwipeToDismissBoxValue.StartToEnd ->
                    SwipeAction(Theme.colors.accent, if (item.pinned) t("common.unpin") else t("common.pin"), Icons.Rounded.PushPin, Alignment.CenterStart)
                SwipeToDismissBoxValue.EndToStart ->
                    SwipeAction(Theme.colors.muted, t("common.hide"), Icons.Rounded.VisibilityOff, Alignment.CenterEnd, Theme.colors.surface)
                SwipeToDismissBoxValue.Settled -> Unit
            }
        },
    ) {
        Box(Modifier.background(Theme.colors.surface)) {
            DeadlineLink(item) { DeadlineRow(item, now, overdueStyle) }
        }
    }
}

/** A hidden row: it opens and has its menu (with Restore) as on iOS, and a
 * swipe left brings it back. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun RestoreRow(item: DeadlineItem, now: Instant) {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    val state = rememberSwipeToDismissBoxState(
        confirmValueChange = { value ->
            if (value == SwipeToDismissBoxValue.EndToStart) scope.launch { model.setHidden(item, false) }
            false
        },
    )
    SwipeToDismissBox(
        state = state,
        enableDismissFromStartToEnd = false,
        backgroundContent = { SwipeAction(Theme.colors.accent, t("common.restore"), Icons.Rounded.Visibility, Alignment.CenterEnd) },
    ) {
        Box(Modifier.background(Theme.colors.surface)) {
            DeadlineLink(item, hidden = true) { DeadlineRow(item, now) }
        }
    }
}

@Composable
private fun SwipeAction(fill: Color, label: String, icon: ImageVector, alignment: Alignment, ink: Color = Theme.colors.accentInk) {
    Box(Modifier.fillMaxSize().background(fill).padding(horizontal = 20.dp), contentAlignment = alignment) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Icon(icon, null, tint = ink)
            Text(label, color = ink, fontWeight = FontWeight.SemiBold)
        }
    }
}

/** Every assignment of the quarter whatever its state (`AssignmentBoard.record`);
 * older past work stays folded, hidden work comes back from here. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RecordScreen() {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val now = rememberNow(60)
    var unfolded by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) { model.open(SyncView.Assignments) }
    val record = model.board?.record
    Column(Modifier.fillMaxSize().background(Theme.colors.bg)) {
        TopAppBar(
            title = { Text(t("common.allAssignments")) },
            navigationIcon = {
                IconButton(onClick = { navigator.back() }) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, t("common.back")) }
            },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = Theme.colors.bg),
        )
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp)) {
            if (record != null) {
                group(t("common.pinned"), record.pinned, now)
                group(t("common.thisWeek"), record.dueThisWeek, now)
                group(t("assignments.nextWeekAndLater"), record.later, now)
                group(t("common.noDeadline"), record.undated, now)
                group(t("common.past"), record.pastRecent, now)
                if (record.pastEarlier.isNotEmpty()) {
                    if (unfolded) {
                        group(t("common.earlier"), record.pastEarlier, now)
                    } else {
                        item {
                            Box(Modifier.padding(top = 12.dp)) {
                                GroupCard {
                                    Text(
                                        t("assignments.showCountEarlier", "count" to record.pastEarlier.size), color = Theme.colors.accent,
                                        modifier = Modifier.fillMaxWidth().clickable { unfolded = true }.padding(16.dp),
                                    )
                                }
                            }
                        }
                    }
                }
                group(t("assignments.hidden"), record.hidden, now, restores = true)
            }
        }
    }
}
