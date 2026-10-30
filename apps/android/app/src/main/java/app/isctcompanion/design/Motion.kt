package app.isctcompanion.design

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibilityScope
import androidx.compose.animation.ExperimentalSharedTransitionApi
import androidx.compose.animation.SharedTransitionScope
import androidx.compose.animation.SharedTransitionScope.ResizeMode.Companion.RemeasureToBounds
import android.content.Context
import android.provider.Settings
import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.unit.dp
import app.isct.design.IsctTokens
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.key
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.TextUnit

// The finish the content components share (slotledger `approach/mobile-moment-first`), the
// Compose counterpart of iOS's `Design/Polish.swift`. Only Compose's own
// animation APIs. Motion is quiet and never overshoots (`approach/motion-tokens`),
// and "Remove animations" is read explicitly (`approach/motion-reduced`).

/** The system asks for less motion: "Remove animations" sets the animator or
 * transition scale to 0. Read once at start and provided by the root. */
val LocalReducedMotion = staticCompositionLocalOf { false }

fun reducedMotion(context: Context): Boolean {
    fun scale(name: String) = Settings.Global.getFloat(context.contentResolver, name, 1f)
    return scale(Settings.Global.ANIMATOR_DURATION_SCALE) == 0f ||
        scale(Settings.Global.TRANSITION_ANIMATION_SCALE) == 0f
}

/** The three lengths of design/tokens/motion.json, as tweens on the desktop's
 * curves and as critically damped springs of about the same length. */
object Motion {
    private val standard = CubicBezierEasing(0.2f, 0f, 0f, 1f)
    private val emphasized = CubicBezierEasing(0.22f, 1f, 0.36f, 1f)

    fun <T> fast() = tween<T>(IsctTokens.motionDurationFast, easing = standard)
    fun <T> base() = tween<T>(IsctTokens.motionDurationBase, easing = standard)
    fun <T> slow() = tween<T>(IsctTokens.motionDurationSlow, easing = emphasized)

    fun <T> fastSpring() = spring<T>(Spring.DampingRatioNoBouncy, Spring.StiffnessHigh)
    fun <T> baseSpring() = spring<T>(Spring.DampingRatioNoBouncy, Spring.StiffnessMedium)
    fun <T> slowSpring() = spring<T>(Spring.DampingRatioNoBouncy, Spring.StiffnessMediumLow)
}

/** A placeholder in the shape of what is loading, while a view has no data
 * yet (slotledger `approach/loading-skeleton`). */
@Composable
fun Skeleton(modifier: Modifier, shape: Shape = RoundedCornerShape(Theme.radiusBlock)) {
    Box(modifier.clip(shape).background(Theme.colors.surface2))
}

/** A card of [count] placeholder list rows: a course square and two lines. */
@Composable
fun SkeletonRows(count: Int = 4, modifier: Modifier = Modifier) {
    Card(modifier) {
        repeat(count) { index ->
            if (index > 0) RowDivider()
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(Theme.rowGap),
                modifier = Modifier.fillMaxWidth().padding(horizontal = Theme.rowX, vertical = Theme.rowY),
            ) {
                Skeleton(Modifier.size(10.dp), RoundedCornerShape(3.dp))
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Skeleton(Modifier.fillMaxWidth(if (index % 2 == 0) 0.7f else 0.55f).height(14.dp))
                    Skeleton(Modifier.fillMaxWidth(0.4f).height(10.dp))
                }
            }
        }
    }
}

/** [content] once [loaded], after a crossfade from [placeholder]. */
@Composable
fun Loading(loaded: Boolean, placeholder: @Composable () -> Unit, content: @Composable () -> Unit) {
    Crossfade(loaded, animationSpec = Motion.base(), label = "loading") { ready ->
        if (ready) content() else placeholder()
    }
}

/** Figures that count (minutes left, the week range) roll instead of
 * jumping: each changed character slides in, digits that stay put stay
 * still, like SwiftUI's `numericText`. Characters are keyed from the end, so
 * 10 → 9 rolls the last digit rather than shifting both. */
@Composable
fun CountingText(
    text: String,
    modifier: Modifier = Modifier,
    style: TextStyle = TextStyle.Default,
    color: Color = Color.Unspecified,
    fontSize: TextUnit = TextUnit.Unspecified,
    fontWeight: FontWeight? = null,
    countsDown: Boolean = true,
) {
    // With less motion the figures simply change.
    if (LocalReducedMotion.current) {
        Text(text, modifier, style = style, color = color, fontSize = fontSize, fontWeight = fontWeight, maxLines = 1)
        return
    }
    Row(modifier) {
        text.forEachIndexed { index, character ->
            key(text.length - index) {
                AnimatedContent(
                    character,
                    transitionSpec = {
                        // Counting down, the new figure drops in from above.
                        val from = if (countsDown) -1 else 1
                        (slideInVertically(Motion.baseSpring()) { from * it } + fadeIn(Motion.base())) togetherWith
                            (slideOutVertically(Motion.baseSpring()) { -from * it } + fadeOut(Motion.base()))
                    },
                    label = "counting figure",
                ) {
                    Text(it.toString(), style = style, color = color, fontSize = fontSize, fontWeight = fontWeight, maxLines = 1)
                }
            }
        }
    }
}

// Zoom into a course

/** The navigation host's shared-transition scope and the scope of the
 * destination on screen, provided by the root (`App.kt`). */
@OptIn(ExperimentalSharedTransitionApi::class)
val LocalSharedTransition = compositionLocalOf<SharedTransitionScope?> { null }
val LocalDestinationScope = compositionLocalOf<AnimatedVisibilityScope?> { null }

/** Marks where a course opened with the same `zoom` id grows from: a
 * timetable block, a Today card or a course row. Back shrinks it home. */
@Composable
fun Modifier.courseZoomSource(zoom: String?): Modifier = courseZoom(zoom, destination = false)

/** The course page that grows out of its `courseZoomSource`. */
@Composable
fun Modifier.courseZoomDestination(zoom: String?): Modifier = courseZoom(zoom, destination = true)

@OptIn(ExperimentalSharedTransitionApi::class)
@Composable
private fun Modifier.courseZoom(zoom: String?, destination: Boolean): Modifier {
    // With less motion the course page only fades in (NavHost's default).
    if (LocalReducedMotion.current) return this
    val shared = LocalSharedTransition.current ?: return this
    val scope = LocalDestinationScope.current ?: return this
    zoom ?: return this
    return with(shared) {
        val bounds = this@courseZoom.sharedBounds(
            rememberSharedContentState("course-zoom-$zoom"),
            scope,
            resizeMode = RemeasureToBounds,
            clipInOverlayDuringTransition = OverlayClip(RoundedCornerShape(Theme.radiusPanel)),
        )
        // The source is only ever stretched past its own size, but the page
        // would be squeezed down to the block's: laid out at that size its
        // collapsed large title measured a negative height and crashed. The
        // page keeps its final layout and the animated bounds only clip it.
        if (destination) bounds.skipToLookaheadSize() else bounds
    }
}
