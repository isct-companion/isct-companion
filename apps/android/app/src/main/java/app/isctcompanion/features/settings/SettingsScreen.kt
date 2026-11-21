package app.isctcompanion.features.settings

import app.isctcompanion.design.BackButton
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.material.icons.rounded.Language
import androidx.compose.material.icons.rounded.FormatSize
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.Download
import androidx.compose.material.icons.rounded.RestartAlt
import androidx.compose.material.icons.rounded.Restore
import androidx.compose.material.icons.rounded.Lightbulb
import androidx.compose.material.icons.rounded.Feedback
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Cloud
import androidx.compose.material.icons.rounded.Sync
import androidx.compose.material.icons.rounded.SystemUpdate
import androidx.compose.material.icons.rounded.School
import androidx.compose.material.icons.rounded.Notifications
import app.isctcompanion.design.t
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SegmentedButton
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.SchoolAuthProfile
import app.isctcompanion.core.SchoolBrowserHost
import app.isctcompanion.core.SyncState
import app.isctcompanion.core.WebRouter
import app.isctcompanion.core.exportApplicationLogs
import app.isctcompanion.core.feedbackFormUrl
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
                Bar(t("common.settings"))
                RootPage { page = it }
            }
            is SettingsPage.Account -> {
                Bar(current.name, back = { page = SettingsPage.Root })
                AccountPage(current.profile) { page = SettingsPage.Root }
            }
            SettingsPage.Notifications -> {
                Bar(t("common.notifications"), back = { page = SettingsPage.Root })
                NotificationsPage { page = SettingsPage.AssignmentLeads }
            }
            SettingsPage.AssignmentLeads -> {
                Bar(t("settings.notification.assignmentReminderTimes"), back = { page = SettingsPage.Notifications })
                AssignmentLeadsPage()
            }
            SettingsPage.StudyProfile -> {
                Bar(t("settings.main.studyProfile"), back = { page = SettingsPage.Root })
                StudyProfilePage()
            }
            SettingsPage.DataUpdates -> {
                Bar(t("settings.main.dataUpdates"), back = { page = SettingsPage.Root })
                DataUpdatesPage()
            }

            SettingsPage.Backup -> {
                Bar(t("settings.main.backup"), back = { page = SettingsPage.Root })
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
            if (back != null) BackButton(back)
        },
        colors = TopAppBarDefaults.topAppBarColors(containerColor = Theme.colors.bg),
    )
}

@Composable
internal fun Page(content: @Composable ColumnScope.() -> Unit) {
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = Theme.gutter).padding(bottom = Theme.space11),
        verticalArrangement = Arrangement.spacedBy(Theme.space4),
        content = content,
    )
}

