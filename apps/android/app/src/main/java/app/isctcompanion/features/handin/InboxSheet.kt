package app.isctcompanion.features.handin

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.HandInAvailability
import app.isctcompanion.core.SharedInbox
import app.isctcompanion.design.DeadlineRow
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import app.isctcompanion.shared.Thumbnail
import java.time.Instant
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** The current selection and the assignments that can receive it. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun InboxSheet() {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val scope = rememberCoroutineScope()
    SharedInbox.error?.let { error ->
        AlertDialog(
            onDismissRequest = SharedInbox::dismissError,
            text = { Text(error) },
            confirmButton = { TextButton(onClick = SharedInbox::dismissError) { Text(t("common.close")) } },
        )
    }
    val selection = SharedInbox.selection ?: return
    var started by remember(selection) { mutableStateOf(false) }
    var error by remember(selection) { mutableStateOf<String?>(null) }
    val now = remember(selection) { Instant.now() }

    fun give(target: SharedInbox.Target) {
        error = null
        scope.launch {
            try {
                SharedInbox.give(selection, target, model)?.let(navigator::openPage)
            } catch (failure: CancellationException) {
                throw failure
            } catch (failure: Exception) {
                error = AppModel.describe(failure)
                model.refresh()
            }
        }
    }

    LaunchedEffect(selection, SharedInbox.working) {
        if (!SharedInbox.working && !started) {
            started = true
            selection.target?.let(::give)
        }
    }
    ModalBottomSheet(onDismissRequest = { SharedInbox.discard(selection) }, containerColor = Theme.colors.bg) {
        Text(
            t("common.handInTo"), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold,
            modifier = Modifier.padding(horizontal = Theme.space8, vertical = Theme.space4),
        )
        val groups = model.board?.today
        TextButton(onClick = { SharedInbox.discard(selection) }) { Text(t("common.cancel")) }
        LazyColumn {
            item(key = "files") {
                LazyRow(
                    horizontalArrangement = Arrangement.spacedBy(Theme.space6),
                    contentPadding = PaddingValues(horizontal = Theme.space8, vertical = Theme.space2),
                ) {
                    items(selection.files, key = { it.file.path }) { item ->
                        Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.width(80.dp)) {
                            Thumbnail(item.file, Modifier.size(72.dp, 92.dp))
                            Text(item.name, style = MaterialTheme.typography.labelSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                    }
                }
            }
            if (SharedInbox.working) item(key = "working") {
                CircularProgressIndicator(Modifier.padding(Theme.space8))
            }
            error?.let { message ->
                item(key = "error") { Text(message, color = Theme.colors.danger, modifier = Modifier.padding(Theme.space8)) }
            }
            if (groups != null) {
                for ((title, rows) in listOf(
                    t("common.pinned") to groups.pinned,
                    t("common.thisWeek") to groups.dueThisWeek,
                    t("assignments.nextWeekAndLater") to groups.later,
                    t("common.noDeadline") to groups.undated,
                    t("common.past") to groups.pastRecent,
                )) {
                    val available = rows.filter { it.handInAvailability == HandInAvailability.PREPARE_DRAFT }
                    if (available.isEmpty()) continue
                    item(key = title) { SectionLabel(title) }
                    items(available, key = { "$title-${it.assignmentId}" }) { row ->
                        Box(Modifier.clickable(enabled = !SharedInbox.working) {
                            give(SharedInbox.Target(row.assignmentId, row.title))
                        }) { DeadlineRow(row, now) }
                    }
                }
            }
        }
    }
}
