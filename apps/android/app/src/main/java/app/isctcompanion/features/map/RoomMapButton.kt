package app.isctcompanion.features.map

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.design.t
import app.isctcompanion.core.campusMapHas

/** Opens the map for a room string, or with none to browse it. Nothing
 * shows until Core can place the string (with none, until a map file is
 * stored). */
@Composable
fun RoomMapButton(room: String?, modifier: Modifier = Modifier, label: @Composable () -> Unit) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    var placed by remember(room) { mutableStateOf(false) }
    // Again once the map's file lands (the reference source's update) or the
    // data changes.
    LaunchedEffect(room, model.contentRevision, model.syncSource("references")?.lastUpdated) {
        placed = model.campusMapHas(room)
    }
    if (placed) {
        val spoken = if (room == null) t("campusMap.browse") else t("campusMap.open")
        Box(
            modifier
                .clickable(role = Role.Button) { navigator.openMap(room) }
                .semantics { contentDescription = spoken },
        ) { label() }
    }
}