@Composable
private fun RootPage(open: (SettingsPage) -> Unit) {
    val model = LocalModel.current
    val context = LocalContext.current
    Page {
        SectionLabel(t("settings.main.schoolAccounts"))
        Card {
            val science = accountName(SchoolAuthProfile.SCIENCE)
            AccountRow(
                SchoolAuthProfile.SCIENCE, science, model.credentials?.science, model.sessions?.science,
                model.syncSource("lms")?.lastUpdated,
            ) { open(SettingsPage.Account(SchoolAuthProfile.SCIENCE, science)) }
            RowDivider(inset = 36.dp)
            val tokyoTech = accountName(SchoolAuthProfile.TOKYO_TECH)
            AccountRow(
                SchoolAuthProfile.TOKYO_TECH, tokyoTech, model.credentials?.tokyoTech, model.sessions?.tokyoTech,
                model.syncSource("kyomu")?.lastUpdated,
            ) { open(SettingsPage.Account(SchoolAuthProfile.TOKYO_TECH, tokyoTech)) }
        }
        Card(Modifier.padding(top = Theme.space8)) {
            NavRow(
                t("common.notifications"), ReminderLeads.summary(model.records.settings.optJSONObject("notifications")),
                Icons.Rounded.Notifications, Theme.colors.danger,
            ) { open(SettingsPage.Notifications) }
            RowDivider(inset = ICON_ROW_INSET)
            NavRow(t("settings.main.studyProfile"), null, Icons.Rounded.School) { open(SettingsPage.StudyProfile) }
            RowDivider(inset = ICON_ROW_INSET)
            NavRow(t("settings.main.dataUpdates"), dataSummary(model.records.settings), Icons.Rounded.Sync, Theme.colors.ok) {
                open(SettingsPage.DataUpdates)
            }
            RowDivider(inset = ICON_ROW_INSET)
            NavRow(t("settings.main.backup"), backupSummary(model.backup.state), Icons.Rounded.Cloud) { open(SettingsPage.Backup) }
        }
        ThemeCard()
        LanguageCard()
        // No maps-app choice: Android has no Apple Maps, and Core opens Google Maps.
        WebCard()
        // As on iOS: About holds the version, the tips and the logs, then the
        // desktop note; the destructive Reset group comes last.
        Spacer(Modifier.height(Theme.space4))
        SectionLabel(t("settings.main.about"))
        Card {
            Row(
                Modifier.fillMaxWidth().padding(horizontal = Theme.rowX, vertical = Theme.rowY),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(Theme.rowGap),
            ) {
                SettingsIcon(Icons.Rounded.Info, Theme.colors.muted)
                Text("IsctCompanion", color = Theme.colors.ink, modifier = Modifier.weight(1f))
                Text(appVersion(context), color = Theme.colors.muted)
            }
            RowDivider(inset = ICON_ROW_INSET)
            UpdateRows()
            RowDivider(inset = ICON_ROW_INSET)
            ActionRow(t("settings.main.showWelcomeAndTipsAgain"), Icons.Rounded.Lightbulb) { Guide.reset() }
            RowDivider(inset = ICON_ROW_INSET)
            ActionRow(t("about.feedback"), Icons.Rounded.Feedback) {
                val url = feedbackFormUrl(appVersion(context), device())
                context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
            }
            RowDivider(inset = ICON_ROW_INSET)
            LogsRow()
        }
        Footer(t("settings.main.calendarEditingCourseFoldersAnd"))
        ResetCard(open)
    }
}

/** The desktop's Reset group: each row opens its confirmation page. */
@Composable
private fun ResetCard(open: (SettingsPage) -> Unit) {
    Spacer(Modifier.height(Theme.space4))
    SectionLabel(t("settings.reset.group"))
    Card {
        ResetRow(t("settings.reset.caches.title"), t("settings.reset.caches.hint"), Theme.colors.ink, Icons.Rounded.Restore, Theme.colors.warn) {
            open(SettingsPage.Reset(ResetKind.SchoolCaches))
        }
        RowDivider(inset = ICON_ROW_INSET)
        ResetRow(t("settings.reset.full.title"), t("settings.reset.full.hint"), Theme.colors.danger, Icons.Rounded.RestartAlt, Theme.colors.danger) {
            open(SettingsPage.Reset(ResetKind.Everything))
        }
    }
}

@Composable
private fun ResetRow(title: String, hint: String, color: Color, icon: ImageVector, tint: Color, onClick: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Theme.rowGap),
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = Theme.rowX, vertical = Theme.rowY),
    ) {
        SettingsIcon(icon, tint)
        Column(Modifier.weight(1f)) {
            Text(title, color = color)
            Text(hint, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted)
        }
        Icon(Icons.Rounded.ChevronRight, null, tint = Theme.colors.muted)
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
            }.onFailure { fail(t("settings.main.exportFailedReason", "reason" to AppModel.describe(it))) }
        }
    }
    // One row for both log actions, as a menu (iOS's Application logs row).
    var menu by remember { mutableStateOf(false) }
    Box {
        ActionRow(t("settings.main.applicationLogs"), Icons.Rounded.Description) { menu = true }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            DropdownMenuItem(
                text = { Text(t("settings.main.saveToFile")) },
                leadingIcon = { Icon(Icons.Rounded.Download, null) },
                onClick = {
                    menu = false
                    save.launch("IsctCompanion-logs-${LocalDate.now()}.txt")
                },
            )
            DropdownMenuItem(
                text = { Text(t("settings.schoolAccounts.copy")) },
                leadingIcon = { Icon(Icons.Rounded.ContentCopy, null) },
                onClick = {
                    menu = false
                    // Android shows its own confirmation for every clipboard write.
                    runCatching {
                        context.getSystemService(ClipboardManager::class.java)
                            .setPrimaryClip(ClipData.newPlainText(t("settings.main.applicationLogs"), logText(context)))
                    }.onFailure { fail(t("settings.otp.copyFailed")) }
                },
            )
        }
    }
}

private fun appVersion(context: Context): String =
    runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull().orEmpty()

