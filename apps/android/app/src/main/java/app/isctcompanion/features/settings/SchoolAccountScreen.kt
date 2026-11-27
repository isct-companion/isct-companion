package app.isctcompanion.features.settings

import app.isctcompanion.design.t
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.autofill.ContentType
import androidx.compose.ui.semantics.contentType
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.CredentialAvailability
import app.isctcompanion.core.CredentialSummary
import app.isctcompanion.core.SchoolAuthProfile
import app.isctcompanion.core.SchoolConnectOutcome
import app.isctcompanion.core.SignInPresentation
import app.isctcompanion.core.SignInRecovery
import app.isctcompanion.core.SchoolCredentials
import app.isctcompanion.core.SchoolProfileSession
import app.isctcompanion.core.SchoolSessionState
import app.isctcompanion.core.SchoolSite
import app.isctcompanion.design.Card
import app.isctcompanion.design.Format
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.rememberSecretReveal
import kotlinx.coroutines.launch
import app.isctcompanion.core.connect
import app.isctcompanion.core.retrySchool
import app.isctcompanion.core.deleteCredentials

internal fun accountName(profile: SchoolAuthProfile) =
    if (profile == SchoolAuthProfile.SCIENCE) "Science Tokyo" else t("settings.main.tokyoTechFormer")

/** The colour of an account's state: green while connected, red when the
 * school turned the details down, amber while it waits on the school page or
 * the network. */
@Composable
private fun stateColor(state: SchoolSessionState): Color = when (state) {
    SchoolSessionState.NORMAL -> Theme.colors.ok
    SchoolSessionState.REFUSED -> Theme.colors.danger
    SchoolSessionState.NEEDS_WEB, SchoolSessionState.UNAVAILABLE -> Theme.colors.warn
    SchoolSessionState.NOT_CONFIGURED, SchoolSessionState.SIGNING_IN -> Theme.colors.muted
}

@Composable
internal fun AccountRow(
    profile: SchoolAuthProfile,
    name: String,
    credential: CredentialSummary?,
    session: SchoolProfileSession?,
    lastRun: String?,
    onClick: () -> Unit,
) {
    val connected = credential?.availability == CredentialAvailability.AVAILABLE ||
        credential?.availability == CredentialAvailability.SAVED_LOCKED
    val (label, color) = when {
        session?.presentation?.recovery?.contains(SignInRecovery.FINISH_SIGN_OUT) == true -> AppModel.sessionText(session) to Theme.colors.danger
        !connected -> t("common.notConnected") to Theme.colors.muted
        session == null -> t("settings.main.checking") to Theme.colors.muted
        session.state == SchoolSessionState.NORMAL -> (
            lastRun?.let { t("settings.main.connectedUpdatedTime", "time" to Format.due(it)) } ?: AppModel.sessionText(session)
            ) to Theme.colors.ok
        else -> AppModel.sessionText(session) to stateColor(session.state)
    }
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Theme.rowGap),
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = Theme.rowX, vertical = Theme.rowY),
    ) {
        Box(Modifier.size(8.dp).clip(CircleShape).background(if (connected) color else Theme.colors.line))
        Column(Modifier.weight(1f)) {
            Text(name, color = Theme.colors.ink)
            Text(label, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted)
            session?.maintenance?.takeIf { connected }?.let {
                Text(AppModel.maintenanceText(profile, it), style = MaterialTheme.typography.bodySmall, color = Theme.colors.warn)
            }
        }
        Icon(Icons.Rounded.ChevronRight, null, tint = Theme.colors.muted)
    }
}

/** Where the account's session stands when it needs a word: why the school
 * turned the details down, why it wants its own page, or when Core tries
 * again, each with Retry; and the site's maintenance. */
@Composable
private fun SessionStatus(
    profile: SchoolAuthProfile,
    session: SchoolProfileSession,
    retrying: Boolean,
    enabled: Boolean,
    openPortal: () -> Unit,
    openSchoolPage: () -> Unit,
    retry: () -> Unit,
) {
    val state = session.state
    val presentation = session.presentation
    val maintenance = session.maintenance
    if (presentation.recovery.isEmpty() && maintenance == null) return
    Card {
        Column(Modifier.padding(Theme.space8), verticalArrangement = Arrangement.spacedBy(Theme.space4)) {
            if (presentation.recovery.isNotEmpty()) {
                Text(AppModel.sessionText(session), color = stateColor(state))
                SchoolPageRecovery(presentation, openPortal, openSchoolPage)
                if (SignInRecovery.RETRY in presentation.recovery) {
                    OutlinedButton(onClick = retry, enabled = enabled, modifier = Modifier.fillMaxWidth()) {
                        if (retrying) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp) else Text(t("common.retry"))
                    }
                }
                if (state == SchoolSessionState.REFUSED) {
                    Text(t("schoolSession.retry.otherDevice"), style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
                }
            }
            maintenance?.let {
                Text(AppModel.maintenanceText(profile, it), style = MaterialTheme.typography.bodyMedium, color = Theme.colors.warn)
            }
        }
    }
}

