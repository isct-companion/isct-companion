package app.isctcompanion.features.settings

import androidx.compose.material.icons.rounded.Map
import app.isctcompanion.design.t
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import app.isctcompanion.LocalModel
import app.isctcompanion.design.Card
import app.isctcompanion.design.Theme
import kotlinx.coroutines.launch

/** Which maps app the campus map's "Open in Maps" links open in. Core builds
 * the link from this device setting, which on Android reads as Google Maps
 * until the person chooses. */
@Composable
internal fun MapCard() {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    Card(Modifier.padding(top = Theme.space8)) {
        Column(Modifier.padding(Theme.space8), verticalArrangement = Arrangement.spacedBy(Theme.space5)) {
            IconHeading(t("settings.main.openPlacesIn"), Icons.Rounded.Map, Theme.colors.ok)
            Choices(
                listOf("google" to t("settings.main.googleMaps"), "apple" to t("settings.main.appleMaps")),
                model.records.settings.optString("mapsApp").ifEmpty { "google" },
            ) { value -> scope.launch { model.updateSettings { it.put("mapsApp", value) } } }
        }
    }
}
