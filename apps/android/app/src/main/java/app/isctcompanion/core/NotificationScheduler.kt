package app.isctcompanion.core

import android.Manifest
import android.app.AlarmManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import app.isctcompanion.MainActivity
import app.isctcompanion.R
import app.isctcompanion.design.Format
import java.time.Instant

/** Registers Core's notification plan with the system. Core decides what is
 * due and when (class lead time, deadlines, settings); the shell schedules
 * alarms for it, at most `LIMIT` pending as on iOS. Alarms are exact when the
 * person allowed exact alarms in system settings. */
object NotificationScheduler {
    private const val LIMIT = 64
    private const val HORIZON_DAYS = 7L
    private const val CHANNEL = "reminders"
    private const val SCHEDULED = "scheduled"
    private const val WINDOW_MS = 10 * 60 * 1000L

    fun allowed(context: Context): Boolean =
        context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

    /** Schedules the plan. True when Core has reminders this device may not
     * post: the switches sync from the other devices, so this one may never
     * have asked for the permission, and the screen asks then. */
    suspend fun sync(context: Context, session: Session): Boolean {
        val now = Instant.now()
        val horizon = now.plusSeconds(HORIZON_DAYS * 24 * 3600)
        val query = WorkspaceQuery.NotificationPlan(Format.timestamp(now), Format.timestamp(horizon), null)
        val result = runCatching { session.scheduleQuery(listOf(query)).value() }.getOrNull()
        val plan = ((result as? OperationResult.Schedule)?.snapshot?.values?.firstOrNull()
            as? WorkspaceQueryResult.NotificationPlan)?.v1 ?: return false
        if (!allowed(context)) return plan.items.isNotEmpty()
        val alarms = context.getSystemService(AlarmManager::class.java) ?: return false
        val store = context.getSharedPreferences("notifications", Context.MODE_PRIVATE)
        for (key in store.getStringSet(SCHEDULED, emptySet()).orEmpty()) {
            alarms.cancel(pending(context, key, null))
        }
        val scheduled = mutableSetOf<String>()
        for (item in plan.items.take(LIMIT)) {
            val at = Format.date(item.at) ?: continue
            if (!at.isAfter(now)) continue
            val intent = pending(context, item.key, item)
            if (alarms.canScheduleExactAlarms()) {
                alarms.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at.toEpochMilli(), intent)
            } else {
                // Without the exact-alarm grant, the shortest window Android
                // honours: a class reminder may arrive up to that much late.
                alarms.setWindow(AlarmManager.RTC_WAKEUP, at.toEpochMilli(), WINDOW_MS, intent)
            }
            scheduled.add(item.key)
        }
        store.edit().putStringSet(SCHEDULED, scheduled).apply()
        return false
    }

    /** The desktop's Test notification, posted now. False without permission. */
    fun sendTest(context: Context): Boolean {
        if (!allowed(context)) return false
        post(context, "test", "IsctCompanion", app.isctcompanion.design.t("This is a test notification."), null)
        return true
    }

    private fun post(context: Context, key: String, title: String?, body: String?, deepLink: String?) {
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, context.getString(R.string.reminders), NotificationManager.IMPORTANCE_DEFAULT),
        )
        val open = PendingIntent.getActivity(
            context, 0,
            Intent(context, MainActivity::class.java).putExtra("deepLink", deepLink),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val notification = android.app.Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(body)
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        manager.notify(key.hashCode(), notification)
    }

    private fun pending(context: Context, key: String, item: NotificationItem?): PendingIntent {
        val intent = Intent(context, Receiver::class.java).setAction("app.isctcompanion.REMIND.$key")
        if (item != null) {
            intent.putExtra("key", item.key).putExtra("title", item.title).putExtra("body", item.body)
                .putExtra("deepLink", item.deepLink)
        }
        return PendingIntent.getBroadcast(
            context, key.hashCode(), intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
    }

    /** Posts a reminder when its alarm fires. */
    class Receiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (!allowed(context)) return
            post(
                context, intent.getStringExtra("key").orEmpty(), intent.getStringExtra("title"),
                intent.getStringExtra("body"), intent.getStringExtra("deepLink"),
            )
        }
    }
}