/** Sign in, change or forget a school account. The password goes straight to
 * Core, which signs in with it and stores it in the Keystore vault only when
 * the school accepted it. */
@Composable
internal fun AccountPage(profile: SchoolAuthProfile, done: () -> Unit) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val scope = rememberCoroutineScope()
    val summary = if (profile == SchoolAuthProfile.SCIENCE) model.credentials?.science else model.credentials?.tokyoTech
    var username by remember { mutableStateOf(summary?.username.orEmpty()) }
    var password by remember { mutableStateOf("") }
    var otpSecret by remember { mutableStateOf("") }
    var matrix by remember { mutableStateOf("") }
    var saving by remember { mutableStateOf(false) }
    var retrying by remember { mutableStateOf(false) }
    var removing by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var outcome by remember { mutableStateOf<SchoolConnectOutcome?>(null) }
    // Bumped when a Connect saved the entries, which clears the OTP and
    // Matrix sections' own state along with the fields.
    var entries by remember { mutableIntStateOf(0) }
    val busy = saving || retrying || removing
    val saved = summary?.saved == true
    val session = if (profile == SchoolAuthProfile.SCIENCE) model.sessions?.science else model.sessions?.tokyoTech
    // The portal's start page as it is, where the person signs in and sets up
    // school OTP by hand.
    val openPortal: () -> Unit = {
        model.web.openUnsigned(SchoolSite.SCIENCE_PORTAL)
        navigator.openTab("tab/web")
    }
    val openSchoolPage: () -> Unit = {
        model.web.showTab(if (profile == SchoolAuthProfile.SCIENCE) SchoolSite.SCIENCE_PORTAL else SchoolSite.TOKYO_PORTAL)
        navigator.openTab("tab/web")
    }
    val finishingSignOut = session?.presentation?.recovery?.contains(SignInRecovery.FINISH_SIGN_OUT) == true
    Page {
        if (session != null && finishingSignOut) {
            Footer(AppModel.sessionText(session), Theme.colors.danger)
        } else if (session != null) {
            SessionStatus(profile, session, retrying, enabled = !busy, openPortal = openPortal, openSchoolPage = openSchoolPage) {
                scope.launch {
                    retrying = true
                    error = null
                    try {
                        model.retrySchool(profile)
                    } catch (failure: Exception) {
                        error = AppModel.describe(failure)
                    }
                    retrying = false
                }
            }
        }
        OutlinedTextField(
            username, { username = it }, label = { Text(t("settings.schoolAccounts.id")) }, singleLine = true,
            keyboardOptions = KeyboardOptions(
                capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false, keyboardType = KeyboardType.Ascii,
            ),
            // Compose fields join system Autofill only when they say what they hold.
            modifier = Modifier.fillMaxWidth().semantics { contentType = ContentType.Username },
        )
        Secret(password, { password = it }, if (saved) t("settings.main.passwordEnterOnlyToChange") else t("settings.main.password"), login = true)
        Footer(t("settings.main.thePasswordIsEncryptedIn"))
        key(entries) {
            if (profile == SchoolAuthProfile.SCIENCE) {
                SectionLabel(t("settings.main.oneTimePassword"))
                OtpImportField(
                    saved = summary?.hasOtp == true,
                    verifiedAccount = summary?.saved == true,
                    canShowCode = session?.signOutPending != true,
                    setup = otpSecret,
                    openSetup = {
                        model.web.openOtpSetup(username)
                        navigator.openTab("tab/web")
                    },
                ) { otpSecret = it }
            } else {
                SectionLabel(t("settings.main.matrixCard"))
                MatrixImportSection(summary?.hasMatrix == true) { matrix = it }
            }
        }
        outcome?.let { ConnectOutcome(it, openPortal, openSchoolPage) }
        error?.let { Footer(it, Theme.colors.danger) }
        Button(
            onClick = {
                scope.launch {
                    saving = true
                    error = null
                    outcome = null
                    try {
                        outcome = model.connect(profile, SchoolCredentials(username, password, otpSecret, matrix))
                        // Only Connected saved the entries; after any other
                        // outcome they stay, so the person can fix them and
                        // press Connect again.
                        if (outcome is SchoolConnectOutcome.Connected) {
                            password = ""
                            otpSecret = ""
                            matrix = ""
                            entries++
                        }
                    } catch (failure: Exception) {
                        error = AppModel.describe(failure)
                    }
                    saving = false
                }
            },
            enabled = !busy && session?.signOutPending != true && username.isNotEmpty() && (password.isNotEmpty() || saved),
            modifier = Modifier.fillMaxWidth().padding(top = Theme.space4),
        ) {
            if (saving) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp) else Text(t("settings.main.saveAndConnect"))
        }
        if (saved || finishingSignOut) {
            // Core stops the account's reads and clears the web sign-in
            // before this returns, which can take a moment.
            TextButton(
                onClick = {
                    scope.launch {
                        removing = true
                        error = null
                        try {
                            model.deleteCredentials(profile)
                            done()
                        } catch (failure: Exception) {
                            error = AppModel.describe(failure)
                        }
                        removing = false
                    }
                },
                enabled = !busy,
                modifier = Modifier.fillMaxWidth(),
            ) {
                if (removing) {
                    CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp, color = Theme.colors.danger)
                } else {
                    Text(t(if (finishingSignOut) "settings.schoolAccounts.finishSignOut" else "settings.main.removeSignInDetailsFrom"), color = Theme.colors.danger)
                }
            }
        }
    }
}

