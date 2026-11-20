package app.isctcompanion.features.handin

import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.DeadlineItem
import app.isctcompanion.core.SharedInbox
import app.isctcompanion.design.DeadlineRow
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import java.time.Instant
import kotlinx.coroutines.launch
import app.isctcompanion.core.give
import app.isctcompanion.shared.Thumbnail

/** The upload panel: what waits in the inbox, then the quarter's unsubmitted
 * assignments as 今日 groups them. Picking one puts the files in its upload
 * card and opens its page, where 「LMS にアップロード」 is the next step. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun InboxSheet(close: () -> Unit) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    LaunchedEffect(SharedInbox.items.isEmpty()) { if (SharedInbox.items.isEmpty()) close() }
    ModalBottomSheet(onDismissRequest = close, containerColor = Theme.colors.bg) {
        Text(
            t("common.handInTo"), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold,
            modifier = Modifier.padding(horizontal = Theme.space8, vertical = Theme.space4),
        )
        AssignmentPicker(
            header = {
                LazyRow(
                    horizontalArrangement = Arrangement.spacedBy(Theme.space6),
                    contentPadding = PaddingValues(horizontal = Theme.space8, vertical = Theme.space2),
                ) {
                    items(SharedInbox.items, key = { it.id }) { item ->
                        Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.width(80.dp)) {
                            Box {
                                Thumbnail(item.file, Modifier.size(72.dp, 92.dp))
                                IconButton(onClick = {
                                    SharedInbox.remove(context, item)
                                    haptics.performHapticFeedback(HapticFeedbackType.Confirm)
                                }, modifier = Modifier.align(Alignment.TopEnd).size(28.dp)) {
                                    Icon(Icons.Rounded.Delete, t("common.delete"), tint = Theme.colors.muted, modifier = Modifier.size(16.dp))
                                }
                            }
                            Text(item.name, style = MaterialTheme.typography.labelSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                    }
                }
            },
        ) { assignment ->
            scope.launch {
                if (model.give(SharedInbox.items, assignment)) {
                    close()
                    navigator.openAssignment(assignment)
                }
            }
        }
    }
}

/** The quarter's unsubmitted assignments, as 今日 groups them, under `header`. */
@Composable
private fun AssignmentPicker(header: @Composable () -> Unit, pick: (String) -> Unit) {
    val groups = LocalModel.current.board?.today
    val now = remember { Instant.now() }
    LazyColumn {
        item(key = "header") { header() }
        if (groups == null) return@LazyColumn
        for ((title, rows) in listOf(
            t("common.pinned") to groups.pinned,
            t("common.thisWeek") to groups.dueThisWeek,
            t("assignments.nextWeekAndLater") to groups.later,
            t("common.noDeadline") to groups.undated,
            t("common.past") to groups.pastRecent,
        )) {
            if (rows.isEmpty()) continue
            item(key = title) { SectionLabel(title) }
            items(rows, key = { "$title-${it.assignmentId}" }) { row: DeadlineItem ->
                Box(Modifier.clickable { pick(row.assignmentId) }) { DeadlineRow(row, now) }
            }
        }
    }
}
