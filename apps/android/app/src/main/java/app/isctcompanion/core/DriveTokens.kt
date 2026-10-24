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
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder
import java.security.MessageDigest
import kotlin.concurrent.thread

/** Drive access tokens from Google Identity's authorization API, which keeps
 * the grant in the device's Google account and refreshes it itself. Core owns
 * every Drive request; this only answers "a token for this scope". The
 * Android OAuth client is matched by package name and signing certificate. */
class DriveTokens(private val context: Context) : NativeDriveTokens {
    private var consent: ActivityResultLauncher<IntentSenderRequest>? = null
    private var answer: CompletableDeferred<Intent?>? = null
    @Volatile private var lastToken: String? = null
    /** The scope Core asked for; after a restart, the one it always asks for. */
    @Volatile private var grantedScope = DRIVE_APPDATA

    /** The activity in front registers the consent screen launcher. */
    fun attach(activity: ComponentActivity) {
        consent = activity.registerForActivityResult(ActivityResultContracts.StartIntentSenderForResult()) {
            answer?.complete(it.data)
        }
    }

    override suspend fun accessToken(scope: String, interactive: Boolean): String {
        grantedScope = scope
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

    /** Forgets the grant, as the port promises, so turning Drive on again
     * asks instead of reconnecting silently. A drive.appdata-only grant
     * does not name its Google account, which `revokeAccess` needs, so the
     * grant is revoked through Google's OAuth revoke endpoint with a token
     * (the last one, or after a restart one a silent authorization gives),
     * and the token then leaves the system cache. Core has already
     * forgotten that Drive was connected, so a failure here leaves only the
     * grant on Google's side. */
    override fun signOut() {
        val client = Identity.getAuthorizationClient(context)
        val revoke = { token: String ->
            thread(name = "drive-revoke") {
                runCatching {
                    val connection = URL(REVOKE).openConnection() as HttpURLConnection
                    connection.requestMethod = "POST"
                    connection.doOutput = true
                    connection.setRequestProperty("Content-Type", "application/x-www-form-urlencoded")
                    connection.outputStream.use { it.write("token=${URLEncoder.encode(token, "UTF-8")}".toByteArray()) }
                    connection.responseCode.also { connection.disconnect() }
                }.onSuccess { Log.i("isct", "Drive grant revoked ($it)") }
                    .onFailure { Log.w("isct", "Drive grant not revoked", it) }
                client.clearToken(ClearTokenRequest.builder().setToken(token).build())
            }
        }
        val cached = lastToken
        lastToken = null
        if (cached != null) {
            revoke(cached)
        } else {
            client.authorize(AuthorizationRequest.builder().setRequestedScopes(listOf(Scope(grantedScope))).build())
                .addOnSuccessListener { result -> result.accessToken?.let(revoke) }
        }
    }

    private companion object {
        const val DRIVE_APPDATA = "https://www.googleapis.com/auth/drive.appdata"
        const val REVOKE = "https://oauth2.googleapis.com/revoke"
    }
}
