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
import android.net.Uri
import android.util.Log
import app.isctcompanion.BackgroundWake
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
        val stored = readRecords(store)
        val incomplete = recordKeys(store) - stored.keys
        if (incomplete.isNotEmpty() && !persistRecords(store, stored, incomplete)) {
            Log.w("isct", "Could not remove incomplete reminder alarms")
        }
        val pending = stored.keys.map { key ->
            val requests = stored.forKey(key).sortedBy { Format.date(it.at) }
            val live = requests.firstOrNull { alarm ->
                (!alarm.exact || exact) && pendingIntent(context, key, alarm.token) != null
            }
            val alarm = live ?: requests.first()
            PendingReminder(
                key = key,
                at = alarm.at,
                title = alarm.title,
                body = alarm.body,
                deepLink = alarm.link,
                state = when {
                    live == null -> PendingReminderState.MISSING
                    alarm.exact != exact || key in stored.installing -> PendingReminderState.NEEDS_REFRESH
                    else -> PendingReminderState.PENDING
                },
            )
        }
        observed = pending.associateBy { it.key }
        ReminderPortFacts(
            permitted = allowed(context),
            availableSlots = (LIMIT - pending.count { it.state != PendingReminderState.MISSING }).coerceAtLeast(0).toUInt(),
            canUpdateElapsedContent = true,
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
        val existing = readRecords(store)
        val installed = existing.installed.toMutableMap()
        val installing = existing.installing.toMutableMap()
        val cancelled = plan.cancel.toSet()
        for (key in cancelled) {
            installed.remove(key)
            installing.remove(key)
        }

        val updated = plan.updatePending.map { item ->
            for (records in listOf(installed, installing)) {
                val alarm = records[item.key]
                if (alarm != null && alarm.at == observed[item.key]?.at) {
                    records[item.key] = alarm.withContent(item)
                }
            }
            item.key
        }
        val refreshed = mutableSetOf<String>()
        val registrations = plan.register.mapNotNull { item ->
            val at = Format.date(item.at) ?: return@mapNotNull null
            val now = Instant.now()
            if (alarms == null || at <= now) return@mapNotNull null
            val previous = observed[item.key]
            val previousAt = Format.date(previous?.at)
            if (previous != null && item.key !in existing.keys) return@mapNotNull null
            if (previousAt != null && previousAt <= now) return@mapNotNull null
            val active = installed[item.key]
            if (active != null && previousAt == at && previous?.state == PendingReminderState.PENDING &&
                active.exact == exact && pendingIntent(context, item.key, active.token) != null
            ) {
                installed[item.key] = active.withContent(item)
                refreshed.add(item.key)
                return@mapNotNull null
            }
            val unfinished = installing[item.key]
            val alarm = if (unfinished != null && Format.date(unfinished.at) == at && unfinished.exact == exact) {
                // The handle alone does not show whether AlarmManager accepted it.
                // Scheduling the same PendingIntent again preserves its ownership.
                unfinished.withContent(item)
            } else {
                unfinished?.let { installed.putIfAbsent(item.key, it) }
                StoredAlarm(item.at, exact, UUID.randomUUID().toString(), item.title, item.body, item.deepLink)
            }
            installing[item.key] = alarm
            item.key to alarm
        }

        val changed = cancelled + updated + refreshed + registrations.map { it.first }
        // The original alarm and the candidate both have a durable payload
        // before the candidate can reach AlarmManager.
        val prepared = AlarmRecords(installed.toMap(), installing.toMap())
        val persisted = changed.isEmpty() || persistRecords(store, prepared, changed)
        if (!persisted) {
            persistRecords(store, existing, changed)
            Log.w("isct", "Could not persist reminder alarms")
        }
        val registered = mutableListOf<String>()
        if (persisted) {
            registered.addAll(refreshed)
            for (key in cancelled) {
                existing.forKey(key).forEach { revokeAlarm(context, alarms, key, it.token) }
            }
            val accepted = mutableMapOf<String, String>()
            for ((key, alarm) in registrations) {
                if (installAlarm(context, alarms, key, alarm)) {
                    accepted[key] = alarm.token
                    installed[key] = alarm
                    installing.remove(key)
                } else {
                    existing.installed[key].let { if (it == null) installed.remove(key) else installed[key] = it }
                    existing.installing[key].let { if (it == null) installing.remove(key) else installing[key] = it }
                }
            }
            val finished = registrations.map { it.first }.toSet()
            if (finished.isNotEmpty() && !persistRecords(store, AlarmRecords(installed, installing), finished)) {
                persistRecords(store, prepared, finished)
                // AlarmManager may already have accepted a request. A lost
                // receipt preserves Core's provisional ownership.
                error("Could not finalize reminder alarm ownership")
            }
            for ((key, token) in accepted) {
                existing.forKey(key).filter { it.token != token }
                    .forEach { revokeAlarm(context, alarms, key, it.token) }
                registered.add(key)
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

    private fun installAlarm(context: Context, manager: AlarmManager?, key: String, alarm: StoredAlarm): Boolean =
        attempt("register alarm") {
            val at = Format.date(alarm.at) ?: return@attempt false
            val now = Instant.now()
            val previousAt = Format.date(observed[key]?.at)
            if (manager == null || at <= now || (previousAt != null && previousAt <= now)) return@attempt false
            val existing = pendingIntent(context, key, alarm.token)
            val intent = existing ?: pendingIntent(context, key, alarm.token, install = true) ?: return@attempt false
            try {
                if (alarm.exact) {
                    manager.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at.toEpochMilli(), intent)
                } else {
                    manager.setWindow(AlarmManager.RTC_WAKEUP, at.toEpochMilli(), WINDOW_MS, intent)
                }
            } catch (error: Exception) {
                if (existing == null) intent.cancel()
                throw error
            }
            true
        }

    private data class AlarmRecords(
        val installed: Map<String, StoredAlarm> = emptyMap(),
        val installing: Map<String, StoredAlarm> = emptyMap(),
    ) {
        val keys: Set<String> = installed.keys + installing.keys

        fun forKey(key: String): List<StoredAlarm> = listOfNotNull(installed[key], installing[key])
    }

    private data class StoredAlarm(
        val at: String,
        val exact: Boolean,
        val token: String,
        val title: String,
        val body: String,
        val link: String,
    ) {
        fun withContent(item: NotificationItem): StoredAlarm =
            copy(title = item.title, body = item.body, link = item.deepLink)
    }

    companion object {
        private const val LIMIT = 64
        // Channel importance persists, so use a new ID for the banner channel.
        private const val CHANNEL = "reminder-banners"
        private const val OLD_CHANNEL = "reminders"
        private const val SCHEDULED = "scheduled"
        private const val INSTALLING = "installing"
        private const val NEXT = "installing:"
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

        /** Android can discard its alarms after a reboot or package
         * replacement. Core keeps the durable delivery decision. */
        fun invalidateAlarms(context: Context) {
            val store = store(context)
            val alarms = context.getSystemService(AlarmManager::class.java)
            val keys = recordKeys(store)
            val records = readRecords(store)
            for (key in keys) records.forKey(key).forEach { revokeAlarm(context, alarms, key, it.token) }
            if (keys.isNotEmpty() && !persistRecords(store, AlarmRecords(), keys)) {
                Log.w("isct", "Could not clear discarded reminder alarms")
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
            // Each reminder gets its own request code: with one shared code,
            // FLAG_UPDATE_CURRENT would point every posted reminder at the
            // last one's destination.
            val open = PendingIntent.getActivity(
                context, key.hashCode(),
                Intent(context, MainActivity::class.java).apply {
                    val link = openLink(deepLink)
                    if (link != null) {
                        action = Intent.ACTION_VIEW
                        data = link
                    }
                },
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

        /** Core's reminder links (`/assignments/<id>`, `/today`) as the
         * widget link MainActivity already routes through Core's
         * `widgetRoute`, so a tapped reminder lands where a tapped widget row
         * would. `/today` needs none: the app opens on Today. */
        private fun openLink(deepLink: String?): Uri? {
            val id = deepLink?.removePrefix("/assignments/")?.takeIf { it != deepLink && it.isNotEmpty() }
                ?: return null
            return Uri.Builder().scheme("isctcompanion").authority("widget").path("/assignment")
                .appendQueryParameter("id", id).build()
        }

        private fun recordKeys(store: SharedPreferences): Set<String> =
            store.getStringSet(SCHEDULED, emptySet()).orEmpty() + store.getStringSet(INSTALLING, emptySet()).orEmpty()

        private fun readRecords(store: SharedPreferences): AlarmRecords =
            AlarmRecords(readAlarms(store, SCHEDULED, ""), readAlarms(store, INSTALLING, NEXT))

        private fun readAlarms(store: SharedPreferences, set: String, prefix: String): Map<String, StoredAlarm> =
            store.getStringSet(set, emptySet()).orEmpty().mapNotNull { key ->
                val name = prefix + key
                val at = store.getString("$AT$name", null) ?: return@mapNotNull null
                val token = store.getString("$TOKEN$name", null) ?: return@mapNotNull null
                val title = store.getString("$TITLE$name", null) ?: return@mapNotNull null
                val body = store.getString("$BODY$name", null) ?: return@mapNotNull null
                val link = store.getString("$LINK$name", null) ?: return@mapNotNull null
                key to StoredAlarm(at, store.getBoolean("$EXACT$name", false), token, title, body, link)
            }.toMap()

        private fun persistRecords(
            store: SharedPreferences,
            records: AlarmRecords,
            changed: Set<String>,
        ): Boolean {
            val edit = store.edit()
                .putStringSet(SCHEDULED, records.installed.keys.toSet())
                .putStringSet(INSTALLING, records.installing.keys.toSet())
            for (key in changed) {
                writeAlarm(edit, key, records.installed[key])
                writeAlarm(edit, NEXT + key, records.installing[key])
            }
            return edit.commit()
        }

        private fun writeAlarm(edit: SharedPreferences.Editor, name: String, alarm: StoredAlarm?) {
            edit.putString("$AT$name", alarm?.at)
                .putString("$TITLE$name", alarm?.title)
                .putString("$BODY$name", alarm?.body)
                .putString("$LINK$name", alarm?.link)
                .putString("$TOKEN$name", alarm?.token)
            if (alarm == null) edit.remove("$EXACT$name") else edit.putBoolean("$EXACT$name", alarm.exact)
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
            val records = readRecords(store)
            val requests = records.forKey(key)
            // Replacing a request can leave an old broadcast queued. Only a
            // token still holding the business reminder can produce a banner.
            val delivered = requests.firstOrNull { it.token == token } ?: return
            if (!persistRecords(store, AlarmRecords(records.installed - key, records.installing - key), setOf(key))) {
                Log.w("isct", "Could not remove delivered reminder alarm")
            }
            val alarms = context.getSystemService(AlarmManager::class.java)
            requests.forEach { revokeAlarm(context, alarms, key, it.token) }
            attempt("post alarm reminder") { post(context, key, delivered.title, delivered.body, delivered.link) }
            if (records.keys.size == LIMIT) BackgroundWake.soon(context)
        }
    }
}
