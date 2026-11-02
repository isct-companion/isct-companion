package app.isctcompanion.core

import android.webkit.CookieManager
import android.webkit.WebStorage
import kotlin.coroutines.resume
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** `NativeWebCookies` over the process `CookieManager`, the store every
 * school `WebView` reads. Core writes the account's sign-in cookies here
 * before it opens a school page, and clears them on sign-out, account switch
 * and reset; it never reads them back.
 *
 * Writes and clears run one at a time, each once `CookieManager` reported
 * the previous one done, and a write older than the last clear is dropped,
 * so a sign-in seeded for an account cannot land after its sign-out.
 * `CookieManager` answers on the calling thread's looper, so every call is
 * made on the main thread. */
class WebCookieSeed : NativeWebCookies {
    private val queue = Mutex()

    /** The generation of the last clear; read and written under [queue]. */
    private var lastCleared: ULong = 0u

    override suspend fun write(generation: ULong, cookies: List<WebCookie>) {
        queue.withLock {
            if (generation < lastCleared) return@withLock
            withContext(Dispatchers.Main) {
                val store = CookieManager.getInstance()
                for (cookie in cookies) {
                    // WebView's own parser takes the header as the school sent
                    // it; a cookie rebuilt from its parts would lose the
                    // leading dot of `Domain`.
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
    }

    /** `CookieManager` names no cookie's domain, so it cannot single out
     * school cookies; every view in this app is a school view (other sites
     * open in the browser), so the whole store goes, with the pages' storage. */
    override suspend fun clear(generation: ULong) {
        queue.withLock {
            withContext(Dispatchers.Main) {
                val store = CookieManager.getInstance()
                suspendCancellableCoroutine<Unit> { done -> store.removeAllCookies { done.resume(Unit) } }
                WebStorage.getInstance().deleteAllData()
                store.flush()
            }
            lastCleared = generation
        }
    }
}
