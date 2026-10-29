package app.isctcompanion.features.settings

import android.text.format.Formatter
import android.widget.Toast
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.ProcessLifecycleOwner
import androidx.lifecycle.lifecycleScope
import app.isctcompanion.LocalModel
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.ResetRequest
import app.isctcompanion.design.Card
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import kotlinx.coroutines.launch

/** Core's two reset scopes, as the pages that confirm them. */
internal enum class ResetKind { SchoolCaches, Everything }

/** What every reset removes; the item keys live under `settings.reset.item`. */
private val SCHOOL_CACHES = listOf("settings.reset.item.schoolData", "settings.reset.item.catalog", "settings.reset.item.references", "settings.reset.item.syncRecords", "settings.reset.item.lmsToken")

/** Settings → Reset, the desktop's `ResetSettings` as a page: clear the
 * school caches and read them again, or return this device to its
 * first-launch state. It lists what goes and what stays, with Core's
 * estimate of the space freed; a full reset needs the acknowledgement and a
 * confirmation dialog. Core decides what each scope removes. */
@Composable
internal fun ResetPage(kind: ResetKind, onBusy: (Boolean) -> Unit, done: () -> Unit) {
    val model = LocalModel.current
    val context = LocalContext.current
    // Not the page's scope: switching tabs drops the page, and a reset or
    // restore must not be cancelled halfway.
    val scope = ProcessLifecycleOwner.get().lifecycleScope
    val everything = kind == ResetKind.Everything
    var signOutWeb by remember { mutableStateOf(false) }
    var deleteDownloads by remember { mutableStateOf(false) }
    var acknowledged by remember { mutableStateOf(false) }
    var confirming by remember { mutableStateOf(false) }
    var resetting by remember { mutableStateOf(false) }
    LaunchedEffect(resetting) { onBusy(resetting) }
    DisposableEffect(Unit) { onDispose { onBusy(false) } }
    // Read before the reset, which stops cloud sync.
    val cloudOn = model.cloud?.providers.orEmpty().filter { it.enabled }.map { it.provider }
    var restore by remember { mutableStateOf(true) }
    /** `null` while estimating; an empty text when Core cannot tell. */
    var freed by remember { mutableStateOf<String?>(null) }
    // Signing out frees no space, so only the downloads option re-estimates.
    LaunchedEffect(deleteDownloads) {
        freed = null
        val preview = model.resetPreview(
            if (everything) ResetRequest.Everything else ResetRequest.SchoolCaches(false, deleteDownloads),
        )
        freed = preview?.let { Formatter.formatShortFileSize(context, it.freedBytes.toLong()) } ?: "–"
    }
    val deletes = if (everything) {
        SCHOOL_CACHES + listOf("settings.reset.item.webSession", "settings.reset.item.userData", "settings.reset.item.settings", "settings.reset.item.logins")
    } else {
        SCHOOL_CACHES + listOfNotNull("settings.reset.item.webSession".takeIf { signOutWeb }, "settings.reset.item.downloads".takeIf { deleteDownloads })
    }
    val keeps = if (everything) {
        listOf("settings.reset.item.downloads", "settings.reset.item.cloud", "settings.reset.item.logs")
    } else {
        listOf("settings.reset.item.userData", "settings.reset.item.settings", "settings.reset.item.logins") +
            listOfNotNull("settings.reset.item.webSession".takeIf { !signOutWeb }, "settings.reset.item.downloads".takeIf { !deleteDownloads })
    }
    val confirmLabel = t(if (everything) "settings.reset.full.confirm" else "settings.reset.caches.confirm")
    fun reset() = scope.launch {
        val restoring = if (everything && restore) cloudOn else emptyList()
        resetting = true
        val message = try {
            model.reset(if (everything) ResetRequest.Everything else ResetRequest.SchoolCaches(signOutWeb, deleteDownloads))
            if (restoring.isEmpty() || runCatching { model.restoreFromCloud(restoring) }.isSuccess) {
                t(if (everything) "settings.reset.full.done" else "settings.reset.caches.done")
            } else {
                t("settings.reset.full.restoreFailed")
            }
        } catch (error: Exception) {
            AppModel.describe(error).ifEmpty { t("settings.reset.failed") }.also { resetting = false }
        }
        Toast.makeText(context, message, Toast.LENGTH_LONG).show()
        if (resetting) done()
    }

    Page {
        Footer(t(if (everything) "settings.reset.full.hint" else "settings.reset.caches.hint"))
        if (!everything) {
            Card {
                OptionRow(t("settings.reset.caches.signOut"), t("settings.reset.caches.signOutHint"), signOutWeb, !resetting) {
                    signOutWeb = it
                }
                RowDivider(inset = 16.dp)
                OptionRow(t("settings.reset.caches.downloads"), t("settings.reset.caches.downloadsHint"), deleteDownloads, !resetting) {
                    deleteDownloads = it
                }
            }
        }
        Items(t("settings.reset.deletes"), deletes, Icons.Rounded.Delete)
        Items(t("settings.reset.keeps"), keeps, Icons.Rounded.Check)
        Card {
            Row(Modifier.fillMaxWidth().padding(16.dp)) {
                Text(t("settings.reset.freed"), color = Theme.colors.ink, modifier = Modifier.weight(1f))
                Text(freed ?: t("settings.reset.estimating"), color = Theme.colors.muted)
            }
        }
        if (everything && cloudOn.isNotEmpty()) {
            SectionLabel(t("settings.reset.full.cloudQuestion"))
            Card {
                ChoiceRow(t("settings.reset.full.restore"), t("settings.reset.full.restoreHint"), restore, !resetting) {
                    restore = true
                }
                RowDivider(inset = 16.dp)
                ChoiceRow(t("settings.reset.full.newDevice"), t("settings.reset.full.newDeviceHint"), !restore, !resetting) {
                    restore = false
                }
            }
        }
        if (everything) {
            Card {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.fillMaxWidth().clickable(enabled = !resetting) { acknowledged = !acknowledged }
                        .padding(horizontal = 8.dp, vertical = 8.dp),
                ) {
                    Checkbox(acknowledged, { acknowledged = it }, enabled = !resetting)
                    Text(t("settings.reset.full.acknowledge"), color = Theme.colors.ink)
                }
            }
        }
        Button(
            onClick = { if (everything) confirming = true else reset() },
            enabled = !resetting && (!everything || acknowledged),
            colors = ButtonDefaults.buttonColors(containerColor = Theme.colors.danger),
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
        ) { Text(if (resetting) t("settings.reset.resetting") else confirmLabel) }
    }
    if (confirming) {
        AlertDialog(
            onDismissRequest = { confirming = false },
            title = { Text(t("settings.reset.full.title")) },
            text = { Text(t("settings.reset.full.acknowledge")) },
            confirmButton = {
                TextButton(onClick = {
                    confirming = false
                    reset()
                }) { Text(confirmLabel, color = Theme.colors.danger) }
            },
            dismissButton = { TextButton(onClick = { confirming = false }) { Text(t("settings.reset.cancel")) } },
        )
    }
}

@Composable
private fun OptionRow(title: String, hint: String, checked: Boolean, enabled: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp),
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, color = Theme.colors.ink)
            Text(hint, style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
        }
        Switch(checked, onChange, enabled = enabled)
    }
}

@Composable
private fun ChoiceRow(title: String, hint: String, selected: Boolean, enabled: Boolean, onClick: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        modifier = Modifier.fillMaxWidth().clickable(enabled = enabled, onClick = onClick)
            .padding(horizontal = 8.dp, vertical = 8.dp),
    ) {
        RadioButton(selected, onClick, enabled = enabled)
        Column(Modifier.weight(1f)) {
            Text(title, color = Theme.colors.ink)
            Text(hint, style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
        }
    }
}

@Composable
private fun Items(title: String, keys: List<String>, icon: ImageVector) {
    SectionLabel(title)
    Card {
        keys.forEachIndexed { index, key ->
            if (index > 0) RowDivider(inset = 48.dp)
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(16.dp),
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
            ) {
                Icon(icon, null, tint = Theme.colors.muted)
                Text(t(key), color = Theme.colors.ink)
            }
        }
    }
}
