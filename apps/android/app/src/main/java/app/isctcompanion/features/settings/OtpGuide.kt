package app.isctcompanion.features.settings

import android.content.Context
import android.graphics.BitmapFactory
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.keyframes
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.TouchApp
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.isctcompanion.design.LocalReducedMotion
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import kotlinx.coroutines.launch
import org.json.JSONObject

/** Step-by-step OTP setup guides (slotledger `approach/otp-setup-guide`): the
 * same screenshots and step list as the desktop and iOS, packaged from
 * `resources/otp-guide`. Each step is a screenshot, where to tap on it (as a
 * fraction of its width and height) or `screenshot` when the person should
 * capture that screen, and a caption key. */
enum class OtpGuide(val key: String, val title: String) {
    Google("google", "otpGuide.google.title"),
    TitechApp("titechApp", "otpGuide.titechApp.title");

    class Step(val image: String, val tapX: Float?, val tapY: Float?, val screenshot: Boolean, val caption: String)

    fun steps(context: Context): List<Step> = all(context)[key].orEmpty()

    private companion object {
        private var loaded: Map<String, List<Step>>? = null

        fun all(context: Context): Map<String, List<Step>> = loaded ?: run {
            val json = JSONObject(context.assets.open("otp-guide.json").bufferedReader().use { it.readText() })
            json.keys().asSequence().associateWith { name ->
                val list = json.getJSONArray(name)
                List(list.length()) { index ->
                    val step = list.getJSONObject(index)
                    val tap = step.optJSONObject("tap")
                    Step(
                        step.getString("image"),
                        tap?.getDouble("x")?.toFloat(),
                        tap?.getDouble("y")?.toFloat(),
                        step.optString("action") == "screenshot",
                        step.getString("caption"),
                    )
                }
            }.also { loaded = it }
        }
    }
}

/** "Show steps" for an import guide. */
@Composable
fun OtpGuideButton(guide: OtpGuide) {
    val context = LocalContext.current
    val steps = remember(guide) { guide.steps(context) }
    var shown by remember { mutableStateOf(false) }
    TextButton(onClick = { shown = true }) {
        Icon(Icons.Rounded.TouchApp, null, modifier = Modifier.size(18.dp))
        Text(t("otpGuide.showSteps"), modifier = Modifier.padding(start = 6.dp))
    }
    if (shown) OtpGuideSheet(guide, steps) { shown = false }
}

/** One page per step, swiped; the dots say where you are. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun OtpGuideSheet(guide: OtpGuide, steps: List<OtpGuide.Step>, close: () -> Unit) {
    val pager = rememberPagerState { steps.size }
    ModalBottomSheet(
        onDismissRequest = close,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = Theme.colors.bg,
    ) {
        Column(Modifier.fillMaxWidth().padding(bottom = 24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            Row(Modifier.padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(
                    t(guide.title), style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold,
                    color = Theme.colors.ink, modifier = Modifier.weight(1f),
                )
                TextButton(onClick = close) { Text(t("common.done")) }
            }
            HorizontalPager(pager, Modifier.fillMaxWidth()) { index ->
                val step = steps[index]
                Column(
                    Modifier.fillMaxWidth().padding(horizontal = 16.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    Screenshot(step, isCurrent = index == pager.settledPage)
                    Text(
                        t(step.caption), style = MaterialTheme.typography.bodyMedium, color = Theme.colors.ink,
                        textAlign = TextAlign.Center,
                    )
                }
            }
            Row(Modifier.padding(top = 12.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                repeat(steps.size) { index ->
                    Box(
                        Modifier.size(7.dp).clip(CircleShape)
                            .background(if (index == pager.currentPage) Theme.colors.ink else Theme.colors.line),
                    )
                }
            }
        }
    }
}

/** The screenshot sized to fit, with the tap marker at the step's point. */
@Composable
private fun Screenshot(step: OtpGuide.Step, isCurrent: Boolean) {
    val context = LocalContext.current
    val bitmap = remember(step.image) {
        context.assets.open(step.image).use { BitmapFactory.decodeStream(it) }.asImageBitmap()
    }
    val shape = RoundedCornerShape(12.dp)
    BoxWithConstraints(
        Modifier.heightIn(max = 480.dp).aspectRatio(bitmap.width.toFloat() / bitmap.height)
            .clip(shape).border(1.dp, Theme.colors.line, shape),
    ) {
        Image(bitmap, null, Modifier.fillMaxWidth())
        val x = step.tapX
        val y = step.tapY
        if (x != null && y != null) {
            TapMarker(Modifier.offset(maxWidth * x - 20.dp, maxHeight * y - 20.dp))
        } else if (isCurrent && step.screenshot) {
            CaptureMarker(bitmap, maxWidth, maxHeight)
        }
    }
}

/** "Take a screenshot": the screen flashes, then shrinks into a thumbnail in
 * the corner, the way the system shows a capture; with animations removed
 * only the thumbnail shows. */
@Composable
private fun CaptureMarker(bitmap: ImageBitmap, width: Dp, height: Dp) {
    val still = LocalReducedMotion.current
    val flash = remember { Animatable(0f) }
    val scale = remember { Animatable(3.2f) }
    val shown = remember { Animatable(0f) }
    LaunchedEffect(still) {
        if (still) return@LaunchedEffect
        launch { flash.animateTo(0f, keyframes { durationMillis = 470; 0.85f at 130 }) }
        launch { scale.animateTo(1f, keyframes { durationMillis = 780; 3.2f at 210 }) }
        shown.animateTo(0f, keyframes { durationMillis = 2600; 0f at 130; 1f at 210; 1f at 2210 })
    }
    Box(Modifier.size(width, height)) {
        if (!still) Box(Modifier.matchParentSize().alpha(flash.value).background(Color.White))
        val shape = RoundedCornerShape(6.dp)
        Image(
            bitmap, null,
            Modifier.align(Alignment.BottomStart).padding(start = width * 0.05f, bottom = height * 0.04f)
                .graphicsLayer {
                    if (!still) {
                        scaleX = scale.value
                        scaleY = scale.value
                        alpha = shown.value
                    }
                    transformOrigin = TransformOrigin(0f, 1f)
                }
                .size(width * 0.3f, height * 0.3f)
                .shadow(7.dp, shape).clip(shape).border(3.dp, Color.White, shape),
        )
    }
}

/** A ring that keeps pulsing out of a dot, like a touch indicator in a screen
 * recording; with animations removed it holds still. White edges keep it
 * visible on light and dark screenshots. */
@Composable
private fun TapMarker(modifier: Modifier) {
    val still = LocalReducedMotion.current
    val pulse = rememberInfiniteTransition(label = "tap")
    val progress by pulse.animateFloat(
        0f, 1f, infiniteRepeatable(tween(1600, easing = LinearEasing), RepeatMode.Restart), label = "ring",
    )
    val ring = if (still) 0.5f else progress
    val white = Color.White.copy(alpha = 0.85f)
    Box(modifier.size(40.dp), contentAlignment = Alignment.Center) {
        Box(
            Modifier.size(40.dp).scale(0.5f + ring * 1.1f).alpha(if (still) 1f else 1f - progress)
                .border(5.dp, white, CircleShape).padding(1.dp).border(3.dp, Theme.colors.accent, CircleShape),
        )
        Box(
            Modifier.size(20.dp).clip(CircleShape).background(Theme.colors.accent.copy(alpha = 0.7f))
                .border(2.dp, white, CircleShape),
        )
    }
}
