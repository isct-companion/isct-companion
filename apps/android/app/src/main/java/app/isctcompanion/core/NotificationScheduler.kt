package app.isctcompanion.core

import android.Manifest
import android.app.AlarmManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.content.pm.PackageManager
import android.util.Log
import app.isctcompanion.MainActivity
import app.isctcompanion.R
import app.isctcompanion.design.Format
import java.time.Instant
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Applies Core's reminder plan using AlarmManager and the notification shade. */
class NotificationScheduler(
    private val context: Context,
    private val onPermissionNeeded: (Boolean) -> Unit,
) : NativeReminderPort {
    private var observed = emptyMap<String, PendingReminder>()

    override suspend fun facts(): ReminderPortFacts = withContext(Dispatchers.Main.immediate) {
        val alarms = context.getSystemService(AlarmManager::class.java)
        val exact = alarms?.canScheduleExactAlarms() == true
        val store = store(context)
        val pending = readAlarms(store).map { (key, alarm) ->
            PendingReminder(
                key = key,
                at = alarm.at,
                title = alarm.title,
                body = alarm.body,
                deepLink = alarm.link,
                state = when {
                    alarm.token == null || pendingIntent(context, key, alarm.token) == null ||
                        (alarm.exact && !exact) -> PendingReminderState.MISSING
                    alarm.exact != exact -> PendingReminderState.NEEDS_REFRESH
                    else -> PendingReminderState.PENDING
                },
            )
        }
        observed = pending.associateBy { it.key }
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
        val existing = readAlarms(store)
        val scheduled = existing.toMutableMap()
        val cancelled = plan.cancel.toSet()
        for (key in cancelled) scheduled.remove(key)

        val updated = plan.updatePending.mapNotNull { item ->
            val previous = scheduled[item.key]
            // A broadcast can finish between facts() and apply().
            if (previous?.token == null) return@mapNotNull item.key
            if (previous.at != observed[item.key]?.at) return@mapNotNull null
            scheduled[item.key] = previous.copy(title = item.title, body = item.body, link = item.deepLink)
            item.key
        }
        val registrations = plan.register.mapNotNull { item ->
            val at = Format.date(item.at) ?: return@mapNotNull null
            val now = Instant.now()
            if (alarms == null || at <= now) return@mapNotNull null
            val previous = observed[item.key]
            val previousAt = Format.date(previous?.at)
            if (previous != null && item.key !in existing) return@mapNotNull null
            if (previousAt != null && previousAt <= now) return@mapNotNull null
            val alarm = StoredAlarm(item.at, exact, UUID.randomUUID().toString(), item.title, item.body, item.deepLink)
            scheduled[item.key] = alarm
            item.key to alarm
        }

        val changed = cancelled + updated + registrations.map { it.first }
        // A receiver can start a fresh process as soon as AlarmManager has
        // accepted a request. Its token and content must already be on disk.
        val persisted = changed.isEmpty() || persistAlarms(store, scheduled, changed)
        if (!persisted) {
            persistAlarms(store, existing, changed)
            Log.w("isct", "Could not persist reminder alarms")
        }
        val registered = mutableListOf<String>()
        if (persisted) {
            for (key in cancelled) existing[key]?.token?.let { revokeAlarm(context, alarms, key, it) }
            val rejected = mutableSetOf<String>()
            for ((key, alarm) in registrations) {
                val accepted = attempt("register alarm") {
                    val at = Format.date(alarm.at) ?: return@attempt false
                    val token = alarm.token ?: return@attempt false
                    if (alarms == null || at <= Instant.now()) return@attempt false
                    val previousAt = Format.date(observed[key]?.at)
                    if (previousAt != null && previousAt <= Instant.now()) return@attempt false
                    val intent = pendingIntent(context, key, token, install = true) ?: return@attempt false
                    try {
                        if (alarm.exact) {
                            alarms.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at.toEpochMilli(), intent)
                        } else {
                            alarms.setWindow(AlarmManager.RTC_WAKEUP, at.toEpochMilli(), WINDOW_MS, intent)
                        }
                    } catch (error: Exception) {
                        intent.cancel()
                        throw error
                    }
                    true
                }
                val previous = existing[key]
                if (accepted) {
                    registered.add(key)
                    // A generation has its own handle. Keep the former alarm
                    // available until AlarmManager accepts its replacement.
                    previous?.token?.let { revokeAlarm(context, alarms, key, it) }
                } else {
                    rejected.add(key)
                    if (previous == null) scheduled.remove(key) else scheduled[key] = previous
                }
            }
            if (rejected.isNotEmpty() && !persistAlarms(store, scheduled, rejected)) {
                Log.w("isct", "Could not restore rejected reminder alarms")
            }
        }
        val failed = plan.deliverNow.filterNot { item ->
            attempt("post reminder") { post(context, item.key, item.title, item.body, item.deepLink) }
        }.map { it.key }
        ReminderReceipt(
            registered = registered,
            updatedPending = if (persisted) updated else emptyList(),
            cancelled = if (persisted) cancelled.toList() else emptyList(),
            failedDeliveries = failed,
        )
    }

    private data class StoredAlarm(
        val at: String?,
        val exact: Boolean,
        val token: String?,
        val title: String?,
        val body: String?,
        val link: String?,
    )

    companion object {
        private const val LIMIT = 64
        // Channel importance persists, so use a new ID for the banner channel.
        private const val CHANNEL = "reminder-banners"
        private const val OLD_CHANNEL = "reminders"
        private const val SCHEDULED = "scheduled"
        private const val AT = "alarm-at:"
        private const val EXACT = "alarm-exact:"
        private const val TITLE = "alarm-title:"
        private const val BODY = "alarm-body:"
        private const val LINK = "alarm-link:"
        private const val TOKEN = "alarm-token:"
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
            val store = store(context)
            val alarms = context.getSystemService(AlarmManager::class.java)
            val scheduled = readAlarms(store)
            for ((key, alarm) in scheduled) {
                alarm.token?.let { revokeAlarm(context, alarms, key, it) }
            }
            if (scheduled.isNotEmpty()) {
                persistAlarms(store, scheduled.mapValues { (_, alarm) -> alarm.copy(token = null) }, scheduled.keys)
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

        private fun readAlarms(store: SharedPreferences): Map<String, StoredAlarm> =
            store.getStringSet(SCHEDULED, emptySet()).orEmpty().associateWith { key ->
                StoredAlarm(
                    store.getString("$AT$key", null),
                    store.getBoolean("$EXACT$key", false),
                    store.getString("$TOKEN$key", null),
                    store.getString("$TITLE$key", null),
                    store.getString("$BODY$key", null),
                    store.getString("$LINK$key", null),
                )
            }

        private fun persistAlarms(
            store: SharedPreferences,
            scheduled: Map<String, StoredAlarm>,
            changed: Set<String>,
        ): Boolean {
            val edit = store.edit().putStringSet(SCHEDULED, scheduled.keys.toSet())
            for (key in changed) {
                val alarm = scheduled[key]
                edit.putString("$AT$key", alarm?.at)
                    .putString("$TITLE$key", alarm?.title)
                    .putString("$BODY$key", alarm?.body)
                    .putString("$LINK$key", alarm?.link)
                    .putString("$TOKEN$key", alarm?.token)
                if (alarm == null) edit.remove("$EXACT$key") else edit.putBoolean("$EXACT$key", alarm.exact)
            }
            return edit.commit()
        }

        private fun pendingIntent(context: Context, key: String, token: String, install: Boolean = false): PendingIntent? {
            val intent = Intent(context, Receiver::class.java)
                .setAction("app.isctcompanion.REMIND.$key.$token")
                .putExtra("key", key)
                .putExtra("token", token)
            val flags = if (install) 0 else PendingIntent.FLAG_NO_CREATE
            return PendingIntent.getBroadcast(context, key.hashCode(), intent, flags or PendingIntent.FLAG_IMMUTABLE)
        }

        private fun revokeAlarm(context: Context, alarms: AlarmManager?, key: String, token: String) {
            attempt("cancel alarm") {
                pendingIntent(context, key, token)?.let { intent ->
                    alarms?.cancel(intent)
                    intent.cancel()
                }
                true
            }
        }
    }

    class Receiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val key = intent.getStringExtra("key") ?: return
            val store = store(context)
            val token = intent.getStringExtra("token") ?: return
            val scheduled = readAlarms(store)
            val current = scheduled[key] ?: return
            // Replacing a request can leave an old broadcast queued for the same instant.
            if (token != current.token) return
            pendingIntent(context, key, token)?.cancel()
            if (!persistAlarms(store, scheduled - key, setOf(key))) {
                Log.w("isct", "Could not remove delivered reminder alarm")
            }
            attempt("post alarm reminder") { post(context, key, current.title, current.body, current.link) }
        }
    }
}
