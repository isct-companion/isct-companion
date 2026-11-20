package app.isctcompanion.features.settings

import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import app.isctcompanion.core.CloudProvider
import app.isctcompanion.core.Direction
import app.isctcompanion.core.ProviderState
import app.isctcompanion.design.Card
import app.isctcompanion.design.Format
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** バックアップ: the desktop's Backup settings on Android. Google Drive
 * carries the same record every platform writes, so any device on the same
 * Google account loads the others' data; a backup file moves data between
 * any two devices. Core owns every rule; this renders its journal and
 * forwards the person's choices. */
@Composable
internal fun BackupPage() {
    val model = LocalModel.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }
    var confirm by remember { mutableStateOf<Pair<String, suspend () -> Unit>?>(null) }
    val state = model.backup.state
    val locked = busy || state?.syncing == true
    fun toast(message: String) = Toast.makeText(context, message, Toast.LENGTH_LONG).show()
    fun run(work: suspend () -> Unit) {
        scope.launch {
            busy = true
            try { work() } finally { busy = false }
        }
    }
    LaunchedEffect(Unit) { model.backup.refresh() }

    val save = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/json")) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult
        run {
            val text = model.backup.export() ?: return@run
            runCatching {
                withContext(Dispatchers.IO) {
                    val stream = context.contentResolver.openOutputStream(uri) ?: throw IOException("No output stream")
                    stream.use { it.write(text.toByteArray()) }
                }
            }.onFailure { toast(t("settings.main.exportFailedReason", "reason" to AppModel.describe(it))) }
        }
    }
    val open = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult
        confirm = t("settings.backup.restoreThisFileOnAll") to {
            runCatching {
                val text = withContext(Dispatchers.IO) {
                    context.contentResolver.openInputStream(uri)?.use { String(it.readBytes()) }
                        ?: throw IOException("No input stream")
                }
                model.backup.importBackup(text)
            }.onSuccess { toast(t("settings.backup.backupRestored")) }
                .onFailure { toast(t("settings.backup.importFailedReason", "reason" to AppModel.describe(it))) }
        }
    }

    Page {
        SectionLabel(t("settings.backup.sync"))
        Card {
            val providers = state?.providers.orEmpty()
            providers.forEachIndexed { index, provider ->
                if (index > 0) RowDivider(inset = Theme.rowX)
                SwitchRow(
                    title = providerName(provider.provider),
                    checked = provider.enabled,
                    detail = providerDetail(provider),
                    enabled = !locked,
                    detailColor = if (provider.error != null) Theme.colors.danger else Theme.colors.muted,
                    compact = false,
                ) {
                    run { model.backup.setEnabled(provider.provider, it) }
                }
            }
            if (providers.any { it.enabled }) {
                RowDivider(inset = Theme.rowX)
                val automatic = state?.automatic == true
                SwitchRow(
                    t("settings.backup.syncAutomatically"),
                    checked = automatic,
                    detail = if (automatic) t("settings.backup.pullsOtherDevicesChangesWhen")
                        else t("settings.backup.uploadAndPullByHand"),
                    enabled = !locked,
                    compact = false,
                ) { run { model.backup.setAutomatic(it) } }
                RowDivider(inset = Theme.rowX)
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(Theme.space4),
                    modifier = Modifier.fillMaxWidth().padding(horizontal = Theme.space2),
                ) {
                    TextButton(onClick = { run { model.backup.sync(Direction.UPLOAD) } }, enabled = !locked) { Text(t("settings.backup.upload")) }
                    Spacer(Modifier.weight(1f))
                    if (state?.syncing == true) {
                        CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                        Text(t("settings.backup.syncing"), color = Theme.colors.muted)
                        Spacer(Modifier.weight(1f))
                    }
                    TextButton(onClick = { run { model.backup.sync(Direction.PULL) } }, enabled = !locked) { Text(t("settings.backup.pull")) }
                }
            }
        }
        // As on the desktop: with Drive on, school logins can follow in an
        // encrypted file (Android has no iCloud Keychain).
        if (state?.providers.orEmpty().any { it.provider == CloudProvider.GOOGLE_DRIVE && it.enabled }) {
            Spacer(Modifier.height(Theme.space4))
            BackupCredentials(::toast)
        }
        val versions = state?.versions.orEmpty().sortedByDescending { it.savedAt }
        if (versions.isNotEmpty()) {
            Spacer(Modifier.height(Theme.space4))
            SectionLabel(t("settings.backup.earlierVersions"))
            Card {
                versions.forEachIndexed { index, version ->
                    if (index > 0) RowDivider(inset = Theme.rowX)
                    RestoreRow(Format.due(version.savedAt), version.device, !locked) {
                        confirm = t(
                            "settings.backup.restoreTheVersionFromDate",
                            "date" to Format.due(version.savedAt),
                        ) to {
                            model.backup.restore(version.id)
                            toast(t("settings.backup.restoredTheEarlierVersion"))
                        }
                    }
                }
            }
            Footer(t("settings.backup.historyKeepsHourlyThenDaily"))
        }
        Spacer(Modifier.height(Theme.space4))
        SectionLabel(t("settings.backup.backupFile"))
        Card {
            Action(t("settings.backup.export"), !busy) { save.launch("IsctCompanion.json") }
            RowDivider(inset = Theme.rowX)
            Action(t("settings.backup.import"), !busy) { open.launch(arrayOf("application/json")) }
        }
    }

    confirm?.let { (message, action) ->
        AlertDialog(
            onDismissRequest = { confirm = null },
            text = { Text(message) },
            confirmButton = {
                TextButton(onClick = {
                    confirm = null
                    run { action() }
                }) { Text(t("common.restore"), color = Theme.colors.danger) }
            },
            dismissButton = { TextButton(onClick = { confirm = null }) { Text(t("common.cancel")) } },
        )
    }
}

internal fun providerName(provider: CloudProvider) = when (provider) {
    CloudProvider.GOOGLE_DRIVE -> "Google Drive"
    CloudProvider.ICLOUD -> "iCloud"
}

/** The provider's own reason (signed out, declined consent, offline): without
 * it every failure looks the same. A declined sign-in leaves the switch off. */
private fun providerDetail(provider: ProviderState): String? {
    provider.error?.let {
        return t("settings.backup.couldNotUpdateThisCloud") + "\n" + it
    }
    if (!provider.enabled) return null
    return provider.lastSyncedAt?.let { t("settings.backup.syncedDate", "date" to Format.due(it)) } ?: t("settings.backup.waitingForTheFirstSync")
}

@Composable
private fun RestoreRow(title: String, detail: String?, enabled: Boolean, onRestore: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().padding(start = Theme.rowX, end = Theme.space2, top = Theme.space2, bottom = Theme.space2),
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, color = Theme.colors.ink)
            detail?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted) }
        }
        // Core refuses a restore while a transfer is unfinished.
        TextButton(onClick = onRestore, enabled = enabled) { Text(t("common.restore")) }
    }
}

@Composable
private fun Action(title: String, enabled: Boolean, onClick: () -> Unit) {
    TextButton(onClick = onClick, enabled = enabled, modifier = Modifier.fillMaxWidth().padding(horizontal = Theme.space2)) {
        Text(title, modifier = Modifier.fillMaxWidth())
    }
}
