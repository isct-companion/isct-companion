package app.isctcompanion.design

import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.isct.design.IsctTokens
import app.isct.design.IsctTokensDark

/** Adaptive roles over the generated light/dark tokens (design/tokens). System
 * chrome (navigation bar, top bars, sheets, controls) stays Material's. */
@Immutable
data class Palette(
    val bg: Color,
    val surface: Color,
    val surface2: Color,
    val ink: Color,
    val muted: Color,
    val line: Color,
    val accent: Color,
    val accentSoft: Color,
    val danger: Color,
    val dangerSoft: Color,
    val warn: Color,
    val warnSoft: Color,
    val ok: Color,
    val okSoft: Color,
)

private val light = with(IsctTokens) {
    Palette(
        colorBg, colorSurface, colorSurface2, colorInk, colorMuted, colorLine, colorAccent,
        colorAccentSoft, colorDanger, colorDangerSoft, colorWarn, colorWarnSoft, colorOk, colorOkSoft,
    )
}

private val dark = with(IsctTokensDark) {
    Palette(
        colorBg, colorSurface, colorSurface2, colorInk, colorMuted, colorLine, colorAccent,
        colorAccentSoft, colorDanger, colorDangerSoft, colorWarn, colorWarnSoft, colorOk, colorOkSoft,
    )
}

private val LocalPalette = staticCompositionLocalOf { light }

object Theme {
    val colors: Palette
        @Composable get() = LocalPalette.current

    val radiusSurface: Dp = IsctTokens.radiusSurface
    val radiusPanel: Dp = IsctTokens.radiusPanel
    val radiusChip: Dp = IsctTokens.radiusChip

    /** Course colour strengths: a block that occupies time, and row emphasis. */
    val tintFill = IsctTokens.courseTintFill.toFloat()
    val tintSoft = IsctTokens.courseTintSoft.toFloat()
}

@Composable
fun IsctTheme(content: @Composable () -> Unit) {
    val palette = if (isSystemInDarkTheme()) dark else light
    val scheme = if (palette === dark) {
        darkColorScheme(
            primary = palette.accent, background = palette.bg, surface = palette.surface,
            onBackground = palette.ink, onSurface = palette.ink, error = palette.danger,
            // Navigation bar, chips and switches take the campus tint, not Material's purple.
            secondaryContainer = palette.accentSoft, onSecondaryContainer = palette.accent,
            surfaceContainer = palette.surface, surfaceContainerLow = palette.surface,
            surfaceContainerHigh = palette.surface2, surfaceContainerHighest = palette.surface2,
            onSurfaceVariant = palette.muted, outline = palette.line, outlineVariant = palette.line,
        )
    } else {
        lightColorScheme(
            primary = palette.accent, background = palette.bg, surface = palette.surface,
            onBackground = palette.ink, onSurface = palette.ink, error = palette.danger,
            // Navigation bar, chips and switches take the campus tint, not Material's purple.
            secondaryContainer = palette.accentSoft, onSecondaryContainer = palette.accent,
            surfaceContainer = palette.surface, surfaceContainerLow = palette.surface,
            surfaceContainerHigh = palette.surface2, surfaceContainerHighest = palette.surface2,
            onSurfaceVariant = palette.muted, outline = palette.line, outlineVariant = palette.line,
        )
    }
    CompositionLocalProvider(LocalPalette provides palette) {
        MaterialTheme(colorScheme = scheme, content = content)
    }
}

/** Course colours arrive from Core as `#rrggbb`. */
@Composable
fun courseColor(hex: String?): Color {
    val digits = hex.orEmpty().trimStart('#')
    val value = if (digits.length == 6) digits.toLongOrNull(16) else null
    return value?.let { Color(0xff000000 or it) } ?: Theme.colors.accent
}

/** White rounded card on the warm background, the phone's one container. */
@Composable
fun Card(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Column(
        modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Theme.radiusSurface))
            .background(Theme.colors.surface),
        content = content,
    )
}

/** Small grey caption above a card. */
@Composable
fun SectionLabel(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.labelLarge,
        fontWeight = FontWeight.SemiBold,
        color = Theme.colors.muted,
        modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 8.dp),
    )
}

/** "Belongs to this course" in lists. */
@Composable
fun CourseSquare(color: String?) {
    Box(
        Modifier
            .size(10.dp)
            .clip(RoundedCornerShape(3.dp))
            .background(courseColor(color)),
    )
}

/** Thin divider inset past the leading marker, as in grouped lists. */
@Composable
fun RowDivider(inset: Dp = 40.dp) {
    Box(
        Modifier
            .padding(start = inset)
            .fillMaxWidth()
            .height(0.5.dp)
            .background(Theme.colors.line),
    )
}
