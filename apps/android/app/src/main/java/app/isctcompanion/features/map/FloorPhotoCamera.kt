package app.isctcompanion.features.map

import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.FileProvider
import app.isctcompanion.LocalModel
import app.isctcompanion.design.t
import java.io.ByteArrayOutputStream
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
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
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var uploads by remember { mutableStateOf(mapOf<String, FloorPhotoUploads>()) }
    var failure by remember { mutableStateOf<String?>(null) }
    // The building and file of the photo the camera is taking.
    var pending by remember { mutableStateOf<Pair<String, File>?>(null) }
    val available = remember { context.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY) }

    fun update(building: String, change: (FloorPhotoUploads) -> FloorPhotoUploads) {
        uploads = uploads + (building to change(uploads[building] ?: FloorPhotoUploads()))
    }

    val takePicture = rememberLauncherForActivityResult(ActivityResultContracts.TakePicture()) { taken ->
        val (building, shot) = pending ?: return@rememberLauncherForActivityResult
        pending = null
        if (!taken) {
            shot.delete()
            return@rememberLauncherForActivityResult
        }
        scope.launch {
            val jpeg = withContext(Dispatchers.Default) { runCatching { encode(shot) }.getOrNull() }
            shot.delete()
            if (jpeg == null) {
                failure = t("error.internal")
                return@launch
            }
            update(building) { it.copy(sending = it.sending + 1) }
            val refused = model.campusMapUploadPhoto(building, jpeg)
            update(building) { it.copy(sending = it.sending - 1, sent = if (refused == null) it.sent + 1 else it.sent) }
            failure = refused
        }
    }
    // An app that declares the camera permission must hold it to start the
    // system camera.
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { allowed ->
        val shot = pending?.second ?: return@rememberLauncherForActivityResult
        if (allowed) {
            takePicture.launch(FileProvider.getUriForFile(context, "${context.packageName}.files", shot))
        } else {
            pending = null
            failure = t("settings.matrixCamera.cameraAccessWasDeniedAllow")
        }
    }

    return FloorPhotoCamera(
        available = available,
        uploads = uploads,
        failure = failure,
        take = { building ->
            val shot = File(context.cacheDir, "photos").apply { mkdirs() }.resolve("floor-${System.nanoTime()}.jpg")
            pending = building to shot
            permission.launch(android.Manifest.permission.CAMERA)
        },
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
