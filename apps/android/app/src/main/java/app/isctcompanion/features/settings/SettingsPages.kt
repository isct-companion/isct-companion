package app.isctcompanion.features.settings

import androidx.compose.material3.TextButton
import android.provider.Settings
import android.content.Intent
import androidx.compose.foundation.background
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import android.Manifest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.UnfoldMore
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.FinderDepartment
import app.isctcompanion.core.NotificationScheduler
import app.isctcompanion.core.StudyEnrollment
import app.isctcompanion.core.StudyEntryKind
import app.isctcompanion.core.StudyStage
import app.isctcompanion.design.Card
import app.isctcompanion.design.Format
import app.isctcompanion.design.Localizer
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.SyncIndicatorView
import app.isctcompanion.design.SyncSections
import app.isctcompanion.design.Theme
import app.isctcompanion.design.indicator
import app.isctcompanion.design.t
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject

/** The desktop's reminder choices (`platform/reminders.ts`) and labels
 * (`NotificationSettings.tsx`), so every shell writes the same values to
 * `settings.notifications`. */
internal object ReminderLeads {
    val classChoices = listOf(5, 10, 15, 30)
    const val DEFAULT_CLASS = 10
    val examChoices = listOf(10, 30, 60, 1440, 2880)
    val assignmentChoices = listOf(15, 30, 60, 180, 360, 720, 1440, 2880, 4320, 10080)
    /** Core's `DEFAULT_ASSIGNMENT_LEADS`, used while the key is absent. */
    val defaultAssignment = listOf(1440, 60)
    /** Core accepts one to eight distinct times. */
    const val MAX_ASSIGNMENT = 8

    fun label(minutes: Int): String = when {
        minutes == 1440 -> t("1 day before")
        minutes % 1440 == 0 -> t("{{days}} days before", "days" to minutes / 1440)
        minutes == 60 -> t("1 hour before")
        minutes % 60 == 0 -> t("{{hours}} hours before", "hours" to minutes / 60)
        else -> t("{{minutes}} minutes before", "minutes" to minutes)
    }

    fun classLead(settings: JSONObject?): Int = settings?.optInt("classLeadMinutes", DEFAULT_CLASS) ?: DEFAULT_CLASS

    /** Exams follow the class switch and lead until set on their own (Core's
     * `notification_plan`). */
    fun exams(settings: JSONObject?): Boolean =
        if (settings?.has("exams") == true) settings.optBoolean("exams") else settings?.optBoolean("classes") == true

    fun examLead(settings: JSONObject?): Int =
        if (settings?.has("examLeadMinutes") == true) settings.optInt("examLeadMinutes") else classLead(settings)

    fun assignmentLeads(settings: JSONObject?): List<Int> {
        val stored = settings?.optJSONArray("assignmentLeadMinutes")
        val leads = (0 until (stored?.length() ?: 0)).map { stored!!.optInt(it) }
        return leads.ifEmpty { defaultAssignment }
    }

