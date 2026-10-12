package app.isctcompanion.schoolweb

import android.app.Activity
import android.graphics.Bitmap
import android.net.Uri
import android.os.Message
import android.view.View
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.inputmethod.InputMethodManager
import android.webkit.CookieManager
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.FrameLayout
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class Bounds {
    var x: Double = 0.0
    var y: Double = 0.0
    var width: Double = 1.0
    var height: Double = 1.0
}
@InvokeArg
class ViewArgs {
    lateinit var operation: String
    lateinit var site: String
    var url: String? = null
    var value: String? = null
    var bounds: Bounds? = null
    var visible: Boolean = false
    var scripts: List<String> = emptyList()
    var allowedHosts: List<String> = emptyList()
    var events: Channel? = null
}
private class Resident(val view: WebView, val events: Channel, val scripts: List<String>, val hosts: Set<String>) {
    var lastBounds: Bounds? = null
    var zoom: Double = 1.0
}

/** All views are ordinary Android WebViews, without Tauri's JS bridge. */
@TauriPlugin
class SchoolWebPlugin(private val activity: Activity) : Plugin(activity) {
    private var shell: WebView? = null
    private var overlay: FrameLayout? = null
    private val views = mutableMapOf<String, Resident>()

    override fun load(webView: WebView) { shell = webView }
    private fun event(resident: Resident, kind: String, value: String) {
        resident.events.send(JSObject().put("kind", kind).put("value", value))
    }
    private fun allowed(resident: Resident, raw: String): Boolean {
        val url = Uri.parse(raw)
        return url.scheme == "https" && url.host?.lowercase() in resident.hosts && url.userInfo == null
    }
    private fun navigation(resident: Resident, raw: String): Boolean {
        // Extraction relays are always cancelled here; Rust authenticates the
        // nonce before processing them. Ordinary redirects keep their query/POST.
        event(resident, "navigation", raw)
        return !allowed(resident, raw)
    }
    private fun root(): FrameLayout {
        return overlay ?: FrameLayout(activity).also {
            it.isClickable = false
            activity.addContentView(it, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
            overlay = it
            it.addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ ->
                for (resident in views.values) resident.lastBounds?.let { value -> bounds(resident, value) }
            }
        }
    }
    @Suppress("SetJavaScriptEnabled")
    private fun create(args: ViewArgs): Resident {
        views[args.site]?.let { return it }
        val view = WebView(activity)
        val resident = Resident(view, requireNotNull(args.events), args.scripts, args.allowedHosts.toSet())
        view.settings.javaScriptEnabled = true
        view.settings.domStorageEnabled = true
        view.settings.setSupportMultipleWindows(true)
        view.settings.javaScriptCanOpenWindowsAutomatically = true
        view.settings.allowFileAccess = false
        view.settings.allowContentAccess = false
        CookieManager.getInstance().setAcceptCookie(true)
        CookieManager.getInstance().setAcceptThirdPartyCookies(view, true)
        view.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean = navigation(resident, request.url.toString())
            @Deprecated("Legacy redirects on older Android WebViews")
            override fun shouldOverrideUrlLoading(view: WebView, url: String): Boolean = navigation(resident, url)
            override fun onPageFinished(view: WebView, url: String) {
                if (view.url != url || !allowed(resident, url)) return
                // Remote-page initialization is not reliable on Android. Run
                // support scripts after load, then let Rust inject login/pull.
                for (script in resident.scripts) view.evaluateJavascript(script, null)
                view.evaluateJavascript("document.documentElement.style.zoom = '${resident.zoom}'", null)
                CookieManager.getInstance().flush()
                event(resident, "loaded", url)
            }
        }
        view.webChromeClient = object : WebChromeClient() {
            override fun onReceivedTitle(view: WebView, title: String?) { title?.let { event(resident, "title", it) } }
            override fun onCreateWindow(view: WebView, isDialog: Boolean, isUserGesture: Boolean, resultMsg: Message): Boolean {
                val popup = WebView(activity)
                popup.webViewClient = object : WebViewClient() {
                    private fun route(url: String): Boolean {
                        event(resident, "newWindow", url)
                        popup.post { popup.destroy() }
                        return true
                    }
                    override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean = route(request.url.toString())
                    @Deprecated("Legacy popup redirects")
                    override fun shouldOverrideUrlLoading(view: WebView, url: String): Boolean = route(url)
                    override fun onPageStarted(view: WebView, url: String, favicon: Bitmap?) { if (url != "about:blank") route(url) }
                }
                (resultMsg.obj as WebView.WebViewTransport).webView = popup
                resultMsg.sendToTarget()
                return true
            }
        }
        root().addView(view, FrameLayout.LayoutParams(1, 1))
        view.visibility = if (args.visible) View.VISIBLE else View.INVISIBLE
        args.bounds?.let { bounds(resident, it) }
        views[args.site] = resident
        val url = requireNotNull(args.url)
        require(allowed(resident, url)) { "School URL is outside the allowed official origins." }
        view.loadUrl(url)
        return resident
    }
    @Suppress("DEPRECATION")
    private fun bounds(resident: Resident, bounds: Bounds) {
        resident.lastBounds = bounds
        val root = root()
        if (root.width == 0 || root.height == 0) return
        val shell = requireNotNull(shell) { "Main WebView is unavailable." }
        val shellPosition = IntArray(2)
        val rootPosition = IntArray(2)
        shell.getLocationInWindow(shellPosition)
        root.getLocationInWindow(rootPosition)
        // getScale includes device density and the shell's viewport scale.
        val scale = shell.scale.toDouble().takeIf { it.isFinite() && it > 0 } ?: activity.resources.displayMetrics.density.toDouble()
        val insets = root.rootWindowInsets?.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout() or WindowInsets.Type.ime())
        val window = activity.window.decorView
        val safeLeft = (insets?.left ?: 0) - rootPosition[0]
        val safeTop = (insets?.top ?: 0) - rootPosition[1]
        val safeRight = window.width - (insets?.right ?: 0) - rootPosition[0]
        val safeBottom = window.height - (insets?.bottom ?: 0) - rootPosition[1]
        val rect = SchoolFrame.calculate(bounds.x, bounds.y, bounds.width, bounds.height, scale,
            shellPosition[0] - rootPosition[0], shellPosition[1] - rootPosition[1],
            safeLeft.coerceAtLeast(0), safeTop.coerceAtLeast(0), safeRight.coerceAtMost(root.width), safeBottom.coerceAtMost(root.height))
        resident.view.layoutParams = FrameLayout.LayoutParams(rect.width, rect.height).also { it.leftMargin = rect.x; it.topMargin = rect.y }
    }
    @Command
    fun operate(invoke: Invoke) {
        val args = invoke.parseArgs(ViewArgs::class.java)
        activity.runOnUiThread {
            try {
                val resident = if (args.operation == "create") create(args) else requireNotNull(views[args.site]) { "School view is unavailable." }
                when (args.operation) {
                    "create" -> Unit
                    "bounds" -> bounds(resident, requireNotNull(args.bounds))
                    "eval" -> resident.view.evaluateJavascript(requireNotNull(args.value), null)
                    "navigate" -> { val url = requireNotNull(args.value); require(allowed(resident, url)); resident.view.loadUrl(url) }
                    "reload" -> resident.view.reload()
                    "hide" -> { (activity.getSystemService(Activity.INPUT_METHOD_SERVICE) as InputMethodManager).hideSoftInputFromWindow(resident.view.windowToken, 0); resident.view.clearFocus(); resident.view.visibility = View.INVISIBLE; CookieManager.getInstance().flush() }
                    "show" -> { resident.view.visibility = View.VISIBLE; resident.view.bringToFront() }
                    "focus" -> resident.view.requestFocus()
                    "zoom" -> { val scale = requireNotNull(args.value).toDouble(); require(scale in 0.5..2.0); resident.zoom = scale; resident.view.evaluateJavascript("document.documentElement.style.zoom = '$scale'", null) }
                    else -> error("Unknown school view operation.")
                }
                invoke.resolve(JSObject().put("ok", true))
            } catch (error: Exception) { invoke.reject(error.message ?: "School view operation failed.") }
        }
    }
}
