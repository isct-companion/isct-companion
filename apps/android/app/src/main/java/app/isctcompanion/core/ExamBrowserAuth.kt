package app.isctcompanion.core

import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Core binds and owns the loopback callback before the tab opens. */
class ExamBrowserAuth(private val context: Context) : NativeBrowserAuth {
    override suspend fun authenticate(url: String, callbackScheme: String?): String? = withContext(Dispatchers.Main) {
        val tab = CustomTabsIntent.Builder().build()
        tab.intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        tab.launchUrl(context, Uri.parse(url))
        null
    }

    override suspend fun cancel() = Unit
}
