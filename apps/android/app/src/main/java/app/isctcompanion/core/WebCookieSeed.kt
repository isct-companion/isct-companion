package app.isctcompanion.core

import android.webkit.CookieManager
import android.webkit.WebStorage
import kotlin.coroutines.resume
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext

/** `NativeWebCookies` over the process `CookieManager`, the store every
 * school `WebView` reads. Core writes an account's verified sign-in cookies
 * here before it opens a school page, and clears the school sign-ins on
 * reset or when the person chooses to; it never reads them back.
 *
 * Core runs the calls one at a time and drops stale writes; each call here
 * answers once `CookieManager` reported it done, however long that takes.
 * `CookieManager` answers on the calling thread's looper, so every call is
 * made on the main thread. */
class WebCookieSeed : NativeWebCookies {
    override suspend fun write(cookies: List<WebCookie>) {
        withContext(Dispatchers.Main) {
            val store = CookieManager.getInstance()
            for (cookie in cookies) {
                // WebView's own parser takes the header as the school sent
                // it; a cookie rebuilt from its parts would lose the leading
                // dot of `Domain`.
                val stored = suspendCancellableCoroutine<Boolean> { done ->
                    store.setCookie(cookie.url, cookie.setCookie) { done.resume(it == true) }
                }
                if (!stored) throw SdkException.Callback("WebView refused a school cookie")
            }
            // Session cookies would otherwise reach disk only on the next
            // periodic sync, after a process kill could already lose them.
            store.flush()
        }
    }

    /** `CookieManager` names no cookie's domain, so it cannot single out one
     * account's cookies ([clearsBySite]); every view in this app is a school
     * view (other sites open in the browser), so clearing every school site
     * clears the whole store, with the pages' storage. */
    override suspend fun clear(scope: WebClearScope) {
        if (scope !is WebClearScope.AllSchool) throw SdkException.Callback("WebView clears only every school site")
        withContext(Dispatchers.Main) {
            val store = CookieManager.getInstance()
            suspendCancellableCoroutine<Unit> { done -> store.removeAllCookies { done.resume(Unit) } }
            WebStorage.getInstance().deleteAllData()
            store.flush()
        }
    }

    override fun clearsBySite(): Boolean = false
}
