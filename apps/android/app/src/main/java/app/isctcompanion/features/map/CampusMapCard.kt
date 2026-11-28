package app.isctcompanion.features.map

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Cancel
import androidx.compose.material.icons.rounded.Map
import androidx.compose.material.icons.rounded.PhotoCamera
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.isctcompanion.core.CampusMapCandidate
import app.isctcompanion.core.CampusMapCard
import app.isctcompanion.core.CampusMapCardKind
import app.isctcompanion.core.CampusMapEntranceRow
import app.isctcompanion.core.CampusMapPlace
import app.isctcompanion.design.ActionButton
import app.isctcompanion.design.ButtonLevel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t

/** The place's name with the button that closes the card, under the
 * grabber: a tap on it steps the card between its lowest height and its
 * content's; a drag anywhere here moves the card. */
@Composable
internal fun CardHeader(card: CampusMapCard, folded: Boolean, fold: () -> Unit, close: () -> Unit) {
    val spoken = t(if (folded) "campusMap.unfoldCard" else "campusMap.foldCard")
    Column {
        Box(
            Modifier
                .fillMaxWidth()
                .clickable(role = Role.Button, onClick = fold)
                .semantics { contentDescription = spoken }
                .padding(vertical = Theme.space4),
            contentAlignment = Alignment.Center,
        ) {
            Box(Modifier.size(width = 36.dp, height = 5.dp).clip(CircleShape).background(Theme.colors.muted.copy(alpha = 0.55f)))
        }
        Row(
            Modifier.padding(start = Theme.cardPadding, end = Theme.space2, bottom = Theme.space3),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                card.title,
                style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold, color = Theme.colors.ink,
                maxLines = 2, overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f).semantics { heading() },
            )
            Box(
                Modifier.size(Control).clip(CircleShape).clickable(role = Role.Button, onClick = close),
                contentAlignment = Alignment.Center,
            ) {
                Icon(Icons.Rounded.Cancel, t("campusMap.closeCard"), tint = Theme.colors.muted)
            }
        }
    }
}

