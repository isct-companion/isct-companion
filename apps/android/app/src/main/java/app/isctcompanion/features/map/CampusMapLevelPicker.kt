package app.isctcompanion.features.map

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.isctcompanion.R
import app.isctcompanion.core.CampusMapLevel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import app.isctcompanion.design.tabular

/** Floors top floor first. A dot marks the floor the selected room is on,
 * the entrance icon every floor the building has an entrance on, in the
 * accent on the street floor (the main entrance's) and muted on the rest. A
 * tall building scrolls within the space the page leaves, keeping the floor
 * shown in view. A floor with no plan (the stack lists the ones between two
 * that have one) is greyed and cannot be picked. */
@Composable
internal fun LevelPicker(
    levels: List<CampusMapLevel>,
    selected: String?,
    marked: String?,
    modifier: Modifier = Modifier,
    pick: (String) -> Unit,
) {
    val scroll = rememberScrollState()
    val cell = with(LocalDensity.current) { Control.roundToPx() }
    LaunchedEffect(selected, levels.size) {
        val index = levels.indexOfFirst { it.id == selected }
        if (index >= 0) scroll.animateScrollTo((index * cell - (scroll.viewportSize - cell) / 2).coerceAtLeast(0))
    }
    Column(modifier.floating(RoundedCornerShape(Theme.radiusChip)).verticalScroll(scroll)) {
        for (level in levels) {
            val isSelected = level.id == selected
            val spoken = t("campusMap.level", "name" to level.name)
            val state = listOfNotNull(
                t("campusMap.destinationFloor").takeIf { level.id == marked },
                t("campusMap.streetFloor").takeIf { level.street },
                t("campusMap.entrance").takeIf { level.entered && !level.street },
            ).joinToString(", ")
            val mark = if (isSelected) Theme.colors.accentInk else Theme.colors.accent
            Box(
                Modifier
                    .size(Control)
                    .background(if (isSelected) Theme.colors.accent else Color.Transparent)
                    .clickable(enabled = level.hasPlan, role = Role.Button) { pick(level.id) }
                    .semantics {
                        contentDescription = spoken
                        stateDescription = state
                        this.selected = isSelected
                    },
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    level.name,
                    style = MaterialTheme.typography.bodySmall.tabular(), fontWeight = FontWeight.SemiBold,
                    color = when {
                        isSelected -> Theme.colors.accentInk
                        level.hasPlan -> Theme.colors.ink
                        else -> Theme.colors.muted.copy(alpha = 0.5f)
                    },
                    maxLines = 1,
                )
                if (level.id == marked) {
                    Box(
                        Modifier
                            .align(Alignment.TopEnd)
                            .padding(5.dp)
                            .size(6.dp)
                            .clip(CircleShape)
                            .background(mark),
                    )
                }
                if (level.street || level.entered) {
                    Icon(
                        painterResource(R.drawable.campus_map_entrance), null,
                        tint = if (level.street) mark else if (isSelected) Theme.colors.accentInk else Theme.colors.muted,
                        modifier = Modifier.align(Alignment.BottomStart).padding(3.dp).size(11.dp),
                    )
                }
            }
        }
    }
}
