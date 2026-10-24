package app.isctcompanion.design

import androidx.compose.foundation.background
import androidx.compose.foundation.border
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
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.colorspace.ColorSpaces
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.hypot
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
    /** The 'now' gradient's peak in OKLCH ([nowGradient]): lightness, and the chroma cap. */
    val heroLightness: Float,
    val heroChroma: Float,
)

private val light = with(IsctTokens) {
    Palette(
        colorBg, colorSurface, colorSurface2, colorInk, colorMuted, colorLine, colorAccent,
        colorAccentSoft, colorDanger, colorDangerSoft, colorWarn, colorWarnSoft, colorOk, colorOkSoft,
        heroLightness.toFloat(), heroChroma.toFloat(),
    )
}

private val dark = with(IsctTokensDark) {
    Palette(
        colorBg, colorSurface, colorSurface2, colorInk, colorMuted, colorLine, colorAccent,
        colorAccentSoft, colorDanger, colorDangerSoft, colorWarn, colorWarnSoft, colorOk, colorOkSoft,
        heroLightness.toFloat(), heroChroma.toFloat(),
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

/** Smoothstep from the peak to the surface: (position along the 72% radius,
 * share of the peak), matching the desktop's stops. */
private val HERO_FALLOFF = listOf(0.25f to 0.84f, 0.5f to 0.5f, 0.75f to 0.16f, 1f to 0f)

/** The 'now' gradient's peak: [tint]'s hue at the theme's hero lightness,
 * its chroma capped, so every course glows equally quietly (design/tokens). */
fun heroPeak(tint: Color, palette: Palette): Color {
    val lab = tint.convert(ColorSpaces.Oklab)
    val chroma = hypot(lab.green, lab.blue)
    val scale = if (chroma > palette.heroChroma) palette.heroChroma / chroma else 1f
    return Color(palette.heroLightness, lab.green * scale, lab.blue * scale, 1f, ColorSpaces.Oklab)
        .convert(ColorSpaces.Srgb)
}

/** The 'now' gradient (design/README.md) the hero card shares with desktop's
 * focus row: [heroPeak] in the top-start corner easing (smoothstep) into the
 * surface by 72% of an ellipse 140% wide and 180% tall (the desktop's
 * radial-gradient), with a hairline at [Theme.tintFill]. Every stop is
 * opaque: fading to Color.Transparent (transparent black) greyed the middle. */
fun Modifier.nowGradient(tint: Color, palette: Palette, shape: Shape): Modifier = this
    .clip(shape)
    .drawBehind {
        drawRect(palette.surface)
        val rx = size.width * 1.4f
        val ry = size.height * 1.8f
        if (rx <= 0f || ry <= 0f) return@drawBehind
        val start = if (layoutDirection == LayoutDirection.Rtl) size.width else 0f
        val peak = heroPeak(tint, palette)
        // lerp mixes in Oklab, as the desktop's color-mix(in oklab).
        val stops = arrayOf(0f to peak) + HERO_FALLOFF.map { (at, share) ->
            at * 0.72f to lerp(palette.surface, peak, share)
        }
        // A circle squashed vertically draws the ellipse.
        scale(scaleX = 1f, scaleY = ry / rx, pivot = Offset(start, 0f)) {
            drawRect(
                Brush.radialGradient(*stops, center = Offset(start, 0f), radius = rx),
                size = Size(size.width, size.height * rx / ry),
            )
        }
    }
    .border(0.5.dp, tint.copy(alpha = Theme.tintFill), shape)

@Composable
fun IsctTheme(content: @Composable () -> Unit) {
    val palette = if (isSystemInDarkTheme()) dark else light
    val scheme = if (palette === dark) {
        darkColorScheme(
            primary = palette.accent, background = palette.bg, surface = palette.surface,
            onBackground = palette.ink, onSurface = palette.ink, error = palette.danger,
            // Navigation bar, chips and switches take the campus tint, not Material's purple.
            secondaryContainer = palette.accentSoft, onSecondaryContainer = palette.accent,
            inversePrimary = palette.accentSoft,
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
            inversePrimary = palette.accentSoft,
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