/** Details below the place header, while the card is unfolded. */
@Composable
internal fun CardDetails(
    card: CampusMapCard,
    openInMaps: () -> Unit,
    select: (CampusMapPlace) -> Unit,
    choose: (Int) -> Unit,
    photos: FloorPhotoUploads?,
    takePhoto: () -> Unit,
) {
    Column(
        Modifier.padding(start = Theme.cardPadding, end = Theme.cardPadding, bottom = Theme.cardPadding),
        verticalArrangement = Arrangement.spacedBy(Theme.space5),
    ) {
        // `建物 · 大岡山`, `西9号館 · 2F · 大岡山`.
        val subtitle = listOfNotNull(
            when (card.kind) {
                CampusMapCardKind.BUILDING -> t("campusMap.kind.building")
                CampusMapCardKind.ROOM -> card.buildingName.takeIf { it != card.title }
            },
            card.levelName,
            campusName(card.campus),
        )
        Text(
            subtitle.joinToString(" · "),
            style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted,
        )
        if (card.buildingOnly) {
            Text(t("campusMap.buildingOnly"), style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
        } else if (card.noFloorPlans) {
            Text(t("campusMap.noFloorPlans"), style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
        }
        Actions(card, openInMaps, select, photos, takePhoto)
        if (card.entrances.isNotEmpty()) Entrances(card.entrances, select)
        if (card.candidates.isNotEmpty()) Candidates(card.candidates, choose)
    }
}

/** A state the map cannot be drawn in, with at most one way on. */
@Composable
internal fun Notice(title: String, detail: String, actionLabel: String? = null, action: () -> Unit = {}) {
    Column(
        Modifier.padding(start = Theme.cardPadding, end = Theme.cardPadding, bottom = Theme.cardPadding),
        verticalArrangement = Arrangement.spacedBy(Theme.space3),
    ) {
        Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
        Text(detail, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted)
        if (actionLabel != null) ActionButton(actionLabel, onClick = action, level = ButtonLevel.Secondary)
    }
}

/** The building (on a room's card) and the maps app: the
 * card moves the map, and only this button leaves the app. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Actions(
    card: CampusMapCard,
    openInMaps: () -> Unit,
    select: (CampusMapPlace) -> Unit,
    photos: FloorPhotoUploads?,
    takePhoto: () -> Unit,
) {
    FlowRow(
        horizontalArrangement = Arrangement.spacedBy(Theme.space4),
        verticalArrangement = Arrangement.spacedBy(Theme.space3),
    ) {
        // Each tap opens the camera; the label tells how the photos went.
        if (photos != null) {
            ActionButton(
                when {
                    photos.sending > 0 -> t("campusMap.photos.sending")
                    photos.sent > 0 -> t("campusMap.photos.sent", "count" to photos.sent.toString())
                    else -> t("campusMap.photos.upload")
                },
                onClick = takePhoto,
                level = ButtonLevel.Secondary,
                maxLines = Int.MAX_VALUE,
                icon = Icons.Rounded.PhotoCamera,
            )
        }
        card.building?.let { building ->
            ActionButton(
                card.buildingName,
                onClick = { select(building) },
                modifier = Modifier.semantics { contentDescription = t("campusMap.showBuilding", "name" to card.buildingName) },
                level = ButtonLevel.Secondary,
                maxLines = Int.MAX_VALUE,
            )
        }
        ActionButton(
            t("campusMap.openInMaps"),
            onClick = openInMaps,
            level = ButtonLevel.Secondary,
            maxLines = Int.MAX_VALUE,
            icon = Icons.Rounded.Map,
        )
    }
}

/** Each entrance lights on the map, on its floor, in this same card;
 * "Open in Maps" then pins it. */
@Composable
private fun Entrances(rows: List<CampusMapEntranceRow>, select: (CampusMapPlace) -> Unit) {
    Column {
        Text(
            t("campusMap.entrances"),
            style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.SemiBold, color = Theme.colors.muted,
        )
        rows.forEach { row ->
            Row(
                Modifier.fillMaxWidth().heightIn(min = 48.dp)
                    .clickable { select(CampusMapPlace.Entrance(row.entranceId)) },
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    row.name ?: t("campusMap.entrance"),
                    Modifier.weight(1f),
                    style = MaterialTheme.typography.bodyLarge,
                    fontWeight = if (row.selected) FontWeight.SemiBold else null,
                    color = if (row.selected) Theme.colors.accent else Theme.colors.ink,
                    maxLines = 1,
                )
                row.levelName?.let {
                    Text(it, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted)
                }
            }
        }
    }
}

/** Every room an ambiguous spelling names; one is shown at a time. */
@Composable
private fun Candidates(candidates: List<CampusMapCandidate>, choose: (Int) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(Theme.space3)) {
        Text(
            t("campusMap.chooseRoom"),
            style = MaterialTheme.typography.bodySmall, fontWeight = FontWeight.SemiBold, color = Theme.colors.muted,
        )
        Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(Theme.space4)) {
            candidates.forEachIndexed { index, candidate ->
                Text(
                    "${candidate.label} · ${candidate.buildingName}",
                    style = MaterialTheme.typography.bodySmall, fontWeight = FontWeight.SemiBold, maxLines = 1,
                    color = if (candidate.chosen) Theme.colors.accentInk else Theme.colors.accent,
                    modifier = Modifier
                        .clip(CircleShape)
                        .background(if (candidate.chosen) Theme.colors.accent else Theme.colors.accentSoft)
                        .clickable(role = Role.Button) { choose(index) }
                        .semantics { selected = candidate.chosen }
                        .padding(horizontal = Theme.space5, vertical = Theme.space4),
                )
            }
        }
    }
}
