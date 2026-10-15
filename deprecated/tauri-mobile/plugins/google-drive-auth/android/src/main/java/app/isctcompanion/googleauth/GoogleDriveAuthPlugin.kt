package app.isctcompanion.googleauth

import android.app.Activity
import androidx.activity.result.ActivityResult
import androidx.activity.result.IntentSenderRequest
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.google.android.gms.auth.api.identity.AuthorizationRequest
import com.google.android.gms.auth.api.identity.AuthorizationResult
import com.google.android.gms.auth.api.identity.Identity
import com.google.android.gms.common.api.Scope

@InvokeArg
class SignInArgs { var clientId: String = ""; var interactive: Boolean = false }

/** AuthorizationClient renews on-device grants. Background requests never show consent. */
@TauriPlugin
class GoogleDriveAuthPlugin(private val host: Activity) : Plugin(host) {
    @Command
    fun signIn(invoke: Invoke) {
        val args = invoke.parseArgs(SignInArgs::class.java)
        val request = AuthorizationRequest.builder()
            .setRequestedScopes(listOf(Scope("https://www.googleapis.com/auth/drive.appdata")))
            .build()
        Identity.getAuthorizationClient(host).authorize(request)
            .addOnSuccessListener { result ->
                if (result.hasResolution()) {
                    if (!args.interactive) {
                        invoke.reject("Google Drive authorization needs renewal. Reconnect in Backup settings.")
                    } else {
                        val pending = result.pendingIntent
                        if (pending == null) invoke.reject("Google Drive consent is unavailable.")
                        else startIntentSenderForResult(invoke, IntentSenderRequest.Builder(pending).build(), "authorized")
                    }
                } else complete(invoke, result)
            }
            .addOnFailureListener { invoke.reject(it.message ?: "Google Drive authorization failed.") }
    }
    @ActivityCallback
    fun authorized(invoke: Invoke, result: ActivityResult) {
        try { complete(invoke, Identity.getAuthorizationClient(host).getAuthorizationResultFromIntent(result.data)) }
        catch (error: Exception) { invoke.reject(error.message ?: "Google Drive authorization was cancelled.") }
    }
    private fun complete(invoke: Invoke, result: AuthorizationResult) {
        val token = result.accessToken
        if (token.isNullOrEmpty()) { invoke.reject("Google Drive did not return an access token."); return }
        val response = JSObject()
        response.put("clientId", invoke.parseArgs(SignInArgs::class.java).clientId)
        response.put("accessToken", token)
        response.put("expiresIn", 3600)
        invoke.resolve(response)
    }
    @Command
    fun signOut(invoke: Invoke) { invoke.resolve() }
}
