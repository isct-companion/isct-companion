package app.isctcompanion.features.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import app.isctcompanion.LocalModel
import app.isctcompanion.design.Card
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import kotlinx.coroutines.launch
import org.json.JSONArray

/** The desktop's multi-select of assignment reminder times: one to eight of
 * the offered leads, latest first. */
@Composable
internal fun AssignmentLeadsPage() {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    val options = ReminderLeads.assignmentOptions(model.records.settings.optJSONObject("notifications")) ?: return
    val chosen = options.selected.map { it.toInt() }
    Page {
        Card {
            options.presets.map { it.toInt() }.forEachIndexed { index, minutes ->
                if (index > 0) RowDivider(inset = Theme.rowX)
                val on = minutes in chosen
                val enabled = if (on) chosen.size.toUInt() > options.minSelected else chosen.size.toUInt() < options.maxSelected
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier
                        .fillMaxWidth()
                        .clickable(enabled = enabled) {
                            val next = if (on) chosen - minutes else chosen + minutes
                            scope.launch { model.setNotifications { it.put("assignmentLeadMinutes", JSONArray(next)) } }
                        }
                        .padding(horizontal = Theme.rowX, vertical = Theme.rowY),
                ) {
                    Text(
                        ReminderLeads.label(minutes), color = if (enabled || on) Theme.colors.ink else Theme.colors.muted,
                        modifier = Modifier.weight(1f),
                    )
                    if (on) Icon(Icons.Rounded.Check, null, tint = Theme.colors.accent)
                }
            }
        }
        Footer(t("settings.main.remindsYouAtEveryTime", "max" to options.maxSelected.toInt()))
    }
}
