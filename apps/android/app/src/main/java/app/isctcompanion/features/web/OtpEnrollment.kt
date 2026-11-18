package app.isctcompanion.features.web

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.SchoolAuthProfile
import app.isctcompanion.core.SchoolOtpEnrollment
import app.isctcompanion.core.SchoolOtpIntent
import app.isctcompanion.core.SchoolOtpStage
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t

/** Native consent; Core owns enrollment and the separate reset consent. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun OtpEnrollmentStart(openManual: () -> Unit, openSetup: () -> Unit) {
    var consent by remember { mutableStateOf(false) }
    FlowRow(horizontalArrangement = Arrangement.spacedBy(Theme.rowGap)) {
        TextButton(onClick = { consent = true }) { Text(t("otpEnrollment.help")) }
        TextButton(onClick = openManual) { Text(t("otpEnrollment.manual")) }
    }
    if (consent) {
        AlertDialog(
            onDismissRequest = { consent = false },
            title = { Text(t("otpEnrollment.help")) },
            text = { Text(t("otpEnrollment.consent")) },
            confirmButton = {
                TextButton(onClick = { consent = false; openSetup() }) { Text(t("otpEnrollment.continue")) }
            },
            dismissButton = {
                TextButton(onClick = { consent = false }) { Text(t("common.cancel")) }
            },
        )
    }
}

/** Renders Core's credential-free state and forwards explicit intents. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun OtpEnrollmentNotice(otp: SchoolOtpEnrollment) {
    val web = LocalModel.current.web
    val navigator = LocalNavigator.current
    val confirm = otp.stage == SchoolOtpStage.CONFIRM_RESET
    Column(Modifier.fillMaxWidth().background(Theme.colors.surface).padding(horizontal = Theme.rowX, vertical = Theme.rowY)) {
        Text(t(message(otp.stage), "account" to otp.owner.orEmpty()), style = MaterialTheme.typography.bodyMedium)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(Theme.rowGap)) {
            if (confirm) {
                TextButton(onClick = { web.otpIntent(otp, SchoolOtpIntent.RESET) }) {
                    Text(t("otpEnrollment.reset"), color = Theme.colors.danger)
                }
            }
            if (confirm || otp.stage == SchoolOtpStage.COMPLETE) {
                TextButton(onClick = {
                    if (confirm) web.otpIntent(otp, SchoolOtpIntent.STOP)
                    navigator.openSettings(SchoolAuthProfile.SCIENCE)
                }) { Text(t(if (confirm) "otpEnrollment.import" else "otpEnrollment.settings")) }
            }
            if (otp.stage == SchoolOtpStage.SAVE_FAILED) {
                TextButton(onClick = { web.otpIntent(otp, SchoolOtpIntent.RETRY_SAVE) }) { Text(t("otpEnrollment.retrySave")) }
            }
            if (otp.stage != SchoolOtpStage.SAVING) {
                val label = when (otp.stage) {
                    SchoolOtpStage.CONFIRM_RESET -> "otpEnrollment.keep"
                    SchoolOtpStage.SIGN_IN, SchoolOtpStage.CHECKING, SchoolOtpStage.SETTING_UP -> "common.cancel"
                    else -> "common.done"
                }
                TextButton(onClick = { web.otpIntent(otp, SchoolOtpIntent.STOP) }) { Text(t(label)) }
            }
        }
    }
}

private fun message(stage: SchoolOtpStage): String = when (stage) {
    SchoolOtpStage.SIGN_IN -> "otpEnrollment.signIn"
    SchoolOtpStage.CHECKING -> "otpEnrollment.checking"
    SchoolOtpStage.CONFIRM_RESET -> "otpEnrollment.confirmReset"
    SchoolOtpStage.SETTING_UP -> "otpEnrollment.settingUp"
    SchoolOtpStage.SAVING -> "otpEnrollment.saving"
    SchoolOtpStage.COMPLETE -> "otpEnrollment.complete"
    SchoolOtpStage.ACCOUNT_MISMATCH -> "otpEnrollment.accountMismatch"
    SchoolOtpStage.INCOMPLETE -> "otpEnrollment.incomplete"
    SchoolOtpStage.UNKNOWN -> "otpEnrollment.unknown"
    SchoolOtpStage.SAVE_FAILED -> "otpEnrollment.saveFailed"
}