    fun summary(settings: JSONObject?): String {
        val parts = mutableListOf<String>()
        if (settings?.optBoolean("classes") == true) parts.add(t("Classes {{lead}}", "lead" to label(classLead(settings))))
        if (exams(settings)) parts.add(t("Exams {{lead}}", "lead" to label(examLead(settings))))
        if (settings?.optBoolean("assignments") == true) {
            parts.add(t("Assignments ×{{count}}", "count" to assignmentLeads(settings).size))
        }
        return if (parts.isEmpty()) t("Off") else parts.joinToString(" · ")
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
    var denied by remember { mutableStateOf(false) }
    var tested by remember { mutableStateOf(false) }
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
        scope.launch { model.resyncNotifications() }
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
            launcher.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }
    fun lead(key: String, minutes: Int) = scope.launch { model.setNotifications { it.put(key, minutes) } }
    Page {
        Card {
            SwitchRow(t("Remind me before classes"), settings?.optBoolean("classes") == true) { toggle("classes", it) }
            if (settings?.optBoolean("classes") == true) {
                RowDivider(inset = 16.dp)
                PickerRow(t("Remind me"), ReminderLeads.classLead(settings), ReminderLeads.classChoices) { lead("classLeadMinutes", it) }
            }
        }
        Card(Modifier.padding(top = 8.dp)) {
            SwitchRow(t("Remind me before exams"), ReminderLeads.exams(settings)) { toggle("exams", it) }
            if (ReminderLeads.exams(settings)) {
                val current = ReminderLeads.examLead(settings)
                RowDivider(inset = 16.dp)
                PickerRow(t("Remind me"), current, (ReminderLeads.examChoices + current).distinct().sorted()) { lead("examLeadMinutes", it) }
            }
        }
        Card(Modifier.padding(top = 8.dp)) {
            SwitchRow(t("Remind me before assignment deadlines"), settings?.optBoolean("assignments") == true) { toggle("assignments", it) }
            if (settings?.optBoolean("assignments") == true) {
                RowDivider(inset = 16.dp)
                NavRow(
                    t("Remind me"),
                    ReminderLeads.assignmentLeads(settings).sortedDescending().joinToString("、", transform = ReminderLeads::label),
                    onClick = openLeads,
                )
            }
        }
        Card(Modifier.padding(top = 8.dp)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp, top = 4.dp, bottom = 4.dp),
            ) {
                Text(t("Test notification"), color = Theme.colors.ink, modifier = Modifier.weight(1f))
                TextButton(onClick = {
                    if (NotificationScheduler.allowed(context)) {
                        sendTest()
                    } else {
                        pending = TEST
                        launcher.launch(Manifest.permission.POST_NOTIFICATIONS)
                    }
                }) { Text(t("Send")) }
            }
            if (denied) {
                RowDivider(inset = 16.dp)
                NavRow(t("Open Settings"), null) {
                    context.startActivity(
                        Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                            .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName),
                    )
                }
            }
        }
        if (denied) {
            Footer(t("Notifications are not allowed. Allow them in the Settings app under Notifications."), Theme.colors.danger)
        } else if (tested) {
            Footer(t("The system accepted the test notification. If no banner appears, check notification settings, Focus, and screen sharing."))
        }
    }
}

/** `pending` for the test notification rather than a reminder switch. */
private const val TEST = "test"

/** The desktop's multi-select of assignment reminder times: one to eight of
 * the offered leads, latest first. */
@Composable
internal fun AssignmentLeadsPage() {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    val chosen = ReminderLeads.assignmentLeads(model.records.settings.optJSONObject("notifications"))
    Page {
        Card {
            val options = (ReminderLeads.assignmentChoices + chosen).distinct().sortedDescending()
            options.forEachIndexed { index, minutes ->
                if (index > 0) RowDivider(inset = 16.dp)
                val on = minutes in chosen
                val enabled = if (on) chosen.size > 1 else chosen.size < ReminderLeads.MAX_ASSIGNMENT
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier
                        .fillMaxWidth()
                        .clickable(enabled = enabled) {
                            val next = if (on) chosen - minutes else chosen + minutes
                            scope.launch { model.setNotifications { it.put("assignmentLeadMinutes", JSONArray(next)) } }
                        }
                        .padding(16.dp),
                ) {
                    Text(
                        ReminderLeads.label(minutes), color = if (enabled || on) Theme.colors.ink else Theme.colors.muted,
                        modifier = Modifier.weight(1f),
                    )
                    if (on) Icon(Icons.Outlined.Check, null, tint = Theme.colors.accent)
                }
            }
        }
        Footer(t("Reminds you at every time you choose before the deadline (1–{{max}}).", "max" to ReminderLeads.MAX_ASSIGNMENT))
    }
}

/** 学習プロフィール: department, admission year and degree programme, the
 * desktop's Study settings. The department is `settings.departmentId`; the
 * year and programme go through Core's `setStudyProfile`, as on desktop. Each
 * choice saves at once; a programme picked before the year waits for it. */
