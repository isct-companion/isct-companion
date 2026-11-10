package app.isctcompanion.features.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.design.Guide
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import kotlinx.coroutines.launch

/** The first open: the two language choices people did not know were
 * separate, then the way on to the school account. Shown once per device;
 * closing it in any way lets the tips in. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WelcomeSheet(connect: () -> Unit) {
    if (Guide.welcome) return
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val settings = model.records.settings
    fun set(key: String, value: String) = scope.launch { model.updateSettings { it.put(key, value) } }
    fun close(then: () -> Unit = {}) {
        scope.launch { state.hide() }.invokeOnCompletion {
            Guide.closeWelcome()
            then()
        }
    }
    ModalBottomSheet(onDismissRequest = { Guide.closeWelcome() }, sheetState = state, containerColor = Theme.colors.bg) {
        Column(
            Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    t("common.welcomeToIsctCompanion"), style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold,
                    color = Theme.colors.ink, modifier = Modifier.weight(1f),
                )
                TextButton(onClick = { close() }) { Text(t(if (model.needsAccount) "common.later" else "common.done")) }
            }
            SectionLabel(t("common.appLanguage"))
            Choices(
                listOf("en" to "English", "ja" to "日本語", "zh" to "中文"),
                settings.optString("locale").ifEmpty { "en" },
            ) { set("locale", it) }
            SectionLabel(t("guide.courseAndSyllabusLanguage"))
            Choices(
                listOf("auto" to t("common.automatic"), "en" to "English", "ja" to "日本語"),
                settings.optString("contentLanguage").ifEmpty { "auto" },
            ) { set("contentLanguage", it) }
            Footer(t("guide.canDifferFromTheApp"))
            if (model.needsAccount) {
                Button(onClick = { close(connect) }, modifier = Modifier.fillMaxWidth().padding(top = 16.dp)) {
                    Text(t("common.connectSchoolAccounts"), fontWeight = FontWeight.SemiBold)
                }
            }
        }
    }
}
