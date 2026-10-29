package app.isctcompanion.core

import android.annotation.SuppressLint
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.MutableContextWrapper
import android.graphics.Bitmap
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.os.Message
import android.print.PrintAttributes
import android.print.PrintManager
import android.view.View
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.DownloadListener
import android.webkit.JsPromptResult
import android.webkit.JsResult
import android.webkit.MimeTypeMap
import android.webkit.RenderProcessGoneDetail
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.FrameLayout
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

/** `NativeBrowser` over one resident WebView per school site and the popups
 * its pages open (docs/core-school-browser.md). Core decides every
 * navigation and window, and what Back and Forward do; this adapter reports
 * what WebView tells it (each view's and popup's history included), creates,
 * shows and hides the views, executes dispatched commands on the main
 * thread, and shows page dialogs for the view on screen. Port methods arrive
 * on Core workers and only enqueue main-thread work. */
class SchoolBrowserHost(private val context: Context) : NativeBrowser {
    private val lock = Any()
    private var events: BrowserEvents? = null
    /** Last committed URL per view or popup, readable from Core workers. */
    private val urls = mutableMapOf<BrowserId, String>()
    private var closedForGood = false
    private val main = Handler(Looper.getMainLooper())

    // Main-thread state.
    private val views = mutableMapOf<SchoolSite, Tab>()
    private val tabs = mutableMapOf<BrowserId, Tab>()
    private var parking: FrameLayout? = null
    /** The activity in front. Views outlive it, so each holds it through a
     * MutableContextWrapper: WebView adds the windows for a page's own
     * controls (a `<select>` list, a date picker) with that context, and an
     * application context has no window token to add them under. */
    private var activity: Activity? = null
    var onOpenExternal: ((Uri) -> Unit)? = null
    /** A file a page handed over: its URL, the linking page, and WebView's
     * Content-Disposition and MIME type. Core fetches and names it. */
    var onDownload: ((url: String, referer: String?, contentDisposition: String?, mimeType: String?) -> Unit)? = null

    /** Bumped when a view or popup appears or leaves, so containers re-attach. */
    var revision by mutableIntStateOf(0)
        private set

    /** The page dialog waiting for the person, if the page on screen asked. */
    var dialog by mutableStateOf<PageDialog?>(null)
        private set

    /** The files the page on screen asked to choose, while its picker is
     * due or open. */
    var files by mutableStateOf<FileRequest?>(null)
        private set

    /** Where the chosen text scale is kept on this device (設定 › ウェブ). */
    private val preferences = context.getSharedPreferences("web", Context.MODE_PRIVATE)

    /** Text scale for school pages, in percent; starts above 100 for readability. */
    var textScale by mutableIntStateOf(
        preferences.getInt(TEXT_SCALE, 115).takeIf { it in textScales } ?: 115,
    )
        private set

    // NativeBrowser (Core worker threads)

