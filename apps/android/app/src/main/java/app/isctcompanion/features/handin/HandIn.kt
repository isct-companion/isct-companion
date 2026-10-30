package app.isctcompanion.features.handin

import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import android.app.Activity
import android.graphics.Bitmap
import android.graphics.pdf.PdfRenderer
import android.media.ThumbnailUtils
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.util.Size
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.draganddrop.dragAndDropTarget
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.OpenInNew
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.PhotoLibrary
import androidx.compose.material.icons.rounded.RemoveCircleOutline
import androidx.compose.material.icons.rounded.UploadFile
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
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
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.AssignmentDetail
import app.isctcompanion.core.DeadlineItem
import app.isctcompanion.core.LocalFile
import app.isctcompanion.core.SchoolSite
import app.isctcompanion.core.SharedFile
import app.isctcompanion.core.SharedInbox
import app.isctcompanion.core.WebRouter
import app.isctcompanion.design.ActionButton
import app.isctcompanion.design.DeadlineRow
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import java.io.File
import java.time.Instant
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

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

/** The upload panel: what waits in the inbox, then the quarter's unsubmitted
 * assignments as 今日 groups them. Picking one puts the files in its upload
 * card and opens its page, where 「LMS にアップロード」 is the next step. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun InboxSheet(close: () -> Unit) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    LaunchedEffect(SharedInbox.items.isEmpty()) { if (SharedInbox.items.isEmpty()) close() }
    ModalBottomSheet(onDismissRequest = close, containerColor = Theme.colors.bg) {
        Text(
            t("common.handInTo"), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        )
        AssignmentPicker(
            header = {
                LazyRow(
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    contentPadding = PaddingValues(horizontal = 16.dp, vertical = 4.dp),
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
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
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
            RowDivider(16.dp)
        }
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(6.dp),
            modifier = Modifier.fillMaxWidth().clickable { documents.launch(arrayOf("*/*")) }.padding(vertical = 18.dp),
        ) {
            Icon(Icons.Rounded.UploadFile, null, tint = Theme.colors.accent)
            Text(t("handIn.dropFilesHereOrTap"), fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
            Text(t("handIn.upTo32Files20"), style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
        }
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.fillMaxWidth().padding(start = 8.dp, end = 16.dp, bottom = 12.dp),
        ) {
            TextButton(onClick = { photos.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) }) {
                Icon(Icons.Rounded.PhotoLibrary, null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
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

/** A file's thumbnail (images, videos, the first page of a PDF), or its
 * type's icon. Drawn off the main thread. */
@Composable
fun Thumbnail(file: File, modifier: Modifier = Modifier, corner: Dp = 8.dp) {
    val bitmap by produceState<Bitmap?>(null, file) {
        value = withContext(Dispatchers.IO) { thumbnail(file) }
    }
    Box(
        modifier.clip(RoundedCornerShape(corner)).background(Theme.colors.surface2),
        contentAlignment = Alignment.Center,
    ) {
        bitmap?.let {
            Image(it.asImageBitmap(), null, contentScale = ContentScale.Crop, modifier = Modifier.matchParentSize())
        } ?: Icon(Icons.Rounded.Description, null, tint = Theme.colors.muted)
    }
}

private fun thumbnail(file: File): Bitmap? = runCatching {
    val size = Size(256, 320)
    when (file.extension.lowercase()) {
        "jpg", "jpeg", "png", "heic", "heif", "webp", "gif" -> ThumbnailUtils.createImageThumbnail(file, size, null)
        "mp4", "mov", "m4v" -> ThumbnailUtils.createVideoThumbnail(file, size, null)
        "pdf" -> ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY).use { descriptor ->
            PdfRenderer(descriptor).use { pdf ->
                pdf.openPage(0).use { page ->
                    Bitmap.createBitmap(size.width, size.width * page.height / page.width, Bitmap.Config.ARGB_8888).also {
                        it.eraseColor(android.graphics.Color.WHITE)
                        page.render(it, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
                    }
                }
            }
        }
        else -> null
    }
}.getOrNull()