@Composable
internal fun StudyProfilePage() {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    var departments by remember { mutableStateOf<List<FinderDepartment>>(emptyList()) }
    var stored by remember { mutableStateOf<AppModel.StudyProfile?>(null) }
    var year by remember { mutableStateOf<Int?>(null) }
    var stage by remember { mutableStateOf(StudyStage.UNKNOWN) }
    var currentYear by remember { mutableStateOf<Int?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    suspend fun load() {
        val profile = model.studyProfile() ?: return
        stored = profile
        year = profile.admissionYear
        stage = profile.enrollments.lastOrNull()?.stage ?: StudyStage.UNKNOWN
    }
    LaunchedEffect(Unit) {
        departments = model.departments()
        currentYear = model.academicYear(Format.today())
    }
    LaunchedEffect(model.contentRevision) { load() }

    // The stored department, matched like the desktop's `departmentById`
    // (a plain id or a subscription-scoped "2026:cs").
    val raw = model.records.settings.optString("departmentId")
    val departmentId = departments.firstOrNull { raw == it.id || raw.endsWith(":${it.id}") }?.id ?: raw
    val departmentOptions = listOf("" to t("Not set")) +
        departments.map { it.id to (Localizer.pick(it.name, model.records.settings) ?: it.id) } +
        // A stored id this list does not hold still shows.
        listOfNotNull(departmentId.takeIf { id -> id.isNotEmpty() && departments.none { it.id == id } }?.let { it to it })
    // The desktop's choices: the ten years up to next year, plus a stored one.
    val now = currentYear ?: Format.today().year
    val years = ((0 until 10).map { now + 1 - it } + listOfNotNull(year)).distinct().sortedDescending()
    val stages = listOf(
        StudyStage.UNKNOWN to t("Not set"), StudyStage.UNDERGRADUATE to t("Undergraduate"), StudyStage.MASTER to t("Master"),
        StudyStage.PROFESSIONAL to t("Professional degree"), StudyStage.DOCTORAL to t("Doctoral"),
    )

    /** Builds the profile the desktop's save would: the programme replaces the
     * latest enrollment's stage, and enrollments without a term start in the
     * admission year's 1Q. */
    fun choose(chosenYear: Int?, chosenStage: StudyStage) {
        year = chosenYear
        stage = chosenStage
        error = null
        val enrollments = stored?.enrollments.orEmpty().toMutableList()
        if (enrollments.isEmpty()) {
            if (chosenStage != StudyStage.UNKNOWN) enrollments.add(StudyEnrollment(chosenStage, StudyEntryKind.UNKNOWN, null, ""))
        } else {
            enrollments[enrollments.lastIndex] = enrollments.last().copy(stage = chosenStage)
        }
        if (enrollments.any { it.admittedTerm.isEmpty() }) {
            if (chosenYear == null) {
                error = t("Choose an admission year.")
                return
            }
            enrollments.replaceAll { if (it.admittedTerm.isEmpty()) it.copy(admittedTerm = "$chosenYear-1Q") else it }
        }
        enrollments.sortBy { it.admittedTerm }
        val profile = AppModel.StudyProfile(chosenYear, enrollments)
        scope.launch {
            try {
                model.setStudyProfile(profile)
                stored = profile
            } catch (failure: Exception) {
                error = AppModel.describe(failure)
                load()
            }
        }
    }

    Page {
        Card {
            PickerRow(t("Department"), departmentId, departmentOptions) { id ->
                scope.launch { model.updateSettings { if (id.isEmpty()) it.remove("departmentId") else it.put("departmentId", id) } }
            }
            RowDivider(inset = 16.dp)
            PickerRow(
                t("Admission year"), year,
                listOf<Pair<Int?, String>>(null to t("Not set")) + years.map { it to t("Entered {{year}}", "year" to it) },
            ) { choose(it, stage) }
            RowDivider(inset = 16.dp)
            PickerRow(t("Degree stage"), stage, stages) { choose(year, it) }
        }
        error?.let { Footer(it, Theme.colors.danger) }
            ?: Footer(t("Used for course suggestions and the study guide. Syncs with the desktop app."))
    }
}

/** データ更新: whether Core's automatic school reads may use cellular data
 * (`settings.subscriptionRefresh.allowCellular`), Core's sync sources and
 * runs, and a manual refresh that asks Core's planner for everything. */
@Composable
internal fun DataUpdatesPage() {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    val cellular = model.records.settings.optJSONObject("subscriptionRefresh")?.optBoolean("allowCellular") == true
    Page {
        Card {
            SwitchRow(t("Update on cellular data"), cellular) { on ->
                scope.launch {
                    model.updateSettings { settings ->
                        val refresh = settings.optJSONObject("subscriptionRefresh") ?: JSONObject()
                        refresh.put("allowCellular", on)
                        settings.put("subscriptionRefresh", refresh)
                    }
                }
            }
        }
        Footer(t("When off, automatic updates wait until you are on Wi-Fi."))
        SyncSections()
        Card(Modifier.padding(top = 8.dp)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.fillMaxWidth().clickable { model.refreshFromSchool() }.padding(16.dp),
            ) {
                Text(t("Refresh now"), color = Theme.colors.accent, modifier = Modifier.weight(1f))
                model.syncSnapshot?.indicator?.let { SyncIndicatorView(it) }
            }
        }
    }
}

