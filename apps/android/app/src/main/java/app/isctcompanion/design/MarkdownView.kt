package app.isctcompanion.design

import android.annotation.SuppressLint
import android.content.ActivityNotFoundException
import android.content.Intent
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.viewinterop.AndroidView
import org.json.JSONObject

/** Whether a document's name marks it as Markdown (the exam site stores it as
 * text/plain). */
fun isMarkdownName(name: String): Boolean =
    name.substringAfterLast('.', "").lowercase() in setOf("md", "markdown")

/**
 * A Markdown document, read-only, with its math set by KaTeX: the page in
 * `resources/markdown-viewer` (built from the desktop's reader rules), which
 * the app bundles in its assets. It loads nothing over the network but images
 * the document links; a link opens in the browser. Raw HTML in the document
 * is shown as text, never run.
 */
@SuppressLint("SetJavaScriptEnabled")
@Composable
fun MarkdownView(text: String, modifier: Modifier = Modifier) {
    val colors = Theme.colors
    val dark = isSystemInDarkTheme()
    val fontScale = LocalDensity.current.fontScale
    val look = JSONObject()
        .put("background", colors.surface.css())
        .put("text", colors.ink.css())
        .put("muted", colors.muted.css())
        .put("line", colors.line.css())
        .put("accent", colors.accent.css())
        .put("surface", colors.surface2.css())
        .put("fontSize", 16)
        .put("dark", dark)
    val script = "isctMarkdown.show(${JSONObject.quote(text)}, $look)"
    val page = remember { MarkdownPage() }
    page.script = script
    AndroidView(
        modifier = modifier,
        factory = { context ->
            WebView(context).apply {
                settings.javaScriptEnabled = true
                settings.allowContentAccess = false
                settings.allowFileAccess = false
                settings.textZoom = (fontScale * 100).toInt()
                setBackgroundColor(colors.surface.toArgb())
                webViewClient = object : WebViewClient() {
                    override fun onPageFinished(view: WebView, url: String?) {
                        page.loaded = true
                        page.shown = page.script
                        view.evaluateJavascript(page.script, null)
                    }

                    override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                        val url = request.url
                        if (url.scheme == "http" || url.scheme == "https" || url.scheme == "mailto") {
                            try {
                                view.context.startActivity(Intent(Intent.ACTION_VIEW, url))
                            } catch (_: ActivityNotFoundException) {
                            }
                        }
                        return true
                    }
                }
                loadUrl("file:///android_asset/markdown-viewer.html")
            }
        },
        update = { view ->
            view.setBackgroundColor(colors.surface.toArgb())
            view.settings.textZoom = (fontScale * 100).toInt()
            if (page.loaded && page.shown != script) {
                page.shown = script
                view.evaluateJavascript(script, null)
            }
        },
        onRelease = { it.destroy() },
    )
}

/** What the page shows: the latest call, and whether the page can take it. */
private class MarkdownPage {
    var script = ""
    var shown = ""
    var loaded = false
}

private fun Color.css(): String = "#%06X".format(toArgb() and 0xFFFFFF)
