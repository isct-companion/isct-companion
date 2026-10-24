package app.isctcompanion.features.settings

import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.material.icons.automirrored.filled.MenuBook
import androidx.compose.material.icons.filled.Translate
import androidx.compose.material.icons.filled.Language
import androidx.compose.material.icons.filled.FormatSize
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.RestartAlt
import androidx.compose.material.icons.filled.Restore
import androidx.compose.material.icons.filled.Lightbulb
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Cloud
import androidx.compose.material.icons.filled.Sync
import androidx.compose.material.icons.filled.School
import androidx.compose.material.icons.filled.Notifications
import app.isctcompanion.design.t
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SegmentedButton
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.widget.Toast
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
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
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
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
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
import androidx.compose.runtime.LaunchedEffect
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
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.CredentialAvailability
import app.isctcompanion.core.CredentialSummary
import app.isctcompanion.core.SchoolAuthProfile
import app.isctcompanion.core.SchoolCredentials
import app.isctcompanion.core.SchoolBrowserHost
import app.isctcompanion.core.SchoolSessionState
import app.isctcompanion.core.SyncState
import app.isctcompanion.core.WebRouter
import app.isctcompanion.core.exportApplicationLogs
import app.isctcompanion.design.Card
import app.isctcompanion.design.Format
import app.isctcompanion.design.Guide
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.Theme
import java.io.IOException
import java.time.Instant
import java.time.LocalDate
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private sealed interface SettingsPage {
    data object Root : SettingsPage
    data class Account(val profile: SchoolAuthProfile, val name: String) : SettingsPage
    data object Notifications : SettingsPage
    data object AssignmentLeads : SettingsPage
    data object StudyProfile : SettingsPage
    data object DataUpdates : SettingsPage
    data object Backup : SettingsPage
    data class Reset(val kind: ResetKind) : SettingsPage

    /** Where back goes: a page opened from another returns to it. */
    val parent: SettingsPage
        get() = if (this == AssignmentLeads) Notifications else Root

    companion object {
        /** Keeps the open page across a tab switch, which drops the composition. */
        val Saver = androidx.compose.runtime.saveable.Saver<SettingsPage, String>(
            save = {
                when (it) {
                    Root -> "root"
                    Notifications -> "notifications"
                    AssignmentLeads -> "assignment-leads"
                    StudyProfile -> "study-profile"
                    DataUpdates -> "data-updates"
                    Backup -> "backup"
                    is Reset -> "reset:${it.kind.name}"
                    is Account -> "account:${it.profile.name}:${it.name}"
                }
            },
            restore = { raw ->
                val parts = raw.split(":", limit = 3)
                when (parts[0]) {
                    "notifications" -> Notifications
                    "assignment-leads" -> AssignmentLeads
                    "study-profile" -> StudyProfile
                    "data-updates" -> DataUpdates
                    "backup" -> Backup
                    "reset" -> runCatching { Reset(ResetKind.valueOf(parts[1])) }.getOrNull() ?: Root
                    "account" -> runCatching { Account(SchoolAuthProfile.valueOf(parts[1]), parts[2]) }.getOrNull() ?: Root
                    else -> Root
                }
            },
        )
    }
}

/** 設定, the last tab: the desktop settings sections that apply on a phone —
 * school accounts, reminders, study profile, data updates, language and
 * reset. A group with more than two controls gets a page. Planning,
 * calendars, folders and developer settings stay on desktop. */
