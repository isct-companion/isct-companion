package app.isctcompanion.core

import android.annotation.SuppressLint
import android.app.Activity
import android.content.Context
import android.content.MutableContextWrapper
import android.graphics.Bitmap
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.os.Message
import android.util.Log
import android.view.View
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.RenderProcessGoneDetail
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.FrameLayout
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.setValue

/** `NativeBrowser` over one resident WebView per school site. Core drives
 * sign-in, navigation, extraction and history; this adapter creates, shows
 * and hides the views, executes dispatched commands on the main thread and
 * forwards WebView callbacks. Port methods arrive on Core workers and only
 * enqueue main-thread work. */
class SchoolBrowserHost(private val context: Context) : NativeBrowser {
    private val lock = Any()
    private var events: BrowserEvents? = null
    /** Last committed URL per view, readable from Core workers. */
    private val urls = mutableMapOf<BrowserId, String>()
    private var closedForGood = false
    private val main = Handler(Looper.getMainLooper())

    // Main-thread state.
    private val views = mutableMapOf<SchoolSite, ResidentView>()
    private var parking: FrameLayout? = null
    var onTitleChange: ((SchoolSite, String) -> Unit)? = null
    var onOpenExternal: ((Uri) -> Unit)? = null

    /** Bumped when a view appears or leaves, so containers re-attach. */
    var revision by mutableIntStateOf(0)
        private set

    /** Text scale for school pages, in percent; starts above 100 for readability. */
    var textScale = 115
        private set

    // NativeBrowser (Core worker threads)

    override fun attach(events: BrowserEvents) {
        synchronized(lock) { this.events = events }
    }

    override fun ensure(browser: BrowserId, initialUrl: String) {
        admit()
        main.post { create(browser, initialUrl) }
    }

    override fun dispatch(command: BrowserCommand) {
        admit()
        main.post { perform(command) }
    }

    override fun currentUrl(browser: BrowserId): String? = synchronized(lock) { urls[browser] }

    override fun dispose(browser: BrowserId) {
        main.post {
            discard(browser)
            runCatching { listener()?.closed(browser) }
        }
    }

    override fun shutdown() {
        synchronized(lock) {
            closedForGood = true
            urls.clear()
        }
        main.post {
            for (view in views.values) destroy(view.webView)
            views.clear()
        }
    }

    private fun admit() {
        if (synchronized(lock) { closedForGood }) throw SdkException.Closed()
    }

    private fun listener(): BrowserEvents? = synchronized(lock) { events }

    // Presentation (main thread)

    /** The resident view for `site`, if Core created one. */
    fun webView(site: SchoolSite): WebView? = views[site]?.webView

