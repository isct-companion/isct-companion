package app.isctcompanion.features.settings

import app.isctcompanion.design.t
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SegmentedButton
import android.Manifest
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.Notifications
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.autofill.ContentType
import androidx.compose.ui.semantics.contentType
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.CredentialAvailability
import app.isctcompanion.core.CredentialSummary
import app.isctcompanion.core.NotificationScheduler
import app.isctcompanion.core.SchoolAuthProfile
import app.isctcompanion.core.SchoolCredentials
import app.isctcompanion.core.SchoolSessionState
import app.isctcompanion.design.Card
import app.isctcompanion.design.Format
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.Theme
import kotlinx.coroutines.launch

private sealed interface SettingsPage {
    data object Root : SettingsPage
    data class Account(val profile: SchoolAuthProfile, val name: String) : SettingsPage
    data object Notifications : SettingsPage

    companion object {
        /** Keeps the open page across a tab switch, which drops the composition. */
        val Saver = androidx.compose.runtime.saveable.Saver<SettingsPage, String>(
            save = {
                when (it) {
                    Root -> "root"
                    Notifications -> "notifications"
                    is Account -> "account:${it.profile.name}:${it.name}"
                }
            },
            restore = { raw ->
                val parts = raw.split(":", limit = 3)
                when (parts[0]) {
                    "notifications" -> Notifications
                    "account" -> runCatching { Account(SchoolAuthProfile.valueOf(parts[1]), parts[2]) }.getOrNull() ?: Root
                    else -> Root
                }
            },
        )
    }
}

/** 設定, the last tab: school accounts, reminders and language. Planning,
 * calendars, folders and developer settings stay on desktop. */
@Composable
fun SettingsScreen() {
    var page by rememberSaveable(stateSaver = SettingsPage.Saver) { mutableStateOf<SettingsPage>(SettingsPage.Root) }
    BackHandler(page != SettingsPage.Root) { page = SettingsPage.Root }
    Column(Modifier.fillMaxSize().background(Theme.colors.bg).imePadding()) {
        when (val current = page) {
            SettingsPage.Root -> {
                Bar(t("Settings"))
                RootPage { page = it }
            }
            is SettingsPage.Account -> {
                Bar(current.name, back = { page = SettingsPage.Root })
                AccountPage(current.profile) { page = SettingsPage.Root }
            }
            SettingsPage.Notifications -> {
                Bar(t("Notifications"), back = { page = SettingsPage.Root })
                NotificationsPage()
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Bar(title: String, back: (() -> Unit)? = null) {
    TopAppBar(
        title = { Text(title) },
        navigationIcon = {
            back?.let { IconButton(onClick = it) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, t("Back")) } }
        },
        colors = TopAppBarDefaults.topAppBarColors(containerColor = Theme.colors.bg),
    )
}

@Composable
private fun Page(content: @Composable ColumnScope.() -> Unit) {
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(bottom = 32.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
        content = content,
    )
}

@Composable
private fun RootPage(open: (SettingsPage) -> Unit) {
    val model = LocalModel.current
    val context = LocalContext.current
    Page {
        SectionLabel(t("School accounts"))
        Card {
            AccountRow("Science Tokyo", model.credentials?.science, model.sessions?.science, model.records.lastRun(SchoolAuthProfile.SCIENCE)) {
                open(SettingsPage.Account(SchoolAuthProfile.SCIENCE, "Science Tokyo"))
            }
            RowDivider(inset = 36.dp)
            AccountRow(t("Tokyo Tech (former)"), model.credentials?.tokyoTech, model.sessions?.tokyoTech, model.records.lastRun(SchoolAuthProfile.TOKYO_TECH)) {
                open(SettingsPage.Account(SchoolAuthProfile.TOKYO_TECH, t("Tokyo Tech (former)")))
            }
        }
        Card(Modifier.padding(top = 16.dp)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                modifier = Modifier.fillMaxWidth().clickable { open(SettingsPage.Notifications) }.padding(16.dp),
            ) {
                Icon(Icons.Outlined.Notifications, null, tint = Theme.colors.accent)
                Column(Modifier.weight(1f)) {
                    Text(t("Notifications"), color = Theme.colors.ink)
                    Text(notificationSummary(), style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted)
                }
                Icon(Icons.Outlined.ChevronRight, null, tint = Theme.colors.muted)
            }
        }
        LanguageCard()
        Footer(t("Calendar editing, course folders and developer settings are in the desktop app."))
        Card(Modifier.padding(top = 16.dp)) {
            Row(Modifier.fillMaxWidth().padding(16.dp)) {
                Text("IsctCompanion", color = Theme.colors.ink, modifier = Modifier.weight(1f))
                val version = runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull()
                Text(version.orEmpty(), color = Theme.colors.muted)
            }
        }
    }
}

@Composable
private fun Footer(text: String, color: Color = Theme.colors.muted) {
    Text(text, style = MaterialTheme.typography.bodySmall, color = color, modifier = Modifier.padding(horizontal = 16.dp))
}

@Composable
private fun notificationSummary(): String {
    val settings = LocalModel.current.records.settings.optJSONObject("notifications")
    val parts = mutableListOf<String>()
    if (settings?.optBoolean("classes") == true) parts.add(t("Classes {{lead}}", "lead" to t("{{minutes}} minutes before", "minutes" to settings.optInt("classLeadMinutes", 10))))
    if (settings?.optBoolean("assignments") == true) parts.add(t("Assignments on the day due"))
    return if (parts.isEmpty()) t("Off") else parts.joinToString(" · ")
}

@Composable
private fun AccountRow(
    name: String,
    credential: CredentialSummary?,
    session: SchoolSessionState?,
    lastRun: String?,
    onClick: () -> Unit,
) {
    val connected = credential?.availability == CredentialAvailability.AVAILABLE ||
        credential?.availability == CredentialAvailability.SAVED_LOCKED
    val (label, color) = when {
        !connected -> t("Not connected") to Theme.colors.muted
        session == SchoolSessionState.ACTIVE -> (lastRun?.let { t("Connected · updated {{time}}", "time" to Format.due(it)) } ?: t("Connected")) to Theme.colors.ok
        session == SchoolSessionState.FAILED -> t("Sign-in failed") to Theme.colors.danger
        session == SchoolSessionState.MAINTENANCE -> t("Under maintenance") to Theme.colors.warn
        else -> t("Checking") to Theme.colors.muted
    }
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick).padding(16.dp),
    ) {
        Box(Modifier.size(8.dp).clip(CircleShape).background(if (connected) color else Theme.colors.line))
        Column(Modifier.weight(1f)) {
            Text(name, color = Theme.colors.ink)
            Text(label, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted)
        }
        Icon(Icons.Outlined.ChevronRight, null, tint = Theme.colors.muted)
    }
}