/** The system this build runs on, as the log and the feedback form name it. */
private fun device() = "Android ${Build.VERSION.RELEASE} · ${Build.MANUFACTURER} ${Build.MODEL}"

private fun logText(context: Context): String =
    exportApplicationLogs("IsctCompanion ${appVersion(context)} · ${device()}")

/** The update check: what it found, or a check now, and whether this device
 * checks on its own (Core reads `appUpdateChecks`; absent is on). */
@Composable
private fun UpdateRows() {
    val model = LocalModel.current
    val updater = model.updater
    val scope = rememberCoroutineScope()
    val release = updater.available
    val title = when {
        release != null -> t("appUpdate.prompt.available", "version" to release.version)
        updater.upToDate -> t("appUpdate.upToDate")
        else -> t("settings.about.checkForUpdates")
    }
    ActionRow(title, Icons.Rounded.SystemUpdate, enabled = !updater.checking && !updater.downloading) {
        if (release != null) updater.install() else scope.launch { model.checkForUpdate(manual = true) }
    }
    RowDivider(inset = ICON_ROW_INSET)
    SwitchRow(t("appUpdate.checkAutomatically"), model.records.settings.optBoolean("appUpdateChecks", true)) { on ->
        scope.launch { model.updateSettings { it.put("appUpdateChecks", on) } }
    }
}

/** An action row; with an icon it reads as a settings row, as on iOS. */
@Composable
private fun ActionRow(title: String, icon: ImageVector? = null, enabled: Boolean = true, onClick: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Theme.rowGap),
        modifier = Modifier.fillMaxWidth().clickable(enabled = enabled, onClick = onClick).padding(horizontal = Theme.rowX, vertical = Theme.rowY),
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
internal fun IconHeading(title: String, icon: ImageVector, tint: Color, modifier: Modifier = Modifier) {
    Row(modifier, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Theme.space6)) {
        SettingsIcon(icon, tint)
        Text(title, color = Theme.colors.ink)
    }
}

@Composable
internal fun Footer(text: String, color: Color = Theme.colors.muted) {
    Text(text, style = MaterialTheme.typography.bodySmall, color = color, modifier = Modifier.padding(horizontal = Theme.space8))
}

/** Off, or when the cloud copy last synced; every connected provider counts,
 * so a Drive-only phone reads the same as an iCloud one. */
private fun backupSummary(cloud: SyncState?): String {
    val enabled = cloud?.providers.orEmpty().filter { it.enabled }
    if (enabled.isEmpty()) return t("settings.main.off")
    val synced = enabled.mapNotNull { it.lastSyncedAt }.maxByOrNull { Format.date(it) ?: Instant.EPOCH }
    return synced?.let { t("settings.backup.syncedDate", "date" to Format.due(it)) } ?: enabled.joinToString(" · ") { providerName(it.provider) }
}

/** The ウェブ tab's device preferences: page text size and the site it opens on. */
@Composable
private fun WebCard() {
    val web = LocalModel.current.web
    Spacer(Modifier.height(Theme.space4))
    SectionLabel(t("common.web"))
    Card {
        Column(Modifier.padding(Theme.space8), verticalArrangement = Arrangement.spacedBy(Theme.space5)) {
            IconHeading(t("web.pageZoom"), Icons.Rounded.FormatSize, Theme.colors.muted)
            Choices(SchoolBrowserHost.textScales.map { "$it" to "$it%" }, "${web.browser.textScale}") {
                web.browser.chooseTextScale(it.toInt())
            }
        }
        RowDivider(inset = ICON_ROW_INSET)
        var picking by remember { mutableStateOf(false) }
        Box {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(Theme.rowGap),
                modifier = Modifier.fillMaxWidth().clickable { picking = true }.padding(horizontal = Theme.rowX, vertical = Theme.rowY),
            ) {
                SettingsIcon(Icons.Rounded.Language, Theme.colors.accent)
                Text(t("settings.main.firstSite"), color = Theme.colors.ink, modifier = Modifier.weight(1f))
                Text(WebRouter.name(web.firstSite), color = Theme.colors.muted)
            }
            DropdownMenu(expanded = picking, onDismissRequest = { picking = false }) {
                for (site in WebRouter.tabSites) {
                    DropdownMenuItem(
                        text = { Text(WebRouter.name(site)) },
                        trailingIcon = { if (site == web.firstSite) Icon(Icons.Rounded.Check, null) },
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
