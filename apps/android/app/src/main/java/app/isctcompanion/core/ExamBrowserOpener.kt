package app.isctcompanion.core

import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Opens Google's sign-in page in a Custom Tab. Core binds the loopback
 * listener before asking, and that listener receives the answer, so the app
 * stays awake meanwhile ([SignInService]; [AppModel.examShare] ends it). */
class ExamBrowserOpener(private val context: Context) : NativeBrowserOpener {
    override suspend fun openUrl(url: String) = withContext(Dispatchers.Main) {
        SignInService.start(context)
        val tab = CustomTabsIntent.Builder().build()
        tab.intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        tab.launchUrl(context, Uri.parse(url))
    }
}