/** Sign in, change or forget a school account. The password goes straight to
 * Core, which stores it in the Keystore vault and starts the session check. */
@Composable
private fun AccountPage(profile: SchoolAuthProfile, done: () -> Unit) {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    val summary = if (profile == SchoolAuthProfile.SCIENCE) model.credentials?.science else model.credentials?.tokyoTech
    var username by remember { mutableStateOf(summary?.username.orEmpty()) }
    var password by remember { mutableStateOf("") }
    var otpSecret by remember { mutableStateOf("") }
    var matrix by remember { mutableStateOf("") }
    var saving by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val saved = summary?.saved == true
    Page {
        OutlinedTextField(
            username, { username = it }, label = { Text(t("Student ID")) }, singleLine = true,
            keyboardOptions = KeyboardOptions(
                capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false, keyboardType = KeyboardType.Ascii,
            ),
            // Compose fields join system Autofill only when they say what they hold.
            modifier = Modifier.fillMaxWidth().semantics { contentType = ContentType.Username },
        )
        Secret(password, { password = it }, if (saved) t("Password (enter only to change)") else t("Password"), login = true)
        Footer(t("The password is encrypted in this device’s keystore and sent only to the school sites."))
        if (profile == SchoolAuthProfile.SCIENCE) {
            SectionLabel(t("One-time password"))
            OtpImportField(summary?.hasOtp == true, otpSecret) { otpSecret = it }
        } else {
            SectionLabel(t("Matrix card"))
            MatrixImportSection(summary?.hasMatrix == true) { matrix = it }
        }
        error?.let { Footer(it, Theme.colors.danger) }
        Button(
            onClick = {
                scope.launch {
                    saving = true
                    error = null
                    try {
                        model.saveCredentials(profile, SchoolCredentials(username, password, otpSecret, matrix))
                        done()
                    } catch (failure: Exception) {
                        error = AppModel.describe(failure)
                    }
                    saving = false
                }
            },
            enabled = !saving && username.isNotEmpty() && (password.isNotEmpty() || saved),
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
        ) {
            if (saving) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp) else Text(t("Save and connect"))
        }
        if (saved) {
            TextButton(
                onClick = {
                    scope.launch {
                        model.deleteCredentials(profile)
                        done()
                    }
                },
                modifier = Modifier.fillMaxWidth(),
            ) { Text(t("Remove sign-in details from this device"), color = Theme.colors.danger) }
        }
    }
}

