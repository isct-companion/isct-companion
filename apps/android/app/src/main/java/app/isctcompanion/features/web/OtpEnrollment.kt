package app.isctcompanion.features.web

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.SchoolAuthProfile
import app.isctcompanion.core.SchoolOtpEnrollment
import app.isctcompanion.core.SchoolOtpIntent
import app.isctcompanion.core.SchoolOtpStage
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t

/** Renders Core's credential-free state and forwards explicit intents. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun OtpEnrollmentNotice(otp: SchoolOtpEnrollment) {
    val web = LocalModel.current.web
    val navigator = LocalNavigator.current
    Column(Modifier.fillMaxWidth().background(Theme.colors.surface).padding(horizontal = Theme.rowX, vertical = Theme.rowY)) {
        Text(t(message(otp.stage), "account" to otp.owner.orEmpty()), style = MaterialTheme.typography.bodyMedium)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(Theme.rowGap)) {
            if (otp.stage == SchoolOtpStage.COMPLETE) {
                TextButton(onClick = { navigator.openSettings(SchoolAuthProfile.SCIENCE) }) {
                    Text(t("otpEnrollment.settings"))
                }
            }
            if (otp.stage == SchoolOtpStage.SAVE_FAILED) {
                TextButton(onClick = { web.otpIntent(otp, SchoolOtpIntent.RETRY_SAVE) }) { Text(t("otpEnrollment.retrySave")) }
            }
            if (otp.stage != SchoolOtpStage.SAVING) {
                val label = when (otp.stage) {
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
    SchoolOtpStage.SETTING_UP -> "otpEnrollment.settingUp"
    SchoolOtpStage.SAVING -> "otpEnrollment.saving"
    SchoolOtpStage.COMPLETE -> "otpEnrollment.complete"
    SchoolOtpStage.ACCOUNT_MISMATCH -> "otpEnrollment.accountMismatch"
    SchoolOtpStage.INCOMPLETE -> "otpEnrollment.incomplete"
    SchoolOtpStage.UNKNOWN -> "otpEnrollment.unknown"
    SchoolOtpStage.SAVE_FAILED -> "otpEnrollment.saveFailed"
}