    /** `onFormResubmission` reports a reload that would resend a form, and
     * popups show inside the web screen over their opener. */
    override fun capabilities() = HostCapabilities(formResubmission = true, inlinePopups = true)

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
            tabs[browser]?.let(::discard)
            runCatching { listener()?.closed(browser) }
        }
    }

    override fun shutdown() {
        synchronized(lock) {
            closedForGood = true
            urls.clear()
        }
        main.post {
            for (view in views.values.toList()) discard(view)
        }
    }

    private fun admit() {
        if (synchronized(lock) { closedForGood }) throw SdkException.Closed()
    }

    private fun listener(): BrowserEvents? = synchronized(lock) { events }

    // Presentation (main thread)

    /** The resident view for `site`, if Core created one. */
    fun webView(site: SchoolSite): WebView? = views[site]?.webView

    /** What the person sees for `site`: its newest popup, else its view. */
    fun front(site: SchoolSite): WebView? = views[site]?.front?.webView

    /** Closes the popup in front of `site`'s view, as its close bar does. */
    fun closeFront(site: SchoolSite) {
        views[site]?.popups?.lastOrNull()?.let(::close)
    }

    /** Prints the page on screen through the system print panel. Pages that
     * call `window.print()` reach nothing: WebView has no hook for it, and
     * no script is injected to add one. */
    fun print(site: SchoolSite, activity: Context) {
        val webView = front(site) ?: return
        val name = webView.title?.takeIf { it.isNotBlank() } ?: WebRouter.name(site)
        activity.getSystemService(PrintManager::class.java)
            ?.print(name, webView.createPrintDocumentAdapter(name), PrintAttributes.Builder().build())
    }

    /** The off-screen host that keeps views running while no screen shows them. */
    fun attachParking(activity: Activity) {
        val root = activity.findViewById<ViewGroup>(android.R.id.content)
        val host = parking ?: FrameLayout(context).apply {
            alpha = 0.01f
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
            // Core's login script focuses fields it fills; a hidden page must
            // not take focus and raise the soft keyboard.
            descendantFocusability = ViewGroup.FOCUS_BLOCK_DESCENDANTS
        }
        (host.parent as? ViewGroup)?.removeView(host)
        root.addView(host, 0, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        parking = host
        this.activity = activity
        for (tab in tabs.values) (tab.webView.context as? MutableContextWrapper)?.baseContext = activity
    }

    /** Lets go of a destroyed activity, so no view keeps it alive. */
    fun detachActivity(activity: Activity) {
        if (this.activity !== activity) return
        this.activity = null
        for (tab in tabs.values) (tab.webView.context as? MutableContextWrapper)?.baseContext = context
    }

    /** Returns a view to the parking host so it keeps running. */
    fun park(webView: WebView) {
        val host = parking ?: return
        if (webView.parent === host) return
        webView.clearFocus()
        (webView.parent as? ViewGroup)?.removeView(webView)
        host.addView(webView, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
    }

    /** Applies and keeps a text scale on every resident page. */
    fun chooseTextScale(percent: Int) {
        textScale = percent
        preferences.edit().putInt(TEXT_SCALE, percent).apply()
        for (tab in tabs.values) tab.webView.settings.textZoom = percent
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun newWebView(): WebView {
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
        // One cookie store: visible pages and hidden sync reads share a session; SSO
        // crosses sites.
        CookieManager.getInstance().setAcceptThirdPartyCookies(webView, true)
        return webView
    }

    private fun create(id: BrowserId, initialUrl: String) {
        if (views[id.site]?.id == id) return
        views[id.site]?.let(::discard)
        val view = Tab(id, newWebView(), opener = null)
        views[id.site] = view
        register(view)
        if (Uri.parse(initialUrl).scheme.isNullOrEmpty()) {
            close(view)
            return
        }
        synchronized(lock) { urls[id] = initialUrl }
        view.webView.loadUrl(initialUrl)
    }

    /** A window Core admitted as `id`, handed to WebView to load the page's
     * own request into, so a posted form and `window.opener` survive. */
    private fun openPopup(id: BrowserId, opener: Tab): Tab {
        val popup = Tab(id, newWebView(), opener)
        popup.root.popups.add(popup)
        register(popup)
        // No history yet: Core's Back closes it until it has some.
        popup.reportHistory()
        return popup
    }

    private fun register(tab: Tab) {
        tabs[tab.id] = tab
        tab.webView.webViewClient = tab.client
        tab.webView.webChromeClient = tab.chrome
        tab.webView.setDownloadListener(tab.downloads)
        revision++
        park(tab.webView)
    }

    private fun perform(command: BrowserCommand) {
        val tab = tabs[command.browser]
        if (tab == null) {
            // A command for a view that no longer exists closes that generation.
            runCatching { listener()?.closed(command.browser) }
            return
        }
        val webView = tab.webView
        when (val action = command.action) {
            is BrowserAction.Navigate -> {
                if (Uri.parse(action.url).scheme.isNullOrEmpty()) {
                    fail(tab, command.actionId, "Invalid URL")
                } else {
                    webView.loadUrl(action.url)
                }
            }
            // A page that would resend a form asks onFormResubmission first.
            is BrowserAction.Reload -> webView.reload()
            is BrowserAction.GoBack -> webView.goBack()
            is BrowserAction.GoForward -> webView.goForward()
            // Scripts report through title and navigation relays.
            is BrowserAction.Evaluate -> webView.evaluateJavascript(action.script, null)
            // Core abandoned this view's attempt and leases it again only once
            // this confirms; the page reported at that point is the last one
            // the attempt may claim.
            is BrowserAction.Stop -> {
                webView.stopLoading()
                webView.evaluateJavascript(action.script) {
                    runCatching { listener()?.stopped(tab.page, command.actionId) }
                }
            }
        }
    }

    private fun fail(tab: Tab, cause: ULong?, detail: String) {
        runCatching { listener()?.loadFailed(tab.nextPage(), cause, null, LoadFailure.OTHER, detail) }
    }

    /** Closes `tab` for good and tells Core. Core closes the popups it opened. */
    private fun close(tab: Tab) {
        discard(tab)
        runCatching { listener()?.closed(tab.id) }
    }

    /** Destroys `tab` and every popup opened from it. */
    private fun discard(tab: Tab) {
        val root = tab.root
        val gone = root.popups.filter { it.descendsFrom(tab) } + listOfNotNull(root.takeIf { it === tab })
        for (each in gone) {
            if (dialog?.browser == each.id) dialog?.answer(false, null)
            if (files?.browser == each.id) files?.answer(null)
            tabs.remove(each.id)
            synchronized(lock) { urls.remove(each.id) }
            (each.webView.parent as? ViewGroup)?.removeView(each.webView)
            each.webView.destroy()
        }
        root.popups.removeAll(gone)
        if (tab === root) views.remove(root.id.site)
        revision++
    }

    /** Whether the person has `tab`'s site on screen, whichever of its pages
     * is in front: a popup opened for a file may not be attached yet. */
    private fun siteShown(tab: Tab): Boolean =
        (listOf(tab.root) + tab.root.popups).any {
            val parent = it.webView.parent
            parent != null && parent !== parking && it.webView.isShown
        }

    private fun decide(request: NavigationRequest): NavigationDecision? =
        runCatching { listener()?.navigation(request) }.getOrNull()

    /** Whether the person is looking at `tab`: the front page of a site
     * whose screen holds it. */
    private fun shown(tab: Tab): Boolean {
        val parent = tab.webView.parent ?: return false
        return tab.root.front === tab && parent !== parking && tab.webView.isShown
    }

    /** A view or popup, its load sequence and its WebView callbacks. */
    private inner class Tab(val id: BrowserId, val webView: WebView, val opener: Tab?) {
        val root: Tab = opener?.root ?: this

        /** On a resident view: its popups and theirs, oldest first. */
        val popups = mutableListOf<Tab>()

        val front: Tab get() = popups.lastOrNull() ?: this

        var sequence: ULong = 0u
            private set

        val page: PageId get() = PageId(id, sequence)

        fun nextPage(): PageId {
            sequence++
            return page
        }

        /** The address whose main-frame load failed. WebView still commits
         * and finishes an error page for it, which is no page to report;
         * matched by address because WebView orders the error and the error
         * page's start differently across versions. */
        private var failedUrl: String? = null

        /** A page loaded here; a popup that never got one is empty. */
        private var loaded = false

        fun descendsFrom(ancestor: Tab): Boolean = generateSequence(this) { it.opener }.any { it === ancestor }

        /** Where this view's or popup's own history can go, for Core's Back
         * and Forward. */
        fun reportHistory() {
            runCatching { listener()?.historyChanged(id, webView.canGoBack(), webView.canGoForward()) }
        }

        /** A popup whose first page went to the system browser or was refused
         * stays blank; close it as a browser closes such a tab. */
        private fun closeIfEmpty(request: WebResourceRequest) {
            if (opener != null && !loaded && request.isForMainFrame) main.post { if (tabs[id] === this) close(this) }
        }

        val client = object : WebViewClient() {
            // WebView asks here for links, GET forms, scripts and redirects in
            // any frame, never for a POST, back/forward or reload.
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                val navigation = NavigationRequest(
                    page,
                    request.url.toString(),
                    request.method,
                    request.isForMainFrame,
                    if (request.isRedirect) NavigationKind.Other else NavigationKind.Unknown,
                    request.hasGesture(),
                )
                return when (val decision = decide(navigation)) {
                    is NavigationDecision.Allow -> false
                    is NavigationDecision.External -> {
                        onOpenExternal?.invoke(Uri.parse(decision.url))
                        closeIfEmpty(request)
                        true
                    }
                    // SkipEntry answers only a resubmission from history,
                    // which WebView never makes (it fails with ERR_CACHE_MISS).
                    is NavigationDecision.Deny, is NavigationDecision.SkipEntry, null -> {
                        closeIfEmpty(request)
                        true
                    }
                }
            }

            override fun onFormResubmission(view: WebView, dontResend: Message, resend: Message) {
                val navigation = NavigationRequest(
                    page, view.url.orEmpty(), "POST", true, NavigationKind.FormResubmit(fromHistory = false), false,
                )
                // Core loads the page again with GET when it refuses.
                if (decide(navigation) is NavigationDecision.Allow) resend.sendToTarget() else dontResend.sendToTarget()
            }

            override fun onPageStarted(view: WebView, url: String?, favicon: Bitmap?) {
                if (url != failedUrl) failedUrl = null
                url?.let { synchronized(lock) { urls[id] = it } }
            }

            override fun onPageFinished(view: WebView, url: String?) {
                url ?: return
                if (url == failedUrl) {
                    failedUrl = null
                    return
                }
                synchronized(lock) { urls[id] = url }
                if (url != BLANK) loaded = true
                // WebView exposes no request id here; never label a late load with an action.
                runCatching { listener()?.pageLoaded(nextPage(), null, url) }
            }

            // Every committed entry, same-document ones (`pushState`) included.
            override fun doUpdateVisitedHistory(view: WebView, url: String?, isReload: Boolean) {
                reportHistory()
            }

            override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
                if (!request.isForMainFrame) return
                val url = request.url.toString()
                val detail = error.description.toString()
                val reason = failure(error.errorCode, detail)
                // Nothing commits for a cancelled load.
                if (reason != LoadFailure.CANCELLED) failedUrl = url
                runCatching { listener()?.loadFailed(nextPage(), null, url, reason, detail) }
            }

            override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean {
                if (tabs[id] === this@Tab) close(this@Tab)
                return true
            }
        }

        /** WebView hands over a response it cannot show; Core downloads it
         * (`onDownload`), never the system DownloadManager. Only a site the
         * person is looking at saves files: Core's `download_permitted` would
         * let a hidden view save adapter-site files, but it is not in the FFI
         * and no hidden work on Android needs one. */
        val downloads = DownloadListener { url, _, contentDisposition, mimeType, _ ->
            val save = onDownload
            if (!siteShown(this) || save == null) return@DownloadListener
            // The page that linked the file; a popup opened for it has none yet.
            val referer = generateSequence(this) { it.opener }
                .firstNotNullOfOrNull { tab -> tab.webView.url?.takeIf { it.startsWith("https:") } }
            save(url, referer, contentDisposition, mimeType)
            // A popup opened only for this file has nothing left; Core closes it.
            runCatching { listener()?.becameDownload(page, loaded, DownloadDisposition.SAVED) }
        }

        val chrome = object : WebChromeClient() {
            override fun onReceivedTitle(view: WebView, title: String?) {
                if (title.isNullOrEmpty()) return
                runCatching { listener()?.titleChanged(page, title) }
            }

            // WebView names no URL until the window navigates; that first
            // navigation reaches Core from the popup like any other.
            override fun onCreateWindow(view: WebView, isDialog: Boolean, isUserGesture: Boolean, resultMsg: Message): Boolean {
                val transport = resultMsg.obj as? WebView.WebViewTransport ?: return false
                val request = NewWindowRequest(page, BLANK, null, NavigationKind.Unknown, isUserGesture, null, null, null)
                return when (val decision = runCatching { listener()?.newWindow(request) }.getOrNull()) {
                    is NewWindowDecision.Popup -> {
                        transport.webView = openPopup(decision.browser, this@Tab).webView
                        resultMsg.sendToTarget()
                        true
                    }
                    is NewWindowDecision.External -> {
                        onOpenExternal?.invoke(Uri.parse(decision.url))
                        false
                    }
                    is NewWindowDecision.Deny, null -> false
                }
            }

            override fun onCloseWindow(window: WebView) {
                if (opener != null && tabs[id] === this@Tab) close(this@Tab)
            }

            override fun onJsAlert(view: WebView, url: String, message: String, result: JsResult) =
                ask(PageDialog.Kind.ALERT, url, message, null, result)

            override fun onJsConfirm(view: WebView, url: String, message: String, result: JsResult) =
                ask(PageDialog.Kind.CONFIRM, url, message, null, result)

            override fun onJsPrompt(view: WebView, url: String, message: String, defaultValue: String?, result: JsPromptResult) =
                ask(PageDialog.Kind.PROMPT, url, message, defaultValue, result)

            override fun onJsBeforeUnload(view: WebView, url: String, message: String, result: JsResult) =
                ask(PageDialog.Kind.LEAVE, url, message, null, result)

            /** A file input opens the system picker for the page on screen,
             * as a dialog does; a hidden page chooses nothing. */
            override fun onShowFileChooser(
                view: WebView,
                callback: ValueCallback<Array<Uri>>,
                params: FileChooserParams,
            ): Boolean {
                if (!shown(this@Tab) || files != null) {
                    callback.onReceiveValue(null)
                    return true
                }
                files = FileRequest(id, chooser(params)) { uris ->
                    files = null
                    callback.onReceiveValue(uris)
                }
                return true
            }
        }

        /** Shows a page dialog for the page on screen. A hidden page is
         * answered at once without UI: Cancel, except that it may always
         * leave, so hidden work is never held on a page. */
        private fun ask(kind: PageDialog.Kind, url: String, message: String, value: String?, result: JsResult): Boolean {
            if (!shown(this) || dialog != null) {
                if (kind == PageDialog.Kind.LEAVE) result.confirm() else result.cancel()
                return true
            }
            dialog = PageDialog(id, kind, origin(url), message, value) { ok, text ->
                dialog = null
                when {
                    !ok -> result.cancel()
                    result is JsPromptResult -> result.confirm(text.orEmpty())
                    else -> result.confirm()
                }
            }
            return true
        }
    }

    /** A dialog a page asked for. [answer] runs once: OK with the prompt's
     * text, or Cancel. */
    class PageDialog(
        val browser: BrowserId,
        val kind: Kind,
        /** The asking frame's origin, as browsers title these. */
        val origin: String,
        val message: String,
        val value: String?,
        answer: (Boolean, String?) -> Unit,
    ) {
        enum class Kind { ALERT, CONFIRM, PROMPT, LEAVE }

        private var reply: ((Boolean, String?) -> Unit)? = answer

        fun answer(ok: Boolean, text: String?) {
            val reply = reply ?: return
            this.reply = null
            reply(ok, text)
        }
    }

    /** Files a page's file input asked for: the picker [intent] to launch,
     * once, and [answer], which runs once with the chosen files or null. */
    class FileRequest(val browser: BrowserId, val intent: Intent, answer: (Array<Uri>?) -> Unit) {
        /** Set once the picker is up, so a recreated screen does not open
         * it again; its result reaches the new screen. */
        var launched = false

        private var reply: ((Array<Uri>?) -> Unit)? = answer

        fun answer(uris: Array<Uri>?) {
            val reply = reply ?: return
            this.reply = null
            reply(uris)
        }
    }

    companion object {
        private const val TEXT_SCALE = "textScale"
        private const val BLANK = "about:blank"

        /** The text scales offered for school pages, in percent. */
        val textScales = listOf(100, 115, 130)

        /** The system picker for a file input. WebView's own
         * `FileChooserParams.createIntent` keeps only the first accepted type,
         * passes an extension such as `.pdf` as a MIME type and never asks
         * for several files, so the intent is built here: extensions are
         * looked up, and an unknown one accepts anything rather than nothing. */
        private fun chooser(params: WebChromeClient.FileChooserParams): Intent {
            val accepted = params.acceptTypes.flatMap { it.split(',') }.map { it.trim().lowercase() }.filter { it.isNotEmpty() }
            val types = accepted.map { if (it.startsWith('.')) MimeTypeMap.getSingleton().getMimeTypeFromExtension(it.drop(1)) else it }
            val mimes = if (types.isEmpty() || null in types) listOf("*/*") else types.filterNotNull().distinct()
            return Intent(Intent.ACTION_GET_CONTENT)
                .addCategory(Intent.CATEGORY_OPENABLE)
                .setType(mimes.singleOrNull() ?: "*/*")
                .putExtra(Intent.EXTRA_MIME_TYPES, mimes.toTypedArray())
                .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, params.mode == WebChromeClient.FileChooserParams.MODE_OPEN_MULTIPLE)
        }

        /** The files a picker returned, one or several; null when it was
         * closed without a choice. */
        fun chosen(resultCode: Int, data: Intent?): Array<Uri>? {
            if (resultCode != Activity.RESULT_OK || data == null) return null
            val clip = data.clipData
            val uris = if (clip != null) (0 until clip.itemCount).mapNotNull { clip.getItemAt(it).uri } else listOfNotNull(data.data)
            return uris.toTypedArray().takeIf { it.isNotEmpty() }
        }

        private fun origin(url: String): String {
            val uri = Uri.parse(url)
            val host = uri.host ?: return url
            return "${uri.scheme}://$host" + if (uri.port != -1) ":${uri.port}" else ""
        }

        /** WebView's main-frame error, in Core's terms. Chromium's net error
         * name is more precise than the code for a few, so it is read first.
         * ERR_CACHE_MISS (back or forward to a form result WebView will not
         * resend) is `Other`: the notice's Retry reloads it, which asks
         * onFormResubmission, and Core opens it again with GET. */
        private fun failure(code: Int, detail: String): LoadFailure = when {
            "ERR_ABORTED" in detail -> LoadFailure.CANCELLED
            "ERR_INTERNET_DISCONNECTED" in detail -> LoadFailure.OFFLINE
            "ERR_CLEARTEXT_NOT_PERMITTED" in detail -> LoadFailure.CLEARTEXT
            // A certificate WebView's default SSL handling refused.
            "ERR_CERT_" in detail || "ERR_SSL_" in detail -> LoadFailure.INSECURE
            else -> when (code) {
                WebViewClient.ERROR_HOST_LOOKUP -> LoadFailure.HOST_NOT_FOUND
                WebViewClient.ERROR_CONNECT, WebViewClient.ERROR_TIMEOUT, WebViewClient.ERROR_IO -> LoadFailure.UNREACHABLE
                WebViewClient.ERROR_FAILED_SSL_HANDSHAKE -> LoadFailure.INSECURE
                else -> LoadFailure.OTHER
            }
        }
    }
}
