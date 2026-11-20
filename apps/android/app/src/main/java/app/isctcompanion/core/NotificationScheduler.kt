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
import android.util.Log
import app.isctcompanion.MainActivity
import app.isctcompanion.R
import app.isctcompanion.design.Format
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Applies Core's reminder plan using AlarmManager and the notification shade. */
class NotificationScheduler(
    private val context: Context,
    private val onPermissionNeeded: (Boolean) -> Unit,
) : NativeReminderPort {
    private var refreshing = emptySet<String>()

    override suspend fun facts(): ReminderPortFacts = withContext(Dispatchers.Main.immediate) {
        val alarms = context.getSystemService(AlarmManager::class.java)
        val exact = alarms?.canScheduleExactAlarms() == true
        val store = store(context)
        val pending = store.getStringSet(SCHEDULED, emptySet()).orEmpty().map { key ->
            val at = store.getString("$AT$key", null)
            val wasExact = store.getBoolean("$EXACT$key", false)
            PendingReminder(
                key = key,
                at = at,
                state = when {
                    pendingIntent(context, key) == null || (wasExact && !exact) -> PendingReminderState.MISSING
                    at == null || wasExact != exact -> PendingReminderState.NEEDS_REFRESH
                    else -> PendingReminderState.PENDING
                },
            )
        }
        refreshing = pending.filter { it.state == PendingReminderState.NEEDS_REFRESH }.map { it.key }.toSet()
        ReminderPortFacts(
            permitted = allowed(context),
            availableSlots = (LIMIT - pending.count { it.state != PendingReminderState.MISSING }).coerceAtLeast(0).toUInt(),
            pending = pending,
        )
    }

    // Alarm broadcasts and these updates run on the main thread. The receiver
    // can remove a fired alarm without racing an update of the stored keys.
    override suspend fun apply(plan: ReminderDeliveryPlan): ReminderReceipt = withContext(Dispatchers.Main.immediate) {
        onPermissionNeeded(plan.permissionNeeded)
        val alarms = context.getSystemService(AlarmManager::class.java)
        val exact = alarms?.canScheduleExactAlarms() == true
        val store = store(context)
        val scheduled = store.getStringSet(SCHEDULED, emptySet()).orEmpty().toMutableSet()
        val edit = store.edit()
        val cancelled = plan.cancel.filter { key ->
            attempt("cancel alarm") {
                pendingIntent(context, key)?.let { intent ->
                    alarms?.cancel(intent)
                    intent.cancel()
                }
                scheduled.remove(key)
                edit.remove("$AT$key").remove("$EXACT$key")
                true
            }
        }
        val failed = plan.deliverNow.filterNot { item ->
            attempt("post reminder") { post(context, item.key, item.title, item.body, item.deepLink) }
        }.map { it.key }
        val registered = plan.register.filter { item ->
            attempt("register alarm") {
                // A previously installed alarm may have fired since facts().
                if (item.key in refreshing && item.key !in scheduled) return@attempt false
                val at = Format.date(item.at) ?: return@attempt false
                if (alarms == null) return@attempt false
                val intent = pendingIntent(context, item.key, item) ?: return@attempt false
                try {
                    if (exact) {
                        alarms.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at.toEpochMilli(), intent)
                    } else {
                        alarms.setWindow(AlarmManager.RTC_WAKEUP, at.toEpochMilli(), WINDOW_MS, intent)
                    }
                } catch (error: Exception) {
                    intent.cancel()
                    throw error
                }
                scheduled.add(item.key)
                edit.putString("$AT${item.key}", item.at).putBoolean("$EXACT${item.key}", exact)
                true
            }
        }.map { it.key }
        if (cancelled.isNotEmpty() || registered.isNotEmpty()) edit.putStringSet(SCHEDULED, scheduled).apply()
        ReminderReceipt(registered = registered, cancelled = cancelled, failedDeliveries = failed)
    }

    companion object {
        private const val LIMIT = 64
        // Channel importance persists, so use a new ID for the banner channel.
        private const val CHANNEL = "reminder-banners"
        private const val OLD_CHANNEL = "reminders"
        private const val SCHEDULED = "scheduled"
        private const val AT = "alarm-at:"
        private const val EXACT = "alarm-exact:"
        private const val WINDOW_MS = 10 * 60 * 1000L
        private const val ASKED = "asked"

        private fun store(context: Context) = context.getSharedPreferences("notifications", Context.MODE_PRIVATE)

        fun allowed(context: Context): Boolean =
            context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

        /** Android records whether it asked, to distinguish refusal from a fresh install. */
        fun denied(context: Context): Boolean = !allowed(context) && store(context).getBoolean(ASKED, false)

        fun asking(context: Context) {
            store(context).edit().putBoolean(ASKED, true).apply()
        }

        /** A reboot or package replacement can discard alarms. Keep the stored
         * keys for Core's ownership recovery, while invalidating their handles. */
        fun invalidateAlarms(context: Context) {
            for (key in store(context).getStringSet(SCHEDULED, emptySet()).orEmpty()) {
                pendingIntent(context, key)?.cancel()
            }
        }

        fun sendTest(context: Context): Boolean {
            if (!allowed(context)) return false
            // Reposting an existing test notification would only update it.
            context.getSystemService(NotificationManager::class.java)?.cancel("test".hashCode())
            return post(context, "test", "IsctCompanion", app.isctcompanion.design.t("settings.notification.thisIsATestNotification"), null)
        }

        private inline fun attempt(action: String, work: () -> Boolean): Boolean = try {
            work()
        } catch (error: Exception) {
            Log.w("isct", action, error)
            false
        }

        private fun post(context: Context, key: String, title: String?, body: String?, deepLink: String?): Boolean {
            if (!allowed(context)) return false
            val manager = context.getSystemService(NotificationManager::class.java) ?: return false
            manager.deleteNotificationChannel(OLD_CHANNEL)
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL, context.getString(R.string.reminders), NotificationManager.IMPORTANCE_HIGH),
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
            return true
        }

        private fun pendingIntent(context: Context, key: String, item: NotificationItem? = null): PendingIntent? {
            val intent = Intent(context, Receiver::class.java).setAction("app.isctcompanion.REMIND.$key")
            val flags = if (item == null) {
                PendingIntent.FLAG_NO_CREATE
            } else {
                intent.putExtra("key", item.key).putExtra("title", item.title).putExtra("body", item.body)
                    .putExtra("deepLink", item.deepLink).putExtra("at", item.at)
                PendingIntent.FLAG_CANCEL_CURRENT
            }
            return PendingIntent.getBroadcast(context, key.hashCode(), intent, flags or PendingIntent.FLAG_IMMUTABLE)
        }
    }

    class Receiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val key = intent.getStringExtra("key") ?: return
            val store = store(context)
            val scheduled = store.getStringSet(SCHEDULED, emptySet()).orEmpty()
            // Ignore a broadcast already queued for an alarm Core has replaced.
            if (key !in scheduled || intent.getStringExtra("at") != store.getString("$AT$key", null)) return
            pendingIntent(context, key)?.cancel()
            store.edit().putStringSet(SCHEDULED, scheduled - key).remove("$AT$key").remove("$EXACT$key").apply()
            attempt("post alarm reminder") {
                post(context, key, intent.getStringExtra("title"), intent.getStringExtra("body"), intent.getStringExtra("deepLink"))
            }
        }
    }
}
