package app.isctcompanion.features.settings

import androidx.compose.material.icons.automirrored.rounded.MenuBook
import androidx.compose.material.icons.rounded.Translate
import app.isctcompanion.design.t
import androidx.compose.foundation.background
import app.isct.design.IsctThemes
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.Role
import androidx.compose.material.icons.rounded.Palette
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.design.Card
import app.isctcompanion.design.Theme
import kotlinx.coroutines.launch

/** The background theme: five round swatches, each its light background
 * with a surface dot; choosing one saves `settings.theme` at once and syncs
 * (slotledger `approach/neutral-themes`). */
@Composable
internal fun ThemeCard() {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    val selected = model.records.settings.optString("theme").ifEmpty { "warm" }
    val themes = listOf("warm" to t("settings.main.warm"), "cool" to t("settings.main.cool"), "paper" to t("settings.main.paper"), "mist" to t("settings.main.mist"), "moss" to t("settings.main.moss"))
    Card(Modifier.padding(top = Theme.space8)) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.fillMaxWidth().padding(horizontal = Theme.rowX, vertical = 12.dp),
        ) {
            IconHeading(t("settings.main.theme"), Icons.Rounded.Palette, Theme.colors.accent, Modifier.weight(1f))
            Row(Modifier.selectableGroup(), horizontalArrangement = Arrangement.spacedBy(Theme.space4)) {
                for ((id, label) in themes) {
                    val neutrals = (IsctThemes[id] ?: continue).first
                    val chosen = id == selected
                    Box(
                        Modifier
                            .size(32.dp)
                            .border(2.dp, if (chosen) Theme.colors.accent else Color.Transparent, CircleShape)
                            .padding(Theme.space1)
                            .clip(CircleShape)
                            .background(neutrals.bg)
                            .border(1.dp, neutrals.line, CircleShape)
                            .selectable(chosen, role = Role.RadioButton) {
                                if (!chosen) scope.launch { model.updateSettings { it.put("theme", id) } }
                            }
                            .semantics { contentDescription = label },
                        contentAlignment = Alignment.Center,
                    ) {
                        Box(Modifier.size(10.dp).clip(CircleShape).background(neutrals.surface))
                    }
                }
            }
        }
    }
}

/** The desktop's language rows: the UI language and the language of school
 * text. Both are workspace settings, so they follow the person to desktop. */
@Composable
internal fun LanguageCard() {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    val settings = model.records.settings
    fun set(key: String, value: String) = scope.launch { model.updateSettings { it.put(key, value) } }
    Card(Modifier.padding(top = Theme.space8)) {
        Column(Modifier.padding(Theme.space8), verticalArrangement = Arrangement.spacedBy(Theme.space5)) {
            IconHeading(t("common.language"), Icons.Rounded.Translate, Theme.colors.accent)
            Choices(
                listOf("en" to "English", "ja" to "日本語", "zh" to "中文"),
                settings.optString("locale").ifEmpty { "en" },
            ) { set("locale", it) }
            IconHeading(t("settings.main.schoolContentLanguage"), Icons.AutoMirrored.Rounded.MenuBook, Theme.colors.accent, Modifier.padding(top = Theme.space3))
            Choices(
                listOf("auto" to t("common.automatic"), "en" to "English", "ja" to "日本語"),
                settings.optString("contentLanguage").ifEmpty { "auto" },
            ) { set("contentLanguage", it) }
            Text(
                t("settings.main.courseNamesRoomsAndSyllabus"),
                style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted,
            )
            SwitchRow(
                t("settings.appearance.showOneLanguageInBilingual"),
                settings.optBoolean("lmsBilingualFilter"),
            ) { enabled ->
                scope.launch { model.updateSettings { it.put("lmsBilingualFilter", enabled) } }
            }
        }
    }
}
