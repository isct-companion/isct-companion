package app.isctcompanion.features.map

import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.input.TextFieldLineLimits
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.foundation.text.input.clearText
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Cancel
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.isctcompanion.core.CampusMapSearch
import app.isctcompanion.core.CampusMapSearchResult
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t

@Composable
internal fun campusName(campus: String): String = when (campus) {
    "suzukakedai" -> t("campusMap.suzukakedai")
    else -> t("campusMap.ookayama")
}

/** A capsule floating over the map, as iOS draws it: no inner field box. */
@Composable
internal fun SearchBar(
    state: TextFieldState,
    interaction: MutableInteractionSource,
    enabled: Boolean,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier
            .defaultMinSize(minHeight = Control)
            .floating(CircleShape)
            .padding(horizontal = Theme.space7),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Theme.space4),
    ) {
        Icon(Icons.Rounded.Search, null, tint = Theme.colors.muted, modifier = Modifier.size(20.dp))
        BasicTextField(
            state = state,
            enabled = enabled,
            lineLimits = TextFieldLineLimits.SingleLine,
            textStyle = MaterialTheme.typography.bodyLarge.copy(color = Theme.colors.ink),
            cursorBrush = SolidColor(Theme.colors.accent),
            // Room codes are capitals and digits; no word is being spelt.
            keyboardOptions = KeyboardOptions(
                capitalization = KeyboardCapitalization.Characters, autoCorrectEnabled = false, imeAction = ImeAction.Search,
            ),
            interactionSource = interaction,
            modifier = Modifier.weight(1f),
            decorator = { field ->
                Box(contentAlignment = Alignment.CenterStart) {
                    if (state.text.isEmpty()) {
                        Text(
                            t("campusMap.search"), style = MaterialTheme.typography.bodyLarge, color = Theme.colors.muted,
                            maxLines = 1, overflow = TextOverflow.Ellipsis,
                        )
                    }
                    field()
                }
            },
        )
        if (state.text.isNotEmpty()) {
            IconButton(onClick = { state.clearText() }, modifier = Modifier.size(Control)) {
                Icon(Icons.Rounded.Cancel, t("common.clear"), tint = Theme.colors.muted, modifier = Modifier.size(20.dp))
            }
        }
    }
}

/** Nothing for an empty query or before Core answers the text shown. */
@Composable
internal fun Results(query: String, answer: Result<CampusMapSearch>?, pick: (CampusMapSearchResult) -> Unit) {
    val found = answer?.getOrNull()
    when {
        answer != null && found == null -> Notice(t("campusMap.failed"), answer.exceptionOrNull()?.message.orEmpty())
        found == null || found.query != query -> Unit
        found.results.isEmpty() -> Text(
            t("campusMap.noResults"),
            style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted,
            modifier = Modifier.padding(start = Theme.cardPadding, end = Theme.cardPadding, bottom = Theme.cardPadding),
        )
        // About three rows: the map stays in view above the keyboard.
        else -> LazyColumn(Modifier.heightIn(max = 200.dp)) {
            items(found.results) { result -> SearchRow(result) { pick(result) } }
        }
    }
}

/** `大岡山西9号館 · 2F · 大岡山`: the building unless it is the title, the
 * floor, the campus. */
@Composable
private fun SearchRow(result: CampusMapSearchResult, pick: () -> Unit) {
    Column(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = pick)
            .defaultMinSize(minHeight = 56.dp)
            .padding(horizontal = Theme.cardPadding, vertical = Theme.space4),
        verticalArrangement = Arrangement.spacedBy(Theme.space1, Alignment.CenterVertically),
    ) {
        Text(result.title, style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
        val details = listOfNotNull(result.buildingName.takeIf { it != result.title }, result.levelName, campusName(result.campus))
        Text(details.joinToString(" · "), style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
        if (result.noFloorPlan) {
            Text(t("campusMap.notOnFloorPlans"), style = MaterialTheme.typography.bodySmall, color = Theme.colors.warn)
        }
    }
}
