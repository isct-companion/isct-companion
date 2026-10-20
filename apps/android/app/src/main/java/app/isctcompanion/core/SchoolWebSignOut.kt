package app.isctcompanion.core

import android.webkit.CookieManager
import android.webkit.WebStorage
import kotlin.coroutines.resume
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext

/** Signs the school web views out for Settings → Reset. `CookieManager`
 * names no cookie's domain or path, so it cannot single out school cookies;
 * every view in this app is a school view (other sites open in the browser),
 * so its whole store goes, then reaches disk before Core reads again. */
class SchoolWebSignOut : NativeWebSignOut {
    override suspend fun signOutSchoolWeb() {
        withContext(Dispatchers.Main) {
            val cookies = CookieManager.getInstance()
            suspendCancellableCoroutine { done -> cookies.removeAllCookies { done.resume(Unit) } }
            cookies.flush()
            WebStorage.getInstance().deleteAllData()
        }
    }
}
