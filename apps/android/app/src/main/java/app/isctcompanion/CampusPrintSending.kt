package app.isctcompanion

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import android.util.Log
import app.isctcompanion.design.t

/**
 * Keeps the app awake while a print job is read and sent. Once the print
 * dialog closes, the app is in the background, and HyperOS freezes it
 * within seconds (as it does a browser sign-in, see `SignInService`): the
 * job then waits, half sent, until the next print dialog binds the service
 * and thaws the app. A short foreground service with a quiet "Sending…"
 * notification keeps it running until the job is done; Android ends it after
 * three minutes at most.
 *
 * `CampusPrintService` starts it while the system has that service bound,
 * which allows a foreground service from the background.
 */
class CampusPrintSending : Service() {
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, t("tools.campusPrinter.campusPrinter"), NotificationManager.IMPORTANCE_LOW),
        )
        val notification = Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(t("tools.campusPrinter.campusPrinter"))
            .setContentText(t("campusPrint.window.sending"))
            .setOngoing(true)
            .build()
        startForeground(ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE)
        return START_NOT_STICKY
    }

    override fun onTimeout(startId: Int) = stopSelf()

    override fun onBind(intent: Intent?): IBinder? = null

    companion object {
        private const val CHANNEL = "campus-print"
        private const val ID = 7302
        private const val TAG = "isct-print"
        private var held = 0

        /** One job starts; the first one keeps the app awake. */
        @Synchronized
        fun hold(context: Context) {
            held += 1
            if (held > 1) return
            try {
                context.startForegroundService(Intent(context, CampusPrintSending::class.java))
            } catch (error: Exception) {
                // Not allowed now: the job still runs, only without the guard.
                Log.w(TAG, "Could not keep the app awake for printing", error)
            }
        }

        /** One job ends; the last one lets the app sleep again. */
        @Synchronized
        fun release(context: Context) {
            if (held == 0) return
            held -= 1
            if (held == 0) context.stopService(Intent(context, CampusPrintSending::class.java))
        }
    }
}