@Composable
private fun Secret(
    value: String,
    onChange: (String) -> Unit,
    label: String,
    capitalization: KeyboardCapitalization = KeyboardCapitalization.None,
    login: Boolean = false,
) {
    // Only the login password is offered to password managers; the TOTP secret
    // and matrix are pasted keys and stay out of Autofill.
    OutlinedTextField(
        value, onChange, label = { Text(label) }, singleLine = true,
        visualTransformation = PasswordVisualTransformation(),
        keyboardOptions = KeyboardOptions(
            capitalization = capitalization, autoCorrectEnabled = false,
            keyboardType = if (login) KeyboardType.Password else KeyboardType.Ascii,
        ),
        modifier = Modifier.fillMaxWidth().then(
            if (login) Modifier.semantics { contentType = ContentType.Password } else Modifier,
        ),
    )
}

/** Reminder switches; the system permission is asked on the first switch-on. */
@Composable
private fun NotificationsPage() {
    val model = LocalModel.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var denied by remember { mutableStateOf(false) }
    var pending by remember { mutableStateOf<String?>(null) }
    val settings = model.records.settings.optJSONObject("notifications")
    val turnOn = { key: String, permission: String ->
        scope.launch {
            model.setNotifications {
                it.put(key, true)
                it.put("localPermission", permission)
            }
        }
    }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        denied = !granted
        val key = pending
        pending = null
        if (granted && key != null) turnOn(key, "granted")
    }
    val toggle = { key: String, on: Boolean ->
        if (!on) {
            scope.launch { model.setNotifications { it.put(key, false) } }
        } else if (NotificationScheduler.allowed(context)) {
            turnOn(key, "granted")
        } else {
            pending = key
            launcher.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }
    Page {
        Card {
            SwitchRow(t("Remind me before classes"), settings?.optBoolean("classes") == true) { toggle("classes", it) }
            if (settings?.optBoolean("classes") == true) {
                RowDivider(inset = 16.dp)
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(t("Remind me"), color = Theme.colors.ink)
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        val lead = settings.optInt("classLeadMinutes", 10)
                        for (minutes in listOf(5, 10, 15, 30)) {
                            FilterChip(
                                selected = lead == minutes,
                                onClick = { scope.launch { model.setNotifications { it.put("classLeadMinutes", minutes) } } },
                                label = { Text(t("{{minutes}} minutes before", "minutes" to minutes)) },
                            )
                        }
                    }
                }
            }
            RowDivider(inset = 16.dp)
            SwitchRow(t("Remind me on the day an assignment is due"), settings?.optBoolean("assignments") == true) { toggle("assignments", it) }
        }
        if (denied) Footer(t("Notifications are not allowed. Allow them in the Settings app under Notifications."), Theme.colors.danger)
    }
}

@Composable
private fun SwitchRow(title: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
    ) {
        Text(title, color = Theme.colors.ink, modifier = Modifier.weight(1f))
        Switch(checked, onChange)
    }
}

/** The desktop's language rows: the UI language and the language of school
 * text. Both are workspace settings, so they follow the person to desktop. */
@Composable
private fun LanguageCard() {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    val settings = model.records.settings
    fun set(key: String, value: String) = scope.launch { model.updateSettings { it.put(key, value) } }
    Card(Modifier.padding(top = 16.dp)) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text(t("Language"), color = Theme.colors.ink)
            Choices(
                listOf("en" to "English", "ja" to "日本語", "zh" to "中文"),
                settings.optString("locale").ifEmpty { "en" },
            ) { set("locale", it) }
            Text(t("School content language"), color = Theme.colors.ink, modifier = Modifier.padding(top = 6.dp))
            Choices(
                listOf("auto" to t("Automatic"), "en" to "English", "ja" to "日本語"),
                settings.optString("contentLanguage").ifEmpty { "auto" },
            ) { set("contentLanguage", it) }
            Text(
                t("Course names, rooms and syllabus text. Automatic uses the menu language."),
                style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted,
            )
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Choices(options: List<Pair<String, String>>, selected: String, choose: (String) -> Unit) {
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
        options.forEachIndexed { index, (value, label) ->
            SegmentedButton(
                selected = value == selected,
                onClick = { if (value != selected) choose(value) },
                shape = SegmentedButtonDefaults.itemShape(index, options.size),
            ) { Text(label) }
        }
    }
}
