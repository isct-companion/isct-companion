package app.isctcompanion.shared

import android.Manifest
import android.content.pm.PackageManager
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import java.io.File
import kotlinx.coroutines.launch

internal sealed interface StillCameraResult {
    data class Captured(val file: File, val uri: Uri) : StillCameraResult
    data object Cancelled : StillCameraResult
    data object Denied : StillCameraResult
}

internal class StillCamera<T>(
    val available: Boolean,
    val take: (T) -> Unit,
)

private data class Capture<T>(val request: T, val file: File, val uri: Uri)

/** A captured file belongs to the adapter and is valid for the result callback's lifetime. */
@Composable
internal fun <T : Any> rememberStillCamera(
    onResult: suspend (T, StillCameraResult) -> Unit,
): StillCamera<T> {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val available = remember(context) {
        context.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)
    }
    var requesting by remember { mutableStateOf<T?>(null) }
    var pending by remember { mutableStateOf<Capture<T>?>(null) }

    val takePicture = rememberLauncherForActivityResult(ActivityResultContracts.TakePicture()) { taken ->
        val shot = pending ?: return@rememberLauncherForActivityResult
        pending = null
        if (taken) {
            scope.launch {
                onResult(shot.request, StillCameraResult.Captured(shot.file, shot.uri))
            }.invokeOnCompletion { shot.file.delete() }
        } else {
            shot.file.delete()
            scope.launch { onResult(shot.request, StillCameraResult.Cancelled) }
        }
    }

    fun capture(request: T) {
        val directory = File(context.cacheDir, "photos").apply { mkdirs() }
        val file = File.createTempFile("still-", ".jpg", directory)
        try {
            val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
            pending = Capture(request, file, uri)
            takePicture.launch(uri)
        } catch (error: Exception) {
            pending = null
            file.delete()
            throw error
        }
    }

    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { allowed ->
        val request = requesting ?: return@rememberLauncherForActivityResult
        requesting = null
        if (allowed) capture(request)
        else scope.launch { onResult(request, StillCameraResult.Denied) }
    }

    return StillCamera(available) { request ->
        // The manifest declares CAMERA, so the system camera requires this app to hold it.
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
            capture(request)
        } else {
            requesting = request
            permission.launch(Manifest.permission.CAMERA)
        }
    }
}
