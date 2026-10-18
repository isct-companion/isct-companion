package app.isctcompanion.features.assignments

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
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material.icons.outlined.VisibilityOff
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
import app.isctcompanion.design.PageSubtitle
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.ProblemBanner
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.Theme
import app.isctcompanion.features.today.rememberNow
import java.time.Instant
import kotlinx.coroutines.launch

/** 課題: what is left to do this quarter, in the groups of Core's task
 * timeline (`AssignmentBoard.today`). Swipe right pins, swipe left hides;
 * every assignment, submitted and hidden included, sits behind one row. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AssignmentsScreen() {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val now = rememberNow(60)
    val board = model.board
    TabPage("課題", onRefresh = { model.pull() }) {
            LazyColumn(
                Modifier.fillMaxSize(),
                contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp),
            ) {
                item { PageSubtitle("${Format.quarter(board?.term.orEmpty())} · 未提出のみ") }
                model.problem?.let { item { Box(Modifier.padding(top = 12.dp)) { ProblemBanner(it) } } }
                if (board != null) {
                    val today = board.today
                    group("ピン留め", today.pinned, now, Format.OverdueStyle.Elapsed)
                    group("今週", today.dueThisWeek, now, Format.OverdueStyle.Elapsed)
                    group("来週以降", today.later, now, Format.OverdueStyle.Elapsed)
                    group("期限切れ", today.pastRecent, now, Format.OverdueStyle.Elapsed)
                    if (today.isEmpty()) {
                        item {
                            Column(
                                Modifier.fillMaxWidth().padding(vertical = 48.dp),
                                horizontalAlignment = Alignment.CenterHorizontally,
                                verticalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                Icon(Icons.Outlined.CheckCircle, null, tint = Theme.colors.muted, modifier = Modifier.size(40.dp))
                                Text("未提出の課題はありません", style = MaterialTheme.typography.titleMedium, color = Theme.colors.ink)
                            }
                        }
                    }
                    item {
                        Box(Modifier.padding(top = 24.dp)) {
                            GroupCard {
                                Row(
                                    verticalAlignment = Alignment.CenterVertically,
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .clickable { navigator.openRecord() }
                                        .padding(horizontal = 16.dp, vertical = 14.dp),
                                ) {
                                    Text("提出済・非表示を含むすべて", color = Theme.colors.ink, modifier = Modifier.weight(1f))
                                    Icon(Icons.Outlined.ChevronRight, null, tint = Theme.colors.muted)
                                }
                            }
                        }
                    }
                }
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
    // Row swipes leave the left 20 dp to the system back gesture (docs/ux/mobile.md).
    val edge = with(LocalDensity.current) { 20.dp.toPx() }
    var fromEdge by remember { mutableStateOf(false) }
    val state = rememberSwipeToDismissBoxState(
        confirmValueChange = { value ->
            when (value) {
                SwipeToDismissBoxValue.StartToEnd -> scope.launch { model.setPinned(item, !item.pinned) }
                SwipeToDismissBoxValue.EndToStart -> scope.launch { model.setHidden(item, true) }
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
                    SwipeAction(Theme.colors.accent, if (item.pinned) "外す" else "ピン留め", Icons.Outlined.PushPin, Alignment.CenterStart)
                SwipeToDismissBoxValue.EndToStart ->
                    SwipeAction(Theme.colors.muted, "非表示", Icons.Outlined.VisibilityOff, Alignment.CenterEnd)
                SwipeToDismissBoxValue.Settled -> Unit
            }
        },
    ) {
        Box(Modifier.background(Theme.colors.surface)) {
            DeadlineLink(item) { DeadlineRow(item, now, overdueStyle) }
        }
    }
}

/** A hidden row: swipe left to bring it back. */
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
        backgroundContent = { SwipeAction(Theme.colors.accent, "戻す", Icons.Outlined.Visibility, Alignment.CenterEnd) },
    ) {
        Box(Modifier.background(Theme.colors.surface)) { DeadlineRow(item, now) }
    }
}

@Composable
private fun SwipeAction(fill: Color, label: String, icon: ImageVector, alignment: Alignment) {
    Box(Modifier.fillMaxSize().background(fill).padding(horizontal = 20.dp), contentAlignment = alignment) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Icon(icon, null, tint = Color.White)
            Text(label, color = Color.White, fontWeight = FontWeight.SemiBold)
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
    val record = model.board?.record
    Column(Modifier.fillMaxSize().background(Theme.colors.bg)) {
        TopAppBar(
            title = { Text("すべての課題") },
            navigationIcon = {
                IconButton(onClick = { navigator.back() }) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, "戻る") }
            },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = Theme.colors.bg),
        )
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp)) {
            if (record != null) {
                group("ピン留め", record.pinned, now)
                group("今週", record.dueThisWeek, now)
                group("来週以降", record.later, now)
                group("期限なし", record.undated, now)
                group("過去", record.pastRecent, now)
                if (record.pastEarlier.isNotEmpty()) {
                    if (unfolded) {
                        group("それ以前", record.pastEarlier, now)
                    } else {
                        item {
                            Box(Modifier.padding(top = 12.dp)) {
                                GroupCard {
                                    Text(
                                        "それ以前の ${record.pastEarlier.size} 件を表示", color = Theme.colors.accent,
                                        modifier = Modifier.fillMaxWidth().clickable { unfolded = true }.padding(16.dp),
                                    )
                                }
                            }
                        }
                    }
                }
                group("非表示", record.hidden, now, restores = true)
            }
        }
    }
}
