package app.isctcompanion.features.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import app.isctcompanion.LocalModel
import app.isctcompanion.design.Card
import app.isctcompanion.features.schoolstatus.SyncIndicatorView
import app.isctcompanion.features.schoolstatus.SyncSections
import app.isctcompanion.design.Theme
import app.isctcompanion.features.schoolstatus.indicator
import app.isctcompanion.design.t
import kotlinx.coroutines.launch
import org.json.JSONObject

/** データ更新: whether Core's automatic school reads may use cellular data
 * (`settings.subscriptionRefresh.allowCellular`), Core's sync sources and
 * runs, and a manual refresh that asks Core's planner for everything. */
@Composable
internal fun DataUpdatesPage() {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    val cellular = model.records.settings.optJSONObject("subscriptionRefresh")?.optBoolean("allowCellular") == true
    Page {
        Card {
            SwitchRow(t("settings.main.updateOnCellularData"), cellular) { on ->
                scope.launch {
                    model.updateSettings { settings ->
                        val refresh = settings.optJSONObject("subscriptionRefresh") ?: JSONObject()
                        refresh.put("allowCellular", on)
                        settings.put("subscriptionRefresh", refresh)
                    }
                }
            }
        }
        Footer(t("settings.main.whenOffAutomaticUpdatesWait"))
        SyncSections()
        Card(Modifier.padding(top = Theme.space4)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.fillMaxWidth().clickable { model.refreshFromSchool() }.padding(horizontal = Theme.rowX, vertical = Theme.rowY),
            ) {
                Text(t("common.refreshNow"), color = Theme.colors.accent, modifier = Modifier.weight(1f))
                model.syncSnapshot?.indicator?.let { SyncIndicatorView(it) }
            }
        }
    }
}

internal fun dataSummary(settings: JSONObject): String =
    if (settings.optJSONObject("subscriptionRefresh")?.optBoolean("allowCellular") == true) {
        t("settings.main.updateOnCellularData")
    } else {
        t("settings.main.updatesAutomaticallyOnWiFi")
    }
