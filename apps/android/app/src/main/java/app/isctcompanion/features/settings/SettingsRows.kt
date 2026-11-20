package app.isctcompanion.features.settings

import androidx.compose.foundation.background
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.UnfoldMore
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.isctcompanion.design.Theme
import androidx.compose.material3.Switch

/** A setting with a switch and optional supporting text. */
@Composable
internal fun SwitchRow(
    title: String,
    checked: Boolean,
    detail: String? = null,
    enabled: Boolean = true,
    detailColor: Color = Theme.colors.muted,
    compact: Boolean = true,
    onChange: (Boolean) -> Unit,
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = if (compact) Arrangement.Start else Arrangement.spacedBy(Theme.rowGap),
        modifier = Modifier.fillMaxWidth().padding(horizontal = Theme.rowX, vertical = if (compact) 8.dp else Theme.rowY),
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, color = Theme.colors.ink)
            detail?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = detailColor) }
        }
        Switch(checked, onChange, enabled = enabled)
    }
}

/** A labelled choice that opens a menu of `options`, the Form picker row. */
@Composable
internal fun <T> PickerRow(title: String, selected: T, options: List<Pair<T, String>>, choose: (T) -> Unit) {
    var menu by remember { mutableStateOf(false) }
    Box {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(Theme.space4),
            modifier = Modifier.fillMaxWidth().clickable { menu = true }.padding(horizontal = Theme.rowX, vertical = Theme.rowY),
        ) {
            Text(title, color = Theme.colors.ink, modifier = Modifier.weight(1f))
            Text(options.firstOrNull { it.first == selected }?.second.orEmpty(), color = Theme.colors.muted)
            Icon(Icons.Rounded.UnfoldMore, null, tint = Theme.colors.muted, modifier = Modifier.size(18.dp))
        }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }, modifier = Modifier.align(Alignment.TopEnd)) {
            for ((value, label) in options) {
                DropdownMenuItem(
                    text = { Text(label) },
                    trailingIcon = { if (value == selected) Icon(Icons.Rounded.Check, null, tint = Theme.colors.accent) },
                    onClick = {
                        menu = false
                        if (value != selected) choose(value)
                    },
                )
            }
        }
    }
}

/** The system settings' row icon, as iOS draws it: a white symbol on a
 * rounded square in a token role (accent for preferences, ok, warn, danger),
 * never a rainbow of colours. */
@Composable
internal fun SettingsIcon(icon: ImageVector, tint: Color) {
    Box(
        Modifier.size(SETTINGS_ICON).clip(RoundedCornerShape(7.dp))
            .background(Brush.verticalGradient(listOf(tint.copy(alpha = 0.82f).compositeOver(Color.White), tint))),
        contentAlignment = Alignment.Center,
    ) { Icon(icon, null, tint = Color.White, modifier = Modifier.size(18.dp)) }
}

internal val SETTINGS_ICON = 29.dp

/** Where a divider starts under a row with a settings icon: at its text. */
internal val ICON_ROW_INSET = Theme.rowX + SETTINGS_ICON + Theme.rowGap

/** A row that opens a page: title, its current value, a chevron. */
@Composable
internal fun NavRow(title: String, detail: String?, icon: ImageVector? = null, tint: Color = Theme.colors.accent, onClick: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Theme.rowGap),
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = Theme.rowX, vertical = Theme.rowY),
    ) {
        icon?.let { SettingsIcon(it, tint) }
        Column(Modifier.weight(1f)) {
            Text(title, color = Theme.colors.ink)
            detail?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted, maxLines = 2) }
        }
        Icon(Icons.Rounded.ChevronRight, null, tint = Theme.colors.muted)
    }
}
