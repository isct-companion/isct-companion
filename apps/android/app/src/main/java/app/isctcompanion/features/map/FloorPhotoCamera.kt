package app.isctcompanion.features.map

import app.isctcompanion.core.campusMapUploadPhoto
import app.isctcompanion.shared.StillCameraResult
import app.isctcompanion.shared.rememberStillCamera
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import app.isctcompanion.LocalModel
import app.isctcompanion.design.t
import java.io.ByteArrayOutputStream
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** A building's photo uploads during this map visit. */
data class FloorPhotoUploads(val sending: Int = 0, val sent: Int = 0)

/** The system camera for a building's photos: `take` opens it, each photo
 * is sent in the background, and `uploads` tells the card's button how they
 * went. A photo is decoded upright, scaled to at most 2048 px and encoded
 * anew as JPEG, so no EXIF and no location leave the device; Core signs it
 * for the maintainers' inbox. */
class FloorPhotoCamera(
    val available: Boolean,
    val uploads: Map<String, FloorPhotoUploads>,
    val failure: String?,
    val take: (String) -> Unit,
    val dismissFailure: () -> Unit,
)

@Composable
fun rememberFloorPhotoCamera(): FloorPhotoCamera {
    val model = LocalModel.current
    var uploads by remember { mutableStateOf(mapOf<String, FloorPhotoUploads>()) }
    var failure by remember { mutableStateOf<String?>(null) }

    fun update(building: String, change: (FloorPhotoUploads) -> FloorPhotoUploads) {
        uploads = uploads + (building to change(uploads[building] ?: FloorPhotoUploads()))
    }

    val camera = rememberStillCamera<String> { building, result ->
        when (result) {
            is StillCameraResult.Captured -> {
                val jpeg = withContext(Dispatchers.Default) { runCatching { encode(result.file) }.getOrNull() }
                if (jpeg == null) {
                    failure = t("error.internal")
                    return@rememberStillCamera
                }
                update(building) { it.copy(sending = it.sending + 1) }
                val refused = model.campusMapUploadPhoto(building, jpeg)
                update(building) { it.copy(sending = it.sending - 1, sent = if (refused == null) it.sent + 1 else it.sent) }
                failure = refused
            }
            StillCameraResult.Denied -> failure = t("settings.matrixCamera.cameraAccessWasDeniedAllow")
            StillCameraResult.Cancelled -> Unit
        }
    }

    return FloorPhotoCamera(
        available = camera.available,
        uploads = uploads,
        failure = failure,
        take = camera.take,
        dismissFailure = { failure = null },
    )
}

/** The photo upright (ImageDecoder applies its EXIF orientation), at most
 * 2048 px on the long side, as a fresh JPEG with no metadata. */
private fun encode(file: File): ByteArray {
    val bitmap = ImageDecoder.decodeBitmap(ImageDecoder.createSource(file)) { decoder, info, _ ->
        val longest = maxOf(info.size.width, info.size.height)
        if (longest > MAX_SIDE) {
            val scale = MAX_SIDE.toFloat() / longest
            decoder.setTargetSize((info.size.width * scale).toInt(), (info.size.height * scale).toInt())
        }
        decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
    }
    return ByteArrayOutputStream().use { out ->
        bitmap.compress(Bitmap.CompressFormat.JPEG, 80, out)
        bitmap.recycle()
        out.toByteArray()
    }
}

private const val MAX_SIDE = 2048
