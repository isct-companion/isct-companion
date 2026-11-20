package app.isctcompanion.features.settings

import androidx.compose.material3.TextButton
import android.provider.Settings
import android.content.Intent
import android.Manifest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import app.isctcompanion.LocalModel
import app.isctcompanion.core.NotificationScheduler
import app.isctcompanion.design.Card
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import kotlinx.coroutines.launch
import org.json.JSONObject
import app.isctcompanion.core.ReminderLeadOptions

/** The desktop's reminder choices (`platform/reminders.ts`) and labels
 * (`NotificationSettings.tsx`), so every shell writes the same values to
 * `settings.notifications`. */
internal object ReminderLeads {
    val classChoices = listOf(5, 10, 15, 30)
    val examChoices = listOf(10, 30, 60, 1440, 2880)

    fun label(minutes: Int): String = when {
        minutes == 1440 -> t("settings.notification.oneDayBefore")
        minutes % 1440 == 0 -> t("settings.notification.daysBefore", "days" to minutes / 1440)
        minutes == 60 -> t("settings.notification.oneHourBefore")
        minutes % 60 == 0 -> t("settings.notification.hoursBefore", "hours" to minutes / 60)
        else -> t("settings.notification.minutesBefore", "minutes" to minutes)
    }

    // Core resolves every preference's default (`notification_preferences`)
    // before the settings arrive, so these only read the values.
    fun classLead(settings: JSONObject?): Int = settings?.optInt("classLeadMinutes") ?: 0

    fun exams(settings: JSONObject?): Boolean = settings?.optBoolean("exams") == true

    /** Quizzes count as exams (upcoming exams, exam reminders). */
    fun quizzes(settings: JSONObject?): Boolean = settings?.optBoolean("quizzes") == true

    fun examLead(settings: JSONObject?): Int = settings?.optInt("examLeadMinutes") ?: 0

    fun assignmentOptions(settings: JSONObject?): ReminderLeadOptions? {
        val options = settings?.optJSONObject("assignmentLeadOptions") ?: return null
        val presets = options.getJSONArray("presets")
        val selected = options.getJSONArray("selected")
        return ReminderLeadOptions(
            presets = (0 until presets.length()).map { presets.getInt(it).toUInt() },
            selected = (0 until selected.length()).map { selected.getInt(it).toUInt() },
            minSelected = options.getInt("minSelected").toUInt(),
            maxSelected = options.getInt("maxSelected").toUInt(),
        )
    }

    fun assignmentLeads(settings: JSONObject?): List<Int> =
        assignmentOptions(settings)?.selected.orEmpty().map { it.toInt() }

    fun summary(settings: JSONObject?): String {
        val parts = mutableListOf<String>()
        if (settings?.optBoolean("classes") == true) parts.add(t("settings.main.classesLead", "lead" to label(classLead(settings))))
        if (exams(settings)) parts.add(t("settings.main.examsLead", "lead" to label(examLead(settings))))
        if (settings?.optBoolean("assignments") == true) {
            parts.add(t("settings.main.assignmentsCount", "count" to assignmentLeads(settings).size))
        }
        return if (parts.isEmpty()) t("settings.main.off") else parts.joinToString(" · ")
    }
}

/** Reminder switches and their timings; the system permission is asked on
 * the first switch-on. The switches sync between devices but the permission
 * is this phone's, so the page offers the desktop's test notification. */