/** What a Connect ended with, under the form. */
@Composable
private fun ConnectOutcome(outcome: SchoolConnectOutcome, openPortal: () -> Unit, openSchoolPage: () -> Unit) {
    val (presentation, color) = when (outcome) {
        is SchoolConnectOutcome.Connected -> outcome.presentation to Theme.colors.ok
        is SchoolConnectOutcome.Refused -> outcome.presentation to Theme.colors.danger
        is SchoolConnectOutcome.NeedsWeb -> outcome.presentation to Theme.colors.warn
        is SchoolConnectOutcome.Unreachable -> outcome.presentation to Theme.colors.warn
        is SchoolConnectOutcome.NotSaved -> outcome.presentation to Theme.colors.danger
        is SchoolConnectOutcome.Stopped -> outcome.presentation to Theme.colors.muted
    }
    val text = if (outcome is SchoolConnectOutcome.Refused || outcome is SchoolConnectOutcome.NeedsWeb) {
        val reason = presentation.reasonKey?.let { t(it) } ?: t(presentation.stateKey)
        "$reason\n${t("schoolSession.connect.notSaved")}"
    } else AppModel.signInText(presentation)
    Footer(text, color)
    SchoolPageRecovery(presentation, openPortal, openSchoolPage)
}

@Composable
private fun SchoolPageRecovery(presentation: SignInPresentation, openPortal: () -> Unit, openSchoolPage: () -> Unit) {
    if (SignInRecovery.SET_UP_OTP_ON_PORTAL in presentation.recovery) {
        OutlinedButton(onClick = openPortal, modifier = Modifier.fillMaxWidth()) {
            Text(t("schoolSession.openPortal"))
        }
    }
    if (SignInRecovery.OPEN_SCHOOL_PAGE in presentation.recovery) {
        OutlinedButton(onClick = openSchoolPage, modifier = Modifier.fillMaxWidth()) {
            Text(t("settings.schoolAccounts.showPage"))
        }
    }
}

@Composable
private fun Secret(
    value: String,
    onChange: (String) -> Unit,
    label: String,
    capitalization: KeyboardCapitalization = KeyboardCapitalization.None,
    login: Boolean = false,
) {
    // Only the login password is offered to password managers; the TOTP secret
    // and matrix are pasted keys and stay out of Autofill.
    val reveal = rememberSecretReveal()
    OutlinedTextField(
        value, onChange, label = { Text(label) }, singleLine = true,
        visualTransformation = reveal.transformation,
        trailingIcon = { reveal.Toggle() },
        keyboardOptions = KeyboardOptions(
            capitalization = capitalization, autoCorrectEnabled = false,
            keyboardType = if (login) KeyboardType.Password else KeyboardType.Ascii,
        ),
        modifier = Modifier.fillMaxWidth().then(
            if (login) Modifier.semantics { contentType = ContentType.Password } else Modifier,
        ),
    )
}
