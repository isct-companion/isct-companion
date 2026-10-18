package app.isctcompanion.features.settings

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
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Notifications
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.rememberModalBottomSheetState
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
}

/** 設定 (sheet from Today's gear): school accounts and reminders. Planning,
 * calendars, folders and developer settings stay on desktop. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsSheet(onDismiss: () -> Unit) {
    var page by remember { mutableStateOf<SettingsPage>(SettingsPage.Root) }
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = Theme.colors.bg,
        dragHandle = null,
    ) {
        BackHandler(page != SettingsPage.Root) { page = SettingsPage.Root }
        Column(Modifier.fillMaxSize().imePadding()) {
            when (val current = page) {
                SettingsPage.Root -> {
                    Bar("設定", close = onDismiss)
                    RootPage { page = it }
                }
                is SettingsPage.Account -> {
                    Bar(current.name, back = { page = SettingsPage.Root })
                    AccountPage(current.profile) { page = SettingsPage.Root }
                }
                SettingsPage.Notifications -> {
                    Bar("通知", back = { page = SettingsPage.Root })
                    NotificationsPage()
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Bar(title: String, back: (() -> Unit)? = null, close: (() -> Unit)? = null) {
    TopAppBar(
        title = { Text(title) },
        navigationIcon = {
            back?.let { IconButton(onClick = it) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, "戻る") } }
        },
        actions = {
            close?.let { IconButton(onClick = it) { Icon(Icons.Outlined.Close, "閉じる") } }
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
        SectionLabel("学校アカウント")
        Card {
            AccountRow("Science Tokyo", model.credentials?.science, model.sessions?.science) {
                open(SettingsPage.Account(SchoolAuthProfile.SCIENCE, "Science Tokyo"))
            }
            RowDivider(inset = 36.dp)
            AccountRow("Tokyo Tech（旧）", model.credentials?.tokyoTech, model.sessions?.tokyoTech) {
                open(SettingsPage.Account(SchoolAuthProfile.TOKYO_TECH, "Tokyo Tech（旧）"))
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
                    Text("通知", color = Theme.colors.ink)
                    Text(notificationSummary(), style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted)
                }
                Icon(Icons.Outlined.ChevronRight, null, tint = Theme.colors.muted)
            }
        }
        Footer("学年暦の編集、コースフォルダ、開発者向け設定はデスクトップ版にあります。")
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
    if (settings?.optBoolean("classes") == true) parts.add("授業 ${settings.optInt("classLeadMinutes", 10)} 分前")
    if (settings?.optBoolean("assignments") == true) parts.add("課題 当日")
    return if (parts.isEmpty()) "オフ" else parts.joinToString(" · ")
}

@Composable
private fun AccountRow(name: String, credential: CredentialSummary?, session: SchoolSessionState?, onClick: () -> Unit) {
    val model = LocalModel.current
    val connected = credential?.availability == CredentialAvailability.AVAILABLE ||
        credential?.availability == CredentialAvailability.SAVED_LOCKED
    val (label, color) = when {
        !connected -> "未接続" to Theme.colors.muted
        session == SchoolSessionState.ACTIVE -> (model.lastPull?.let { "接続済 · ${Format.time(it)} 更新" } ?: "接続済") to Theme.colors.ok
        session == SchoolSessionState.FAILED -> "ログインに失敗しました" to Theme.colors.danger
        session == SchoolSessionState.MAINTENANCE -> "メンテナンス中" to Theme.colors.warn
        else -> "確認中" to Theme.colors.muted
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
            username, { username = it }, label = { Text("学籍番号") }, singleLine = true,
            keyboardOptions = KeyboardOptions(
                capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false, keyboardType = KeyboardType.Ascii,
            ),
            // Compose fields join system Autofill only when they say what they hold.
            modifier = Modifier.fillMaxWidth().semantics { contentType = ContentType.Username },
        )
        Secret(password, { password = it }, if (saved) "パスワード（変更する場合のみ）" else "パスワード", login = true)
        Footer("パスワードはこの端末のキーストアで暗号化して保存され、学校のサイトにだけ送られます。")
        if (profile == SchoolAuthProfile.SCIENCE) {
            SectionLabel("ワンタイムパスワード")
            OtpImportField(summary?.hasOtp == true, otpSecret) { otpSecret = it }
        } else {
            SectionLabel("マトリクス表")
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
            if (saving) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp) else Text("保存して接続")
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
            ) { Text("この端末からログイン情報を削除", color = Theme.colors.danger) }
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
            SwitchRow("授業の前に通知", settings?.optBoolean("classes") == true) { toggle("classes", it) }
            if (settings?.optBoolean("classes") == true) {
                RowDivider(inset = 16.dp)
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("通知のタイミング", color = Theme.colors.ink)
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        val lead = settings.optInt("classLeadMinutes", 10)
                        for (minutes in listOf(5, 10, 15, 30)) {
                            FilterChip(
                                selected = lead == minutes,
                                onClick = { scope.launch { model.setNotifications { it.put("classLeadMinutes", minutes) } } },
                                label = { Text("$minutes 分前") },
                            )
                        }
                    }
                }
            }
            RowDivider(inset = 16.dp)
            SwitchRow("課題の締切当日に通知", settings?.optBoolean("assignments") == true) { toggle("assignments", it) }
        }
        if (denied) Footer("通知が許可されていません。設定アプリの「通知」から許可してください。", Theme.colors.danger)
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