@Composable
fun SettingsScreen() {
    var page by rememberSaveable(stateSaver = SettingsPage.Saver) { mutableStateOf<SettingsPage>(SettingsPage.Root) }
    // The welcome sheet's "Connect school accounts" lands on the account page.
    val navigator = LocalNavigator.current
    LaunchedEffect(navigator.settingsAccount) {
        val profile = navigator.settingsAccount ?: return@LaunchedEffect
        navigator.settingsAccount = null
        page = SettingsPage.Account(profile, accountName(profile))
    }
    // A running reset holds the page, as on iOS: leaving would look like it
    // stopped while Core carries on.
    var busy by remember { mutableStateOf(false) }
    BackHandler(page != SettingsPage.Root) { if (!busy) page = page.parent }
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
                NotificationsPage { page = SettingsPage.AssignmentLeads }
            }
            SettingsPage.AssignmentLeads -> {
                Bar(t("Assignment reminder times"), back = { page = SettingsPage.Notifications })
                AssignmentLeadsPage()
            }
            SettingsPage.StudyProfile -> {
                Bar(t("Study profile"), back = { page = SettingsPage.Root })
                StudyProfilePage()
            }
            SettingsPage.DataUpdates -> {
                Bar(t("Data updates"), back = { page = SettingsPage.Root })
                DataUpdatesPage()
            }

            SettingsPage.Backup -> {
                Bar(t("Backup"), back = { page = SettingsPage.Root })
                BackupPage()
            }
            is SettingsPage.Reset -> {
                val title = if (current.kind == ResetKind.Everything) "settings.reset.full.title" else "settings.reset.caches.title"
                Bar(t(title), back = if (busy) null else ({ page = SettingsPage.Root }))
                ResetPage(current.kind, onBusy = { busy = it }) { page = SettingsPage.Root }
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
internal fun Page(content: @Composable ColumnScope.() -> Unit) {
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
            val science = accountName(SchoolAuthProfile.SCIENCE)
            AccountRow(science, model.credentials?.science, model.sessions?.science, model.syncSource("lms")?.lastUpdated) {
                open(SettingsPage.Account(SchoolAuthProfile.SCIENCE, science))
            }
            RowDivider(inset = 36.dp)
            val tokyoTech = accountName(SchoolAuthProfile.TOKYO_TECH)
            AccountRow(tokyoTech, model.credentials?.tokyoTech, model.sessions?.tokyoTech, model.syncSource("kyomu")?.lastUpdated) {
                open(SettingsPage.Account(SchoolAuthProfile.TOKYO_TECH, tokyoTech))
            }
        }
        Card(Modifier.padding(top = 16.dp)) {
            NavRow(
                t("Notifications"), ReminderLeads.summary(model.records.settings.optJSONObject("notifications")),
                Icons.Filled.Notifications, Theme.colors.danger,
            ) { open(SettingsPage.Notifications) }
            RowDivider(inset = ICON_ROW_INSET)
            NavRow(t("Study profile"), null, Icons.Filled.School) { open(SettingsPage.StudyProfile) }
            RowDivider(inset = ICON_ROW_INSET)
            NavRow(t("Data updates"), dataSummary(model.records.settings), Icons.Filled.Sync, Theme.colors.ok) {
                open(SettingsPage.DataUpdates)
            }
            RowDivider(inset = ICON_ROW_INSET)
            NavRow(t("Backup"), backupSummary(model.cloud), Icons.Filled.Cloud) { open(SettingsPage.Backup) }
        }
        LanguageCard()
        WebCard()
        // As on iOS: About holds the version, the tips and the logs, then the
        // desktop note; the destructive Reset group comes last.
        Spacer(Modifier.height(8.dp))
        SectionLabel(t("About"))
        Card {
            Row(
                Modifier.fillMaxWidth().padding(16.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                SettingsIcon(Icons.Filled.Info, Theme.colors.muted)
                Text("IsctCompanion", color = Theme.colors.ink, modifier = Modifier.weight(1f))
                val version = runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull()
                Text(version.orEmpty(), color = Theme.colors.muted)
            }
            RowDivider(inset = ICON_ROW_INSET)
            ActionRow(t("Show welcome and tips again"), Icons.Filled.Lightbulb) { Guide.reset() }
            RowDivider(inset = ICON_ROW_INSET)
            LogsRow()
        }
        Footer(t("Calendar editing, course folders and developer settings are in the desktop app."))
        ResetCard(open)
    }
}

private fun accountName(profile: SchoolAuthProfile) =
    if (profile == SchoolAuthProfile.SCIENCE) "Science Tokyo" else t("Tokyo Tech (former)")

/** The desktop's Reset group: each row opens its confirmation page. */
@Composable
private fun ResetCard(open: (SettingsPage) -> Unit) {
    Spacer(Modifier.height(8.dp))
    SectionLabel(t("settings.reset.group"))
    Card {
        ResetRow(t("settings.reset.caches.title"), t("settings.reset.caches.hint"), Theme.colors.ink, Icons.Filled.Restore, Theme.colors.warn) {
            open(SettingsPage.Reset(ResetKind.SchoolCaches))
        }
        RowDivider(inset = ICON_ROW_INSET)
        ResetRow(t("settings.reset.full.title"), t("settings.reset.full.hint"), Theme.colors.danger, Icons.Filled.RestartAlt, Theme.colors.danger) {
            open(SettingsPage.Reset(ResetKind.Everything))
        }
    }
}

@Composable
private fun ResetRow(title: String, hint: String, color: Color, icon: ImageVector, tint: Color, onClick: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick).padding(16.dp),
    ) {
        SettingsIcon(icon, tint)
        Column(Modifier.weight(1f)) {
            Text(title, color = color)
            Text(hint, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted)
        }
        Icon(Icons.Outlined.ChevronRight, null, tint = Theme.colors.muted)
    }
}

/** Core formats and redacts the log; the shell names its build, and the
 * system document picker and clipboard deliver it. */
@Composable
private fun LogsRow() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    fun fail(message: String) = Toast.makeText(context, message, Toast.LENGTH_LONG).show()
    val save = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("text/plain")) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult
        scope.launch {
            runCatching {
                withContext(Dispatchers.IO) {
                    val stream = context.contentResolver.openOutputStream(uri) ?: throw IOException("No output stream")
                    stream.use { it.write(logText(context).toByteArray()) }
                }
            }.onFailure { fail(t("Export failed: {{reason}}", "reason" to AppModel.describe(it))) }
        }
    }
    // One row for both log actions, as a menu (iOS's Application logs row).
    var menu by remember { mutableStateOf(false) }
    Box {
        ActionRow(t("Application logs"), Icons.Filled.Description) { menu = true }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            DropdownMenuItem(
                text = { Text(t("Save to file")) },
                leadingIcon = { Icon(Icons.Filled.Download, null) },
                onClick = {
                    menu = false
                    save.launch("IsctCompanion-logs-${LocalDate.now()}.txt")
                },
            )
            DropdownMenuItem(
                text = { Text(t("Copy")) },
                leadingIcon = { Icon(Icons.Filled.ContentCopy, null) },
                onClick = {
                    menu = false
                    // Android shows its own confirmation for every clipboard write.
                    runCatching {
                        context.getSystemService(ClipboardManager::class.java)
                            .setPrimaryClip(ClipData.newPlainText(t("Application logs"), logText(context)))
                    }.onFailure { fail(t("Copy failed")) }
                },
            )
        }
    }
}

