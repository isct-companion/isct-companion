package app.isctcompanion.features.settings

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import app.isctcompanion.LocalModel
import app.isctcompanion.design.t
import kotlinx.coroutines.launch

/**
 * The update prompt, as on iOS: "IsctCompanion x.y.z 已发布" with 稍后, 不再显示此版本
 * and the update itself as the filled, preferred button. Shown when Core
 * offers the release a check of its own found; turning the checks off is in
 * Settings › About.
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
        title = { Text(t("appUpdate.prompt.available", "version" to release.version)) },
        confirmButton = {
            Button(onClick = {
                answer(null)
                updater.install()
            }) { Text(t("appUpdate.prompt.update")) }
        },
        dismissButton = {
            TextButton(onClick = { answer(null) }) { Text(t("common.later")) }
            TextButton(onClick = { answer { it.put("skippedAppVersion", release.version) } }) {
                Text(t("appUpdate.prompt.skip"))
            }
        },
    )
}