internal fun dataSummary(settings: JSONObject): String =
    if (settings.optJSONObject("subscriptionRefresh")?.optBoolean("allowCellular") == true) {
        t("Update on cellular data")
    } else {
        t("Updates automatically on Wi-Fi only")
    }

/** A labelled choice that opens a menu of `options`, the Form picker row. */
@Composable
internal fun <T> PickerRow(title: String, selected: T, options: List<Pair<T, String>>, choose: (T) -> Unit) {
    var menu by remember { mutableStateOf(false) }
    Box {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            modifier = Modifier.fillMaxWidth().clickable { menu = true }.padding(16.dp),
        ) {
            Text(title, color = Theme.colors.ink, modifier = Modifier.weight(1f))
            Text(options.firstOrNull { it.first == selected }?.second.orEmpty(), color = Theme.colors.muted)
            Icon(Icons.Outlined.UnfoldMore, null, tint = Theme.colors.muted, modifier = Modifier.size(18.dp))
        }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }, modifier = Modifier.align(Alignment.TopEnd)) {
            for ((value, label) in options) {
                DropdownMenuItem(
                    text = { Text(label) },
                    trailingIcon = { if (value == selected) Icon(Icons.Outlined.Check, null, tint = Theme.colors.accent) },
                    onClick = {
                        menu = false
                        if (value != selected) choose(value)
                    },
                )
            }
        }
    }
}

@JvmName("minutePickerRow")
@Composable
private fun PickerRow(title: String, selected: Int, minutes: List<Int>, choose: (Int) -> Unit) =
    PickerRow(title, selected, minutes.map { it to ReminderLeads.label(it) }, choose)

/** A row that opens a page: title, its current value, a chevron. */
/** The system settings' row icon, as iOS draws it: a white symbol on a
 * rounded square in a token role (accent for preferences, ok, warn, danger),
 * never a rainbow of colours. */
@Composable
internal fun SettingsIcon(icon: ImageVector, tint: Color) {
    Box(
        Modifier.size(SETTINGS_ICON).clip(RoundedCornerShape(7.dp))
            .background(Brush.verticalGradient(listOf(tint.copy(alpha = 0.82f).compositeOver(Color.White), tint))),
        contentAlignment = Alignment.Center,
    ) { Icon(icon, null, tint = Color.White, modifier = Modifier.size(18.dp)) }
}

internal val SETTINGS_ICON = 29.dp

/** Where a divider starts under a row with a settings icon: at its text. */
internal val ICON_ROW_INSET = 16.dp + SETTINGS_ICON + 12.dp

@Composable
internal fun NavRow(title: String, detail: String?, icon: ImageVector? = null, tint: Color = Theme.colors.accent, onClick: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick).padding(16.dp),
    ) {
        icon?.let { SettingsIcon(it, tint) }
        Column(Modifier.weight(1f)) {
            Text(title, color = Theme.colors.ink)
            detail?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted, maxLines = 2) }
        }
        Icon(Icons.Outlined.ChevronRight, null, tint = Theme.colors.muted)
    }
}
