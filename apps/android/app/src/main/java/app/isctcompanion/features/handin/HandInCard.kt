package app.isctcompanion.features.handin

import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import android.app.Activity
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.draganddrop.dragAndDropTarget
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.OpenInNew
import androidx.compose.material.icons.rounded.PhotoLibrary
import androidx.compose.material.icons.rounded.RemoveCircleOutline
import androidx.compose.material.icons.rounded.UploadFile
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
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
import androidx.compose.ui.draganddrop.DragAndDropEvent
import androidx.compose.ui.draganddrop.DragAndDropTarget
import androidx.compose.ui.draganddrop.mimeTypes
import androidx.compose.ui.draganddrop.toAndroidDragEvent
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.AssignmentDetail
import app.isctcompanion.core.LocalFile
import app.isctcompanion.core.SchoolSite
import app.isctcompanion.core.SharedFile
import app.isctcompanion.core.SharedInbox
import app.isctcompanion.core.WebRouter
import app.isctcompanion.design.ActionButton
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import java.io.File
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import app.isctcompanion.core.addSubmitFiles
import app.isctcompanion.core.handIn
import app.isctcompanion.core.removeSubmitFile
import app.isctcompanion.core.submitFiles
import app.isctcompanion.shared.Thumbnail

/**
 * The assignment page's one place to hand in: a card to tap to choose files
 * or photos, or (on a tablet) to drop files from another app on. What it
 * receives joins the assignment's `submit/`, listed in the card; Upload fills
 * the LMS draft area and shows that page, where the person reviews and saves.
 */
@Composable
fun HandInCard(assignment: AssignmentDetail) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    var files by remember { mutableStateOf<List<LocalFile>>(emptyList()) }
    var targeted by remember { mutableStateOf(false) }
    var working by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val id = assignment.assignmentId

    fun add(uris: List<Uri>) {
        if (uris.isEmpty()) return
        scope.launch {
            try {
                val shared = withContext(Dispatchers.IO) { copyOut(context, uris) }
                files = model.addSubmitFiles(id, shared.map { SharedFile(it.path, it.name) })
                withContext(Dispatchers.IO) { shared.forEach { it.parentFile?.deleteRecursively() } }
            } catch (failure: Exception) {
                error = AppModel.describe(failure)
            }
        }
    }
    val documents = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { add(it) }
    val photos = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(32)) { add(it) }
    LaunchedEffect(id, SharedInbox.items.size) { files = model.submitFiles(id) }
    val drop = remember(id) {
        object : DragAndDropTarget {
            override fun onEntered(event: DragAndDropEvent) { targeted = true }
            override fun onExited(event: DragAndDropEvent) { targeted = false }
            override fun onEnded(event: DragAndDropEvent) { targeted = false }
            override fun onDrop(event: DragAndDropEvent): Boolean {
                val drag = event.toAndroidDragEvent()
                // Files from another app arrive as content URIs readable only
                // while this grant lasts.
                (context as? Activity)?.requestDragAndDropPermissions(drag)
                val clip = drag.clipData ?: return false
                add((0 until clip.itemCount).mapNotNull { clip.getItemAt(it).uri })
                return true
            }
        }
    }
    val shape = RoundedCornerShape(Theme.radiusSurface)
    Column(
        Modifier
            .fillMaxWidth()
            .clip(shape)
            .background(if (targeted) Theme.colors.accentSoft else Theme.colors.surface)
            .border(if (targeted) 2.dp else 1.dp, if (targeted) Theme.colors.accent else Theme.colors.line, shape)
            .dragAndDropTarget(
                shouldStartDragAndDrop = { event -> event.mimeTypes().any { it != "text/plain" } },
                target = drop,
            ),
    ) {
        files.forEach { file ->
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(Theme.rowGap),
                modifier = Modifier.fillMaxWidth().padding(horizontal = Theme.rowX, vertical = Theme.space4),
            ) {
                Thumbnail(File(file.path), Modifier.size(32.dp, 40.dp))
                Text(file.name, color = Theme.colors.ink, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                IconButton(onClick = {
                    scope.launch {
                        runCatching { model.removeSubmitFile(id, file.name) }
                            .onSuccess { files = it }
                            .onFailure { error = AppModel.describe(it) }
                    }
                }) { Icon(Icons.Rounded.RemoveCircleOutline, t("common.delete"), tint = Theme.colors.danger) }
            }
            RowDivider(Theme.rowX)
        }
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(Theme.space3),
            modifier = Modifier.fillMaxWidth().clickable { documents.launch(arrayOf("*/*")) }.padding(vertical = 18.dp),
        ) {
            Icon(Icons.Rounded.UploadFile, null, tint = Theme.colors.accent)
            Text(t("handIn.dropFilesHereOrTap"), fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
            Text(t("handIn.upTo32Files20"), style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
        }
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.fillMaxWidth().padding(start = Theme.space4, end = Theme.space8, bottom = Theme.space6),
        ) {
            TextButton(onClick = { photos.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) }) {
                Icon(Icons.Rounded.PhotoLibrary, null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(Theme.space3))
                Text(t("handIn.photos"), fontWeight = FontWeight.SemiBold)
            }
            Spacer(Modifier.weight(1f))
            if (files.isNotEmpty()) {
                ActionButton(
                    t("handIn.uploadToLMS"),
                    {
                        scope.launch {
                            working = true
                            try {
                                val url = model.handIn(id)
                                haptics.performHapticFeedback(HapticFeedbackType.Confirm)
                                navigator.openPage(WebRouter.PushedPage(SchoolSite.SCIENCE_LMS, url, assignment.title))
                            } catch (failure: Exception) {
                                error = AppModel.describe(failure)
                            } finally {
                                working = false
                            }
                        }
                    },
                    icon = Icons.AutoMirrored.Rounded.OpenInNew,
                    enabled = !working,
                )
            }
        }
    }
    error?.let { message ->
        AlertDialog(
            onDismissRequest = { error = null },
            confirmButton = { TextButton(onClick = { error = null }) { Text("OK") } },
            text = { Text(message) },
        )
    }
}

/** Copies picked or dropped content into the cache, one folder per file,
 * under the sender's name; Core copies it on into `submit/`. */
private fun copyOut(context: android.content.Context, uris: List<Uri>): List<File> =
    uris.mapNotNull { uri ->
        runCatching {
            val dir = File(context.cacheDir, "hand-in/${UUID.randomUUID()}").apply { mkdirs() }
            val target = File(dir, SharedInbox.displayName(context, uri))
            context.contentResolver.openInputStream(uri)?.use { input -> target.outputStream().use { input.copyTo(it) } }
                ?: return@mapNotNull null
            target
        }.getOrNull()
    }
