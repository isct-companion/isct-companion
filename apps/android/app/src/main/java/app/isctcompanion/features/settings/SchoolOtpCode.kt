package app.isctcompanion.features.settings

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material.icons.rounded.Pin
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameMillis
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.LifecycleResumeEffect
import app.isctcompanion.LocalModel
import app.isctcompanion.core.SchoolOtpCode
import app.isctcompanion.design.ActionButton
import app.isctcompanion.design.ButtonLevel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import app.isctcompanion.design.tabular
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.isActive

@Composable
internal fun SchoolOtpCodeButton() {
    var opened by remember { mutableStateOf(false) }
    ActionButton(t("settings.otp.showCode"), { opened = true }, level = ButtonLevel.Secondary, icon = Icons.Rounded.Pin)
    if (opened) SchoolOtpCodeSheet { opened = false }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SchoolOtpCodeSheet(close: () -> Unit) {
    val model = LocalModel.current
    val context = LocalContext.current
    var resumed by remember { mutableStateOf(false) }
    var current by remember { mutableStateOf<SchoolOtpCode?>(null) }
    var seconds by remember { mutableStateOf(0L) }
    var copied by remember { mutableStateOf(false) }
    var unavailable by remember { mutableStateOf(false) }

    LifecycleResumeEffect(Unit) {
        resumed = true
        onPauseOrDispose {
            resumed = false
            current = null
        }
    }
    LaunchedEffect(resumed) {
        if (!resumed) return@LaunchedEffect
        unavailable = false
        while (isActive) {
            if ((current?.remainingMillis() ?: 0) <= 0) {
                current = null
                copied = false
                val next = try {
                    model.schoolOtpCode()
                } catch (_: Exception) {
                    null
                }
                currentCoroutineContext().ensureActive()
                if (next == null) {
                    unavailable = true
                    return@LaunchedEffect
                }
                current = next
            }
            seconds = ((current?.remainingMillis() ?: 0) + 999) / 1000
            delay(1000 - System.currentTimeMillis() % 1000)
        }
    }

    val visible = current?.takeIf { resumed && seconds > 0 }
    val colors = Theme.colors
    fun copy(code: SchoolOtpCode) {
        val remaining = code.remainingMillis()
        if (remaining > 0) copied = copySensitiveOtp(context, t("settings.otp.currentCode"), code.otp, remaining)
    }
    ModalBottomSheet(onDismissRequest = close, containerColor = colors.bg) {
        Column(
            verticalArrangement = Arrangement.spacedBy(Theme.space9),
            modifier = Modifier.fillMaxWidth().padding(start = Theme.space9, end = Theme.space9, bottom = Theme.space10),
        ) {
            Text(t("settings.otp.currentCode"), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            if (visible != null) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(Theme.radiusColumn))
                        .background(colors.surface)
                        // The code is the copy target too, as in an authenticator.
                        .clickable(onClickLabel = t("settings.otp.copyCode")) { copy(visible) }
                        .padding(horizontal = Theme.space9, vertical = Theme.cardPadding),
                ) {
                    OtpDigits(visible.otp, Modifier.weight(1f))
                    OtpCountdown(visible, seconds)
                }
                ActionButton(
                    t(if (copied) "settings.credentialImport.copied" else "settings.otp.copyCode"),
                    { copy(visible) },
                    modifier = Modifier.fillMaxWidth(),
                    icon = if (copied) Icons.Rounded.Check else Icons.Rounded.ContentCopy,
                )
            } else if (unavailable) {
                Text(t("settings.otp.codeUnavailable"), color = colors.muted)
            } else {
                Box(Modifier.fillMaxWidth().height(88.dp), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp)
                }
            }
        }
    }
}

/** Grouped as authenticators show it: 123 456. */
@Composable
private fun OtpDigits(otp: String, modifier: Modifier = Modifier) {
    val half = otp.length / 2
    Text(
        otp.substring(0, half) + "\u2009" + otp.substring(half),
        style = MaterialTheme.typography.displaySmall.tabular(),
        fontWeight = FontWeight.SemiBold,
        letterSpacing = 2.sp,
        color = Theme.colors.ink,
        maxLines = 1,
        modifier = modifier,
    )
}

/** What is left of the time step: a ring that empties smoothly, the seconds
 * inside it, both in the danger colour for the last five seconds. */
@Composable
private fun OtpCountdown(code: SchoolOtpCode, seconds: Long) {
    val colors = Theme.colors
    val ending = seconds <= 5
    val now by produceState(System.currentTimeMillis(), code) {
        while (true) withFrameMillis { value = System.currentTimeMillis() }
    }
    val left = ((code.expiresAt.toLong() * 1000 - now).toFloat() / (code.period.toLong() * 1000)).coerceIn(0f, 1f)
    val label = t("settings.otp.expiresIn", "seconds" to seconds)
    Box(
        contentAlignment = Alignment.Center,
        modifier = Modifier.size(44.dp).semantics(mergeDescendants = true) { contentDescription = label },
    ) {
        CircularProgressIndicator(
            progress = { left },
            modifier = Modifier.fillMaxSize(),
            color = if (ending) colors.danger else colors.accent,
            trackColor = colors.line,
            strokeWidth = 3.dp,
            strokeCap = StrokeCap.Round,
            gapSize = 0.dp,
        )
        Text(
            seconds.toString(),
            style = MaterialTheme.typography.labelLarge.tabular(),
            fontWeight = FontWeight.SemiBold,
            color = if (ending) colors.danger else colors.muted,
        )
    }
}

private fun SchoolOtpCode.remainingMillis() = expiresAt.toLong() * 1000 - System.currentTimeMillis()

/** Hide OTP values in clipboard previews and keep later copies intact. */
internal fun copySensitiveOtp(context: Context, label: String, text: String, lifetimeMs: Long): Boolean {
    val clipboard = context.getSystemService(ClipboardManager::class.java) ?: return false
    val clip = ClipData.newPlainText(label, text)
    clip.description.extras = PersistableBundle().apply {
        putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true)
    }
    clipboard.setPrimaryClip(clip)
    val copied = clipboard.primaryClipDescription?.timestamp
    Handler(Looper.getMainLooper()).postDelayed({
        if (copied != null && clipboard.primaryClipDescription?.timestamp == copied) {
            clipboard.clearPrimaryClip()
        }
    }, lifetimeMs)
    return true
}