private fun logText(context: Context): String {
    val version = runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull()
    return exportApplicationLogs(
        "IsctCompanion ${version.orEmpty()} · Android ${Build.VERSION.RELEASE} · ${Build.MANUFACTURER} ${Build.MODEL}",
    )
}

/** An action row; with an icon it reads as a settings row, as on iOS. */
@Composable
private fun ActionRow(title: String, icon: ImageVector? = null, enabled: Boolean = true, onClick: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        modifier = Modifier.fillMaxWidth().clickable(enabled = enabled, onClick = onClick).padding(16.dp),
    ) {
        icon?.let { SettingsIcon(it, Theme.colors.muted) }
        Text(
            title,
            color = when {
                !enabled -> Theme.colors.muted
                icon != null -> Theme.colors.ink
                else -> Theme.colors.accent
            },
        )
    }
}

/** A heading inside a card, with its settings icon. */
@Composable
private fun IconHeading(title: String, icon: ImageVector, tint: Color, modifier: Modifier = Modifier) {
    Row(modifier, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        SettingsIcon(icon, tint)
        Text(title, color = Theme.colors.ink)
    }
}

@Composable
internal fun Footer(text: String, color: Color = Theme.colors.muted) {
    Text(text, style = MaterialTheme.typography.bodySmall, color = color, modifier = Modifier.padding(horizontal = 16.dp))
}

/** Off, or when the cloud copy last synced; every connected provider counts,
 * so a Drive-only phone reads the same as an iCloud one. */
private fun backupSummary(cloud: SyncState?): String {
    val enabled = cloud?.providers.orEmpty().filter { it.enabled }
    if (enabled.isEmpty()) return t("Off")
    val synced = enabled.mapNotNull { it.lastSyncedAt }.maxByOrNull { Format.date(it) ?: Instant.EPOCH }
    return synced?.let { t("Synced {{date}}", "date" to Format.due(it)) } ?: enabled.joinToString(" · ") { providerName(it.provider) }
}

/** The ウェブ tab's device preferences: page text size and the site it opens on. */
@Composable
private fun WebCard() {
    val web = LocalModel.current.web
    Spacer(Modifier.height(8.dp))
    SectionLabel(t("Web"))
    Card {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            IconHeading(t("Text size"), Icons.Filled.FormatSize, Theme.colors.muted)
            Choices(SchoolBrowserHost.textScales.map { "$it" to "$it%" }, "${web.browser.textScale}") {
                web.browser.chooseTextScale(it.toInt())
            }
        }
        RowDivider(inset = ICON_ROW_INSET)
        var picking by remember { mutableStateOf(false) }
        Box {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                modifier = Modifier.fillMaxWidth().clickable { picking = true }.padding(16.dp),
            ) {
                SettingsIcon(Icons.Filled.Language, Theme.colors.accent)
                Text(t("First site"), color = Theme.colors.ink, modifier = Modifier.weight(1f))
                Text(WebRouter.name(web.firstSite), color = Theme.colors.muted)
            }
            DropdownMenu(expanded = picking, onDismissRequest = { picking = false }) {
                for (site in WebRouter.tabSites) {
                    DropdownMenuItem(
                        text = { Text(WebRouter.name(site)) },
                        trailingIcon = { if (site == web.firstSite) Icon(Icons.Outlined.Check, null) },
                        onClick = {
                            picking = false
                            web.chooseFirstSite(site)
                        },
                    )
                }
            }
        }
    }
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
        session == SchoolSessionState.ACTIVE -> (
            lastRun?.let { t("Connected · updated {{time}}", "time" to Format.due(it)) } ?: t("Connected")
            ) to Theme.colors.ok
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

@Composable
internal fun SwitchRow(title: String, checked: Boolean, onChange: (Boolean) -> Unit) {
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
            IconHeading(t("Language"), Icons.Filled.Translate, Theme.colors.accent)
            Choices(
                listOf("en" to "English", "ja" to "日本語", "zh" to "中文"),
                settings.optString("locale").ifEmpty { "en" },
            ) { set("locale", it) }
            IconHeading(t("School content language"), Icons.AutoMirrored.Filled.MenuBook, Theme.colors.accent, Modifier.padding(top = 6.dp))
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
internal fun Choices(options: List<Pair<String, String>>, selected: String, choose: (String) -> Unit) {
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