@Composable
internal fun NotificationsPage(openLeads: () -> Unit) {
    val model = LocalModel.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var denied by remember { mutableStateOf(NotificationScheduler.denied(context)) }
    var tested by remember { mutableStateOf(false) }
    // The answer can change in the system settings while this page is away,
    // as iOS re-reads it when the scene becomes active.
    LifecycleResumeEffect(Unit) {
        denied = NotificationScheduler.denied(context)
        if (NotificationScheduler.allowed(context) && model.notificationsBlocked) model.resyncNotifications()
        onPauseOrDispose {}
    }
    var pending by remember { mutableStateOf<String?>(null) }
    val settings = model.records.settings.optJSONObject("notifications")
    val turnOn = { key: String ->
        scope.launch {
            model.setNotifications {
                it.put(key, true)
                it.put("localPermission", "granted")
            }
        }
    }
    val sendTest = {
        tested = NotificationScheduler.sendTest(context)
        denied = !tested
        model.resyncNotifications()
    }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        denied = !granted
        val key = pending
        pending = null
        when {
            !granted -> Unit
            key == TEST -> sendTest()
            key != null -> turnOn(key)
        }
    }
    val toggle = { key: String, on: Boolean ->
        if (!on) {
            scope.launch { model.setNotifications { it.put(key, false) } }
        } else if (NotificationScheduler.allowed(context)) {
            turnOn(key)
        } else {
            pending = key
            NotificationScheduler.asking(context)
            launcher.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }
    fun lead(key: String, minutes: Int) = scope.launch { model.setNotifications { it.put(key, minutes) } }
    Page {
        Card {
            SwitchRow(t("settings.main.remindMeBeforeClasses"), settings?.optBoolean("classes") == true) { toggle("classes", it) }
            if (settings?.optBoolean("classes") == true) {
                RowDivider(inset = Theme.rowX)
                PickerRow(t("settings.main.remindMe"), ReminderLeads.classLead(settings), ReminderLeads.classChoices) { lead("classLeadMinutes", it) }
            }
        }
        Card(Modifier.padding(top = Theme.space4)) {
            SwitchRow(t("settings.main.remindMeBeforeExams"), ReminderLeads.exams(settings)) { toggle("exams", it) }
            if (ReminderLeads.exams(settings)) {
                val current = ReminderLeads.examLead(settings)
                RowDivider(inset = Theme.rowX)
                PickerRow(t("settings.main.remindMe"), current, (ReminderLeads.examChoices + current).distinct().sorted()) { lead("examLeadMinutes", it) }
            }
            // Also decides Today's upcoming exams, so it stays while reminders
            // are off and asks for no permission.
            RowDivider(inset = Theme.rowX)
            SwitchRow(
                t("settings.main.countQuizzesAsExams"), ReminderLeads.quizzes(settings),
                detail = t("settings.main.quizzesAppearInUpcomingExams"),
            ) { on -> scope.launch { model.setNotifications { it.put("quizzes", on) } } }
        }
        Card(Modifier.padding(top = Theme.space4)) {
            SwitchRow(t("settings.main.remindMeBeforeAssignmentDeadlines"), settings?.optBoolean("assignments") == true) { toggle("assignments", it) }
            if (settings?.optBoolean("assignments") == true) {
                RowDivider(inset = Theme.rowX)
                NavRow(
                    t("settings.main.remindMe"),
                    ReminderLeads.assignmentLeads(settings).sortedDescending().joinToString("、", transform = ReminderLeads::label),
                    onClick = openLeads,
                )
            }
        }
        Card(Modifier.padding(top = Theme.space4)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.fillMaxWidth().padding(start = Theme.rowX, end = Theme.space4, top = Theme.space2, bottom = Theme.space2),
            ) {
                Text(t("settings.main.testNotification"), color = Theme.colors.ink, modifier = Modifier.weight(1f))
                TextButton(onClick = {
                    if (NotificationScheduler.allowed(context)) {
                        sendTest()
                    } else {
                        pending = TEST
                        NotificationScheduler.asking(context)
                        launcher.launch(Manifest.permission.POST_NOTIFICATIONS)
                    }
                }) { Text(t("settings.notification.send")) }
            }
            if (denied) {
                RowDivider(inset = Theme.rowX)
                NavRow(t("settings.main.openSettings"), null) {
                    context.startActivity(
                        Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                            .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName),
                    )
                }
            }
        }
        if (denied) {
            Footer(t("settings.main.notificationsAreNotAllowedAllow"), Theme.colors.danger)
        } else if (tested) {
            Footer(t("settings.notification.theSystemAcceptedTheTest"))
        }
    }
}

/** `pending` for the test notification rather than a reminder switch. */
private const val TEST = "test"

@JvmName("minutePickerRow")
@Composable
private fun PickerRow(title: String, selected: Int, minutes: List<Int>, choose: (Int) -> Unit) =
    PickerRow(title, selected, minutes.map { it to ReminderLeads.label(it) }, choose)
