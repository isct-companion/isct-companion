package app.isctcompanion.shared

import android.graphics.Bitmap
import android.graphics.pdf.PdfRenderer
import android.media.ThumbnailUtils
import android.os.ParcelFileDescriptor
import android.util.Size
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.isctcompanion.design.Theme
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

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
