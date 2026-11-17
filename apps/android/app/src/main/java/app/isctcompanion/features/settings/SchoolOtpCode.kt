package app.isctcompanion.features.settings

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import app.isctcompanion.LocalModel
import app.isctcompanion.core.SchoolOtpCode
import app.isctcompanion.design.t
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.isActive

@Composable
internal fun SchoolOtpCodeButton() {
    var opened by remember { mutableStateOf(false) }
    Button(onClick = { opened = true }) { Text(t("settings.otp.showCode")) }
    if (opened) SchoolOtpCodeDialog { opened = false }
}

@Composable
private fun SchoolOtpCodeDialog(close: () -> Unit) {
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
    AlertDialog(
        onDismissRequest = close,
        title = { Text(t("settings.otp.currentCode")) },
        text = {
            if (visible != null) {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(visible.otp, style = MaterialTheme.typography.headlineLarge, fontFamily = FontFamily.Monospace)
                    Text(t("settings.otp.expiresIn", "seconds" to seconds))
                }
            } else {
                Text(t(if (unavailable) "settings.otp.codeUnavailable" else "common.loading"))
            }
        },
        dismissButton = { TextButton(onClick = close) { Text(t("common.close")) } },
        confirmButton = {
            if (visible != null) {
                TextButton(onClick = {
                    val remaining = visible.remainingMillis()
                    if (remaining > 0) {
                        copied = copySensitiveOtp(context, t("settings.otp.currentCode"), visible.otp, remaining)
                    }
                }) {
                    Text(t(if (copied) "settings.credentialImport.copied" else "settings.otp.copyCode"))
                }
            }
        },
    )
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
