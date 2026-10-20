package app.isctcompanion.core

import android.webkit.CookieManager
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** `NativeDownloadSession` over the process `CookieManager`, the store every
 * school `WebView` signs in to: Core's HTTP requests (course files, LMS,
 * 教務Web) read the session from it and write server cookies back, so the
 * views and Core never hold diverging sessions. */
class WebCookieSession : NativeDownloadSession {
    override suspend fun cookieHeader(url: String): String = withContext(Dispatchers.IO) {
        CookieManager.getInstance().getCookie(url) ?: ""
    }

    override suspend fun storeSetCookies(url: String, setCookie: List<String>) {
        withContext(Dispatchers.IO) {
            val cookies = CookieManager.getInstance()
            setCookie.forEach { cookies.setCookie(url, it) }
            // Session cookies would otherwise reach disk only on the next
            // periodic sync, after a process kill could already lose them.
            cookies.flush()
        }
    }
}
