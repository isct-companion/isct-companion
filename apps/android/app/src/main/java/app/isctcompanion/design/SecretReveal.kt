package app.isctcompanion.design

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Visibility
import androidx.compose.material.icons.rounded.VisibilityOff
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation

/** A password field's show/hide switch: pass [transformation] as the
 * field's `visualTransformation` and put [Toggle] in its `trailingIcon`.
 * Every secret field (school accounts, backup) uses this one. */
@Stable
class SecretReveal internal constructor() {
    var shown by mutableStateOf(false)
        private set

    val transformation: VisualTransformation
        get() = if (shown) VisualTransformation.None else PasswordVisualTransformation()

    @Composable
    fun Toggle() {
        IconButton(onClick = { shown = !shown }) {
            Icon(
                if (shown) Icons.Rounded.VisibilityOff else Icons.Rounded.Visibility,
                t(if (shown) "common.hidePassword" else "common.showPassword"),
                tint = Theme.colors.muted,
            )
        }
    }
}

/** Hidden at first, and hidden again once the page is left. */
@Composable
fun rememberSecretReveal(): SecretReveal = remember { SecretReveal() }
