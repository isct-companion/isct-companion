package app.isctcompanion.features.map

import android.location.LocationManager
import android.annotation.SuppressLint
import app.isctcompanion.core.CampusMapPoint
import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import androidx.activity.compose.LocalActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.launch

/** The blue dot's permission, asked for only when the person taps the
 * position button. Each tap then reads the position once and hands it on:
 * Core picks the building the person is at. MapLibre draws the dot itself,
 * and there is no background location. */
internal class Position(private val granted: () -> Boolean, private val ask: () -> Unit) {
    var allowed by mutableStateOf(granted())

    /** Android no longer asks: the app's settings page is the way on. */
    var settingsRequired by mutableStateOf(false)

    /** The position the next grant reads, for the tap that asked. */
    var pending: ((CampusMapPoint) -> Unit)? = null

    fun locate(context: Context, located: (CampusMapPoint) -> Unit) {
        // Granted in the system settings since the page opened, perhaps.
        if (granted()) {
            allowed = true
            readOnce(context, located)
        } else {
            pending = located
            ask()
        }
    }

    /** One fix from the system's fused provider (part of AOSP since
     * Android 12), else from GPS where a device ships without it. */
    @SuppressLint("MissingPermission")
    fun readOnce(context: Context, located: (CampusMapPoint) -> Unit) {
        val manager = context.getSystemService(LocationManager::class.java) ?: return
        val provider = if (manager.hasProvider(LocationManager.FUSED_PROVIDER)) LocationManager.FUSED_PROVIDER else LocationManager.GPS_PROVIDER
        runCatching {
            manager.getCurrentLocation(provider, null, context.mainExecutor) { fix ->
                if (fix != null) located(CampusMapPoint(latitude = fix.latitude, longitude = fix.longitude))
            }
        }
    }
}

@Composable
internal fun rememberPosition(): Position {
    val context = LocalContext.current
    val activity = LocalActivity.current
    val permissions = arrayOf(Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION)
    val holder = remember { arrayOfNulls<Position>(1) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
        val position = holder[0] ?: return@rememberLauncherForActivityResult
        val located = position.pending
        position.pending = null
        if (result.values.any { it }) {
            position.allowed = true
            located?.let { position.readOnce(context, it) }
        } else if (activity == null || permissions.none { activity.shouldShowRequestPermissionRationale(it) }) {
            // Refused for good (or restricted): the request showed nothing.
            position.settingsRequired = true
        }
    }
    return remember {
        Position(
            granted = { permissions.any { context.checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED } },
            ask = { launcher.launch(permissions) },
        ).also { holder[0] = it }
    }
}
