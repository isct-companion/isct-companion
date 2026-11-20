package app.isctcompanion.features.handin

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.unit.dp
import app.isctcompanion.core.SharedInbox
import app.isctcompanion.shared.Thumbnail

// The phones' hand-in (slotledger `approach/share-entry-points`), as on iOS: what was shared
// into the app waits in `SharedInbox`; the person gives it to an assignment,
// Core puts it in that assignment's workspace `submit/`, and the assignment
// page's card fills the LMS draft area from there. Core never submits.

/** What was shared into the app, as a small stack of thumbnails above the
 * navigation bar; tapping it opens the inbox. */
@Composable
fun InboxTray(modifier: Modifier = Modifier) {
    val items = SharedInbox.items
    var open by remember { mutableStateOf(false) }
    if (items.isEmpty()) return
    Box(modifier.size(76.dp, 84.dp).clickable { open = true }, contentAlignment = Alignment.Center) {
        items.takeLast(3).forEachIndexed { index, item ->
            Thumbnail(item.file, Modifier.size(52.dp, 66.dp).rotate((index - 1) * 6f))
        }
    }
    if (open) InboxSheet { open = false }
}
