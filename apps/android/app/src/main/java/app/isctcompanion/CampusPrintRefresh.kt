package app.isctcompanion

import android.content.ComponentName
import android.content.Context
import android.content.pm.PackageManager
import android.util.Log
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

/** Gets the campus printer back into print dialogs after the system lost
 * track of it.
 *
 * Unlike Windows, where the campus printer is a queue Windows keeps whether
 * the app runs or not, Android lists it only while the system has
 * `CampusPrintService` bound and the service answers. When HyperOS stops
 * the app while some print dialog's printer search is still open, the
 * system keeps that search, does not bind the service again (HyperOS blocks
 * restarting a stopped app), and every print dialog shows only the
 * remembered, unavailable printer until the service is switched off and on
 * in Settings or the phone restarts.
 *
 * Turning the service's component off and on does the same from the app:
 * the system drops its stale record of the service, adds it again and binds
 * it while the app is in the foreground, where HyperOS allows it. */
internal object CampusPrintRefresh {
    /** Whether the system has the print service running in this process;
     * then it is answering and must not be interrupted. */
    @Volatile
    var serviceRunning = false

    /** Set once a refresh ran in this process. */
    @Volatile
    private var refreshed = false

    /** Turns the service back on in case the app stopped in the middle of a
     * refresh, which would leave the campus printer off for good. A no-op
     * when it is already on. */
    fun ensureEnabled(context: Context) {
        try {
            context.packageManager.setComponentEnabledSetting(
                component(context),
                PackageManager.COMPONENT_ENABLED_STATE_DEFAULT,
                PackageManager.DONT_KILL_APP,
            )
        } catch (error: Exception) {
            Log.w(TAG, "Could not turn the print service on", error)
        }
    }

    /** Makes the system add and bind the print service afresh, unless it is
     * running now. With `oncePerProcess`, only the first call in this
     * process does it, so opening the app does not repeat it on every
     * return to it. Returns once the service is on again. */
    suspend fun refresh(context: Context, oncePerProcess: Boolean = false) {
        if (serviceRunning || (oncePerProcess && refreshed)) return
        refreshed = true
        val packages = context.packageManager
        try {
            packages.setComponentEnabledSetting(
                component(context),
                PackageManager.COMPONENT_ENABLED_STATE_DISABLED,
                PackageManager.DONT_KILL_APP,
            )
            // The system learns of a component change from a broadcast it
            // batches for about a second; both changes in one batch would
            // look like none. The gap makes it see the service go and come.
            delay(GAP_MS)
        } catch (error: Exception) {
            Log.w(TAG, "Could not turn the print service off", error)
        } finally {
            withContext(NonCancellable) { ensureEnabled(context) }
        }
        Log.i(TAG, "print service refreshed")
    }

    private fun component(context: Context) = ComponentName(context, CampusPrintService::class.java)

    private const val TAG = "isct-print"
    private const val GAP_MS = 2_500L
}
