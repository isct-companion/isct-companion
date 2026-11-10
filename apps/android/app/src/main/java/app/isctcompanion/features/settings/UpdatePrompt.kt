package app.isctcompanion.features.settings

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import app.isctcompanion.LocalModel
import app.isctcompanion.design.t
import kotlinx.coroutines.launch

/**
 * The update prompt, as on the desktop and iOS: shown when Core offers the
 * release a check of its own found. Skipping the version and turning the
 * checks off are this device's settings; Settings › About turns them on again.
 */
@Composable
fun UpdatePrompt() {
    val model = LocalModel.current
    val updater = model.updater
    val release = updater.offer ?: return
    val scope = rememberCoroutineScope()
    fun answer(edit: ((org.json.JSONObject) -> Unit)?) {
        updater.answered()
        if (edit != null) scope.launch { model.updateSettings(edit) }
    }
    AlertDialog(
        onDismissRequest = { answer(null) },
        title = { Text(t("appUpdate.prompt.title")) },
        text = { Text(t("appUpdate.prompt.available", "version" to release.version)) },
        confirmButton = {
            TextButton(onClick = {
                answer(null)
                updater.install()
            }) { Text(t("appUpdate.prompt.update")) }
        },
        dismissButton = {
            TextButton(onClick = { answer { it.put("appUpdateChecks", false) } }) {
                Text(t("appUpdate.prompt.turnOff"))
            }
            TextButton(onClick = { answer { it.put("skippedAppVersion", release.version) } }) {
                Text(t("appUpdate.prompt.skip"))
            }
        },
    )
}
