package app.isctcompanion.core

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import app.isctcompanion.R
import app.isctcompanion.design.t

/**
 * Keeps the app running while a browser sign-in returns to Core's loopback
 * listener. Once the app is in the background, the system may freeze it
 * (Xiaomi's power manager does within seconds), and the browser's callback
 * then times out. A short foreground service keeps it awake for the few
 * minutes a sign-in takes; Android ends it after three at most.
 */
class SignInService : Service() {
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, t("common.signingIn"), NotificationManager.IMPORTANCE_LOW),
        )
        val notification = Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(t("common.signingIn"))
            .setOngoing(true)
            .build()
        startForeground(ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE)
        return START_NOT_STICKY
    }

    override fun onTimeout(startId: Int) = stopSelf()

    override fun onBind(intent: Intent?): IBinder? = null

    companion object {
        private const val CHANNEL = "sign-in"
        private const val ID = 7301

        /** Called while the app is in front, as the browser opens. */
        fun start(context: Context) {
            context.startForegroundService(Intent(context, SignInService::class.java))
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, SignInService::class.java))
        }
    }
}
