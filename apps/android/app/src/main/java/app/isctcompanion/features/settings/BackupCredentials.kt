package app.isctcompanion.features.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import app.isctcompanion.LocalModel
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.credentialPasswordLength
import app.isctcompanion.design.Card
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.rememberSecretReveal
import app.isctcompanion.design.t
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** 登录凭据: school logins in an encrypted Google Drive file, the desktop's
 * credential sync. Core decides whether a password restores the backup or
 * encrypts a new one, and keeps the password only when that succeeded. */
@Composable
internal fun BackupCredentials(toast: (String) -> Unit) {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    val length = remember { credentialPasswordLength() }
    var configured by remember { mutableStateOf(false) }
    // Null while Drive is being checked, or when it could not be reached.
    var exists by remember { mutableStateOf<Boolean?>(null) }
    // Password fields show only while setting up, changing or restoring.
    var editing by remember { mutableStateOf(false) }
    var password by remember { mutableStateOf("") }
    var confirmPassword by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        configured = model.backup.credentialBackupConfigured() == true
        exists = model.backup.credentialBackupExists()
    }
    val restoring = !configured && exists == true
    val showFields = editing || restoring
    val count = password.codePointCount(0, password.length).toUInt()
    val matches = password == confirmPassword
    // A backup opens with whatever password made it, which earlier versions
    // allowed shorter; only a new password meets the length Core requires.
    val valid = if (restoring) count > 0u
        else count in length.min..length.max && confirmPassword.isNotEmpty() && matches
    val range = arrayOf<Pair<String, Any>>("min" to length.min.toInt(), "max" to length.max.toInt())

    fun reset() {
        password = ""
        confirmPassword = ""
        editing = false
    }
    fun run(failure: String, work: suspend () -> Unit) {
        scope.launch {
            busy = true
            try {
                work()
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                toast(t(failure) + "\n" + AppModel.describe(error))
            } finally {
                busy = false
            }
        }
    }
    fun save() = run("settings.backup.couldNotEnableCredentialSync") {
        model.backup.enableCredentialBackup(password)
        reset()
        configured = true
        exists = true
        toast(t("settings.backup.loginCredentialsHaveBeenSynced"))
    }
    fun turnOff() = run("settings.backup.couldNotDisableCredentialSync") {
        model.backup.disableCredentialBackup()
        reset()
        configured = false
        // Only this device's password is removed; the Drive file stays unlockable.
        exists = true
        toast(t("settings.backup.loginCredentialSyncIsDisabled"))
    }

    val title = if (restoring) t("settings.backup.restoreLoginCredentials") else t("settings.backup.loginCredentialSync")
    val hint = when {
        exists == null && !configured -> t("settings.backup.checkingGoogleDrive")
        configured -> if (editing) t("settings.backup.chooseANewPassword", *range) else t("settings.extensions.on")
        restoring -> t("settings.backup.enterThePasswordSetOn")
        editing -> t("settings.backup.chooseAPassword", *range)
        else -> t("settings.main.off")
    }

    SectionLabel(t("settings.backup.loginCredentials"))
    Card {
        Column(
            Modifier.fillMaxWidth().padding(horizontal = Theme.rowX, vertical = Theme.rowY),
            verticalArrangement = Arrangement.spacedBy(Theme.space4),
        ) {
            Text(title, color = Theme.colors.ink)
            Text(hint, style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
            if (showFields) {
                val reveal = rememberSecretReveal()
                OutlinedTextField(
                    password, { password = it },
                    singleLine = true,
                    enabled = !busy,
                    label = {
                        Text(t(if (restoring) "settings.backup.encryptionPassword" else "settings.backup.newPassword"))
                    },
                    visualTransformation = reveal.transformation,
                    trailingIcon = { reveal.Toggle() },
                    keyboardOptions = KeyboardOptions(
                        keyboardType = KeyboardType.Password,
                        imeAction = if (restoring) ImeAction.Done else ImeAction.Next,
                    ),
                    modifier = Modifier.fillMaxWidth(),
                )
                if (!restoring) {
                    OutlinedTextField(
                        confirmPassword, { confirmPassword = it },
                        singleLine = true,
                        enabled = !busy,
                        label = { Text(t("settings.backup.confirmPassword")) },
                        isError = confirmPassword.isNotEmpty() && !matches,
                        supportingText = if (confirmPassword.isNotEmpty() && !matches) {
                            { Text(t("settings.backup.theTwoPasswordsDoNot")) }
                        } else null,
                        // The pair shows or hides together, from the first field's switch.
                        visualTransformation = reveal.transformation,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Done),
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
                Text(
                    t("settings.backup.encryptedWithThisPasswordBefore"),
                    style = MaterialTheme.typography.bodySmall,
                    color = Theme.colors.muted,
                )
            }
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(Theme.space4, Alignment.End),
                modifier = Modifier.fillMaxWidth(),
            ) {
                when {
                    showFields -> {
                        if (editing) TextButton(onClick = ::reset, enabled = !busy) { Text(t("common.cancel")) }
                        Button(onClick = ::save, enabled = !busy && valid) {
                            Text(
                                t(
                                    when {
                                        restoring -> "common.restore"
                                        configured -> "common.save"
                                        else -> "settings.backup.turnOn"
                                    },
                                ),
                            )
                        }
                    }
                    configured -> {
                        TextButton(onClick = { editing = true }, enabled = !busy) { Text(t("settings.backup.changePassword")) }
                        TextButton(onClick = ::turnOff, enabled = !busy) { Text(t("settings.backup.turnOff")) }
                    }
                    exists != null -> {
                        TextButton(onClick = { editing = true }, enabled = !busy) { Text(t("settings.backup.setUp")) }
                    }
                }
            }
        }
    }
}
