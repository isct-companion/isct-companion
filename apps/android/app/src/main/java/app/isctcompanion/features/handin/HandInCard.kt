package app.isctcompanion.features.handin

import android.app.Activity
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.draganddrop.dragAndDropTarget
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.PhotoLibrary
import androidx.compose.material.icons.rounded.UploadFile
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draganddrop.DragAndDropEvent
import androidx.compose.ui.draganddrop.DragAndDropTarget
import androidx.compose.ui.draganddrop.mimeTypes
import androidx.compose.ui.draganddrop.toAndroidDragEvent
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import app.isctcompanion.core.AssignmentDetail
import app.isctcompanion.core.HandInAvailability
import app.isctcompanion.core.SharedInbox
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import kotlinx.coroutines.launch

/** Starts a new file selection for this assignment's LMS draft. */
@Composable
fun HandInCard(assignment: AssignmentDetail) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var targeted by remember { mutableStateOf(false) }
    val target = SharedInbox.Target(assignment.assignmentId, assignment.title)
    val documents = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) {
        scope.launch { SharedInbox.receive(context, it, target) }
    }
    val photos = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(32)) {
        scope.launch { SharedInbox.receive(context, it, target) }
    }
    val drop = remember(target) {
        object : DragAndDropTarget {
            override fun onEntered(event: DragAndDropEvent) { targeted = true }
            override fun onExited(event: DragAndDropEvent) { targeted = false }
            override fun onEnded(event: DragAndDropEvent) { targeted = false }
            override fun onDrop(event: DragAndDropEvent): Boolean {
                val drag = event.toAndroidDragEvent()
                val clip = drag.clipData ?: return false
                val uris = (0 until clip.itemCount).mapNotNull { clip.getItemAt(it).uri }
                if (uris.isEmpty()) return false
                val grant = (context as? Activity)?.requestDragAndDropPermissions(drag)
                scope.launch {
                    try { SharedInbox.receive(context, uris, target) }
                    finally { grant?.release() }
                }
                return true
            }
        }
    }
    Column(
        Modifier.fillMaxWidth()
            .clip(RoundedCornerShape(Theme.radiusSurface))
            .background(if (targeted) Theme.colors.accentSoft else Theme.colors.surface)
            .dragAndDropTarget(
                shouldStartDragAndDrop = { event ->
                    assignment.handInAvailability == HandInAvailability.PREPARE_DRAFT &&
                        event.mimeTypes().any { it != "text/plain" }
                },
                target = drop,
            ),
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(Theme.space3),
            modifier = Modifier.fillMaxWidth().clickable { documents.launch(arrayOf("*/*")) }.padding(vertical = Theme.space8),
        ) {
            Icon(Icons.Rounded.UploadFile, null, tint = Theme.colors.accent)
            Text(t("handIn.tapToChoose"), fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
            Text(t("handIn.upTo32Files20"), style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
        }
        TextButton(
            onClick = { photos.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) },
            modifier = Modifier.padding(start = Theme.space4, bottom = Theme.space6),
        ) {
            Icon(Icons.Rounded.PhotoLibrary, null)
            Spacer(Modifier.width(Theme.space3))
            Text(t("handIn.photos"), fontWeight = FontWeight.SemiBold)
        }
    }
}
