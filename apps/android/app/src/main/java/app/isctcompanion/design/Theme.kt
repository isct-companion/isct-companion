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
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.colorspace.ColorSpaces
import androidx.compose.ui.graphics.lerp
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
    /** Text and icons on the solid accent. */
    val accentInk: Color,
    val accentSoft: Color,
    val danger: Color,
    val dangerSoft: Color,
    val warn: Color,
    val warnSoft: Color,
    val ok: Color,
    val okSoft: Color,
    /** The 'now' tint in OKLCH ([nowTint]): lightness, and the chroma cap. */
    val heroLightness: Float,
    val heroChroma: Float,
    /** Share of the 'now' tint left at the card's bottom edge. */
    val heroFade: Float,
)

private val light = with(IsctTokens) {
    Palette(
        colorBg, colorSurface, colorSurface2, colorInk, colorMuted, colorLine, colorAccent,
        colorAccentInk, colorAccentSoft, colorDanger, colorDangerSoft, colorWarn, colorWarnSoft, colorOk, colorOkSoft,
        heroLightness.toFloat(), heroChroma.toFloat(), heroFade.toFloat(),
    )
}

private val dark = with(IsctTokensDark) {
    Palette(
        colorBg, colorSurface, colorSurface2, colorInk, colorMuted, colorLine, colorAccent,
        colorAccentInk, colorAccentSoft, colorDanger, colorDangerSoft, colorWarn, colorWarnSoft, colorOk, colorOkSoft,
        heroLightness.toFloat(), heroChroma.toFloat(), heroFade.toFloat(),
    )
}

private val LocalPalette = staticCompositionLocalOf { light }

object Theme {
    val colors: Palette
        @Composable get() = LocalPalette.current

    val radiusSurface: Dp = IsctTokens.radiusSurface
    val radiusPanel: Dp = IsctTokens.radiusPanel
    val radiusChip: Dp = IsctTokens.radiusChip
    val radiusBlock: Dp = IsctTokens.radiusBlock
    val radiusColumn: Dp = IsctTokens.radiusColumn
    val radiusControl: Dp = IsctTokens.radiusControl

    /** Page side margin. */
    val gutter: Dp = IsctTokens.spaceGutter
    /** A list row's padding, and the gap between rows and a row's mark and text. */
    val rowX: Dp = IsctTokens.spaceRowX
    val rowY: Dp = IsctTokens.spaceRowY
    val rowGap: Dp = IsctTokens.spaceRowGap
    val cardPadding: Dp = IsctTokens.spaceCard

    /** Course colour strength of a block that occupies time, a quiz included. */
    val tintFill = IsctTokens.courseTintFill.toFloat()

    /** An exam block's deeper fill; it takes no ring (a ring means now). */
    val tintExam = IsctTokens.courseTintExam.toFloat()

    /** Text on a fill: the course colour mixed into ink. */
    val tintInk = IsctTokens.courseTintInk.toFloat()

    /** Text on the solid course colour, such as an exam tag. */
    val onSolid: Color = IsctTokens.courseOnSolid
}

/** [course] mixed into [base] at [share], opaque, so row rules and the lunch
 * band never show through a block and a course reads the same on any surface
 * (slotledger `approach/course-tint-opaque-mix`). Mixed in sRGB like the
 * desktop's `color-mix(in srgb, …)`; Compose's own `lerp` mixes in Oklab. */
fun courseMix(course: Color, share: Float, base: Color): Color {
    val from = base.convert(ColorSpaces.Srgb)
    val to = course.convert(ColorSpaces.Srgb)
    fun mix(a: Float, b: Float) = a + (b - a) * share
    return Color(mix(from.red, to.red), mix(from.green, to.green), mix(from.blue, to.blue), 1f)
}

/** The 'now' tint: [tint]'s hue at the theme's hero lightness, its chroma
 * capped, so every course is equally quiet (design/tokens `hero`). */
fun heroTint(tint: Color, palette: Palette): Color {
    val lab = tint.convert(ColorSpaces.Oklab)
    val chroma = hypot(lab.green, lab.blue)
    val scale = if (chroma > palette.heroChroma) palette.heroChroma / chroma else 1f
    return Color(palette.heroLightness, lab.green * scale, lab.blue * scale, 1f, ColorSpaces.Oklab)
        .convert(ColorSpaces.Srgb)
}

/** The 'now' card (design/README.md) the hero card shares with desktop's
 * focus row: [heroTint] fading slightly from top to bottom ([Palette.heroFade],
 * mixed in Oklab like the desktop), with a hairline at [Theme.tintFill]. */
fun Modifier.nowTint(tint: Color, palette: Palette, shape: Shape): Modifier = this
    .clip(shape)
    .background(
        heroTint(tint, palette).let { top ->
            Brush.verticalGradient(listOf(top, lerp(palette.surface, top, palette.heroFade)))
        },
    )
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