    /** The off-screen host that keeps views running while no screen shows them. */
    fun attachParking(activity: Activity) {
        val root = activity.findViewById<ViewGroup>(android.R.id.content)
        val host = parking ?: FrameLayout(context).apply {
            alpha = 0.01f
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
        }
        (host.parent as? ViewGroup)?.removeView(host)
        root.addView(host, 0, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        parking = host
        for (view in views.values) (view.webView.context as? MutableContextWrapper)?.baseContext = activity
    }

    /** Returns a view to the parking host so it keeps running. */
    fun park(webView: WebView) {
        val host = parking ?: return
        if (webView.parent === host) return
        (webView.parent as? ViewGroup)?.removeView(webView)
        host.addView(webView, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
    }

    /** Cycles 100 → 115 → 130 % on every resident page. */
    fun cycleTextScale() {
        textScale = if (textScale >= 130) 100 else textScale + 15
        for (view in views.values) view.webView.settings.textZoom = textScale
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun create(id: BrowserId, initialUrl: String) {
        if (views[id.site]?.id == id) return
        views.remove(id.site)?.let { destroy(it.webView) }
        val activity = (parking?.context as? Activity)
        val webView = WebView(MutableContextWrapper(activity ?: context))
        webView.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            javaScriptCanOpenWindowsAutomatically = true
            setSupportMultipleWindows(true)
            // Larger text without zooming the layout, so fixed-width pages still fit.
            textZoom = textScale
            builtInZoomControls = true
            displayZoomControls = false
            loadWithOverviewMode = true
            useWideViewPort = true
        }
        // One cookie store: visible pages and hidden pulls share a session; SSO
        // crosses sites.
        CookieManager.getInstance().setAcceptThirdPartyCookies(webView, true)
        val resident = ResidentView(id, webView)
        webView.webViewClient = resident.client
        webView.webChromeClient = resident.chrome
        views[id.site] = resident
        revision++
        park(webView)
        if (Uri.parse(initialUrl).scheme.isNullOrEmpty()) {
            discard(id)
            runCatching { listener()?.closed(id) }
            return
        }
        synchronized(lock) { urls[id] = initialUrl }
        webView.loadUrl(initialUrl)
    }

    private fun perform(command: BrowserCommand) {
        val id = command.browser
        val resident = views[id.site]?.takeIf { it.id == id }
        if (resident == null) {
            // A command for a view that no longer exists closes that generation.
            runCatching { listener()?.closed(id) }
            return
        }
        when (val action = command.action) {
            is BrowserAction.Navigate -> {
                if (Uri.parse(action.url).scheme.isNullOrEmpty()) {
                    fail(resident, command.actionId, "Invalid URL")
                } else {
                    resident.webView.loadUrl(action.url)
                }
            }
            is BrowserAction.Reload -> resident.webView.reload()
            // Scripts report through title and navigation relays.
            is BrowserAction.Evaluate -> resident.webView.evaluateJavascript(action.script, null)
        }
    }

    private fun fail(resident: ResidentView, cause: ULong?, detail: String) {
        runCatching { listener()?.loadFailed(resident.nextPage(), cause, null, detail) }
    }

    private fun discard(id: BrowserId) {
        val resident = views[id.site]?.takeIf { it.id == id } ?: return
        destroy(resident.webView)
        views.remove(id.site)
        revision++
        synchronized(lock) { urls.remove(id) }
    }

    private fun destroy(webView: WebView) {
        (webView.parent as? ViewGroup)?.removeView(webView)
        webView.destroy()
    }

    /** One site's WebView and its load sequence. */
    private inner class ResidentView(val id: BrowserId, val webView: WebView) {
        var sequence: ULong = 0u
            private set

        val page: PageId get() = PageId(id, sequence)

        fun nextPage(): PageId {
            sequence++
            return page
        }

        val client = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                val url = request.url
                if (url.scheme in setOf("about", "blob", "data", "javascript")) return false
                val decision = runCatching {
                    listener()?.navigation(page, url.toString(), request.isForMainFrame)
                }.getOrNull() ?: return true
                if (decision != NavigationDecision.ALLOW) Log.d(TAG, "Core cancelled ${url.host} in ${id.site}")
                return decision != NavigationDecision.ALLOW
            }

            override fun onPageStarted(view: WebView, url: String?, favicon: Bitmap?) {
                url?.let { synchronized(lock) { urls[id] = it } }
            }

            override fun onPageFinished(view: WebView, url: String?) {
                url ?: return
                synchronized(lock) { urls[id] = url }
                // Android injects Core's shared scripts after load (core-ffi.md);
                // the shim installs itself into same-origin frames.
                for (script in runCatching { schoolBrowserInitialization() }.getOrDefault(emptyList())) {
                    view.evaluateJavascript(script, null)
                }
                // WebView exposes no request id here; never label a late load with an action.
                runCatching { listener()?.pageLoaded(nextPage(), null, url) }
            }

            override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
                if (!request.isForMainFrame) return
                // A navigation cancelled by Core's own decision is not a failure.
                if (error.errorCode == ERROR_UNKNOWN && error.description == "net::ERR_ABORTED") return
                runCatching {
                    listener()?.loadFailed(nextPage(), null, request.url.toString(), error.description.toString())
                }
            }

            override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean {
                discard(id)
                runCatching { listener()?.closed(id) }
                return true
            }
        }

        val chrome = object : WebChromeClient() {
            override fun onReceivedTitle(view: WebView, title: String?) {
                if (title.isNullOrEmpty()) return
                runCatching { listener()?.titleChanged(page, title) }
                onTitleChange?.invoke(id.site, title)
            }

            override fun onCreateWindow(view: WebView, isDialog: Boolean, isUserGesture: Boolean, resultMsg: Message): Boolean {
                // WebView only names the popup's URL once a view loads it: catch
                // that first request, then let Core decide where it goes.
                val probe = WebView(view.context)
                probe.webViewClient = object : WebViewClient() {
                    override fun shouldOverrideUrlLoading(probeView: WebView, request: WebResourceRequest): Boolean {
                        popup(request.url)
                        probeView.destroy()
                        return true
                    }
                }
                val transport = resultMsg.obj as? WebView.WebViewTransport ?: return false
                transport.webView = probe
                resultMsg.sendToTarget()
                return true
            }
        }

        fun popup(url: Uri) {
            val decision = runCatching { listener()?.newWindow(page, url.toString()) }.getOrNull() ?: return
            Log.d(TAG, "Popup to ${url.host}: $decision")
            when (decision) {
                is NewWindowDecision.SameView -> webView.loadUrl(decision.url)
                // Core already emitted `school-web-open`; the router presents it.
                is NewWindowDecision.OpenSchool -> Unit
                is NewWindowDecision.OpenExternal -> onOpenExternal?.invoke(Uri.parse(decision.url))
                is NewWindowDecision.Ignore -> Unit
            }
        }
    }

    private companion object {
        const val TAG = "school-web"
    }
}
