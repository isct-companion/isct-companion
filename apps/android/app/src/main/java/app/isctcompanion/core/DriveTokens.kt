package app.isctcompanion.core

import android.content.Context
import android.content.Intent
import androidx.activity.ComponentActivity
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.IntentSenderRequest
import androidx.activity.result.contract.ActivityResultContracts
import com.google.android.gms.auth.api.identity.AuthorizationRequest
import com.google.android.gms.auth.api.identity.AuthorizationResult
import com.google.android.gms.auth.api.identity.ClearTokenRequest
import com.google.android.gms.auth.api.identity.Identity
import com.google.android.gms.common.api.Scope
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.tasks.await
import kotlinx.coroutines.withContext

/** Drive access tokens from Google Identity's authorization API, which keeps
 * the grant in the device's Google account and refreshes it itself. Core owns
 * every Drive request; this only answers "a token for this scope". The
 * Android OAuth client is matched by package name and signing certificate. */
class DriveTokens(private val context: Context) : NativeDriveTokens {
    private var consent: ActivityResultLauncher<IntentSenderRequest>? = null
    private var answer: CompletableDeferred<Intent?>? = null
    @Volatile private var lastToken: String? = null

    /** The activity in front registers the consent screen launcher. */
    fun attach(activity: ComponentActivity) {
        consent = activity.registerForActivityResult(ActivityResultContracts.StartIntentSenderForResult()) {
            answer?.complete(it.data)
        }
    }

    override suspend fun accessToken(scope: String, interactive: Boolean): String {
        val client = Identity.getAuthorizationClient(context)
        val request = AuthorizationRequest.builder().setRequestedScopes(listOf(Scope(scope))).build()
        var result: AuthorizationResult = try {
            client.authorize(request).await()
        } catch (error: Exception) {
            throw SdkException.Network("Google authorization failed: ${error.message}")
        }
        if (result.hasResolution()) {
            if (!interactive) throw SdkException.Authentication("Google Drive needs you to sign in again.")
            val pending = result.pendingIntent
                ?: throw SdkException.Authentication("Google sign-in is unavailable.")
            val data = withContext(Dispatchers.Main) {
                val launcher = consent ?: throw SdkException.Unavailable("No screen can show Google sign-in.")
                val waiting = CompletableDeferred<Intent?>().also { answer = it }
                launcher.launch(IntentSenderRequest.Builder(pending.intentSender).build())
                waiting.await()
            } ?: throw SdkException.Cancelled()
            result = try {
                client.getAuthorizationResultFromIntent(data)
            } catch (error: Exception) {
                throw SdkException.Authentication("Google sign-in was declined.")
            }
        }
        return result.accessToken?.also { lastToken = it }
            ?: throw SdkException.Authentication("Google returned no Drive authorization.")
    }

    /** Drops the cached token; Core forgets that Drive was connected. */
    override fun signOut() {
        val token = lastToken ?: return
        lastToken = null
        Identity.getAuthorizationClient(context)
            .clearToken(ClearTokenRequest.builder().setToken(token).build())
    }
}
