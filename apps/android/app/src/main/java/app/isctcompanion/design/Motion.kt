package app.isctcompanion.design

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibilityScope
import androidx.compose.animation.ExperimentalSharedTransitionApi
import androidx.compose.animation.SharedTransitionScope
import androidx.compose.animation.SharedTransitionScope.ResizeMode.Companion.RemeasureToBounds
import androidx.compose.animation.core.spring
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

// The finish the content components share (docs/ux/mobile.md, Look), the
// Compose counterpart of iOS's `Design/Polish.swift`. Only Compose's own
// animation APIs, which follow the system's animator duration scale, so
// "Remove animations" turns them off as Reduce Motion does on iOS.

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
    Row(modifier) {
        text.forEachIndexed { index, character ->
            key(text.length - index) {
                AnimatedContent(
                    character,
                    transitionSpec = {
                        // Counting down, the new figure drops in from above.
                        val from = if (countsDown) -1 else 1
                        (slideInVertically(spring()) { from * it } + fadeIn()) togetherWith
                            (slideOutVertically(spring()) { -from * it } + fadeOut())
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
fun Modifier.courseZoomSource(zoom: String?): Modifier = courseZoom(zoom)

/** The course page that grows out of its `courseZoomSource`. */
@Composable
fun Modifier.courseZoomDestination(zoom: String?): Modifier = courseZoom(zoom)

@OptIn(ExperimentalSharedTransitionApi::class)
@Composable
private fun Modifier.courseZoom(zoom: String?): Modifier {
    val shared = LocalSharedTransition.current ?: return this
    val scope = LocalDestinationScope.current ?: return this
    zoom ?: return this
    return with(shared) {
        this@courseZoom.sharedBounds(
            rememberSharedContentState("course-zoom-$zoom"),
            scope,
            resizeMode = RemeasureToBounds,
            clipInOverlayDuringTransition = OverlayClip(RoundedCornerShape(Theme.radiusPanel)),
        )
    }
}
