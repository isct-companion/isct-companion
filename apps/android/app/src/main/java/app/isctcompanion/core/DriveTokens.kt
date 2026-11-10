package app.isctcompanion.core

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.IntentSenderRequest
import androidx.activity.result.contract.ActivityResultContracts
import com.google.android.gms.auth.api.identity.AuthorizationRequest
import com.google.android.gms.auth.api.identity.AuthorizationResult
import com.google.android.gms.auth.api.identity.ClearTokenRequest
import com.google.android.gms.auth.api.identity.Identity
import com.google.android.gms.common.api.ApiException
import com.google.android.gms.common.api.CommonStatusCodes
import com.google.android.gms.common.api.Scope
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.tasks.await
import kotlinx.coroutines.withContext
import java.security.MessageDigest

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
            } catch (error: ApiException) {
                throw refusal(error)
            }
        }
        return result.accessToken?.also { lastToken = it }
            ?: throw SdkException.Authentication("Google returned no Drive authorization.")
    }

    /** What Google's answer means. A closed prompt is a cancel, not a
     * refusal. A build Google does not know (its package and signing
     * certificate are not on the app's Android OAuth client, which needs an
     * entry per signing key) names the certificate to register. */
    private fun refusal(error: ApiException): SdkException {
        Log.w("isct", "Drive authorization", error)
        val unregistered = error.statusCode == CommonStatusCodes.DEVELOPER_ERROR ||
            "UNREGISTERED_ON_API_CONSOLE" in error.message.orEmpty()
        return when {
            error.statusCode == CommonStatusCodes.CANCELED -> SdkException.Cancelled()
            unregistered -> SdkException.Unavailable(
                "Google does not recognise this build. Register package ${context.packageName} with " +
                    "signing certificate SHA-1 ${signingSha1()} on the app's Android OAuth client.",
            )
            else -> SdkException.Authentication("Google sign-in failed (${error.statusCode}).")
        }
    }

    /** This APK's signing certificate, as the Google Cloud console asks for it. */
    private fun signingSha1(): String = runCatching {
        val info = context.packageManager.getPackageInfo(context.packageName, PackageManager.GET_SIGNING_CERTIFICATES)
        val certificate = info.signingInfo?.apkContentsSigners?.firstOrNull()?.toByteArray() ?: return "?"
        MessageDigest.getInstance("SHA-1").digest(certificate).joinToString(":") { "%02X".format(it) }
    }.getOrDefault("?")

    /** Forgets this device's token, as the port promises. The grant is not
     * revoked: Google revokes it for every client of the project, which
     * would sign the person's other devices out of Drive too. Core has
     * already forgotten that Drive was connected. */
    override fun signOut() {
        val token = lastToken ?: return
        lastToken = null
        Identity.getAuthorizationClient(context).clearToken(ClearTokenRequest.builder().setToken(token).build())
    }
}
