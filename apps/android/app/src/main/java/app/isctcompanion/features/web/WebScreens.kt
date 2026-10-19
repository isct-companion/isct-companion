package app.isctcompanion.features.web

import app.isctcompanion.design.t
import android.content.Intent
import android.net.Uri
import android.view.ViewGroup
import android.webkit.WebView
import android.widget.FrameLayout
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.outlined.OpenInNew
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material.icons.outlined.MoreHoriz
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.Share
import androidx.compose.material.icons.automirrored.outlined.ArrowForward
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.FormatSize
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ScrollableTabRow
import androidx.compose.material3.Tab
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.SchoolBrowserHost
import app.isctcompanion.core.SchoolSite
import app.isctcompanion.core.WebRouter
import app.isctcompanion.design.Theme

/** Hosts a site's resident WebView while on screen and parks it afterwards,
 * so its page, scroll position and session survive tab and route changes. */
@Composable
private fun ResidentWebView(site: SchoolSite, host: SchoolBrowserHost, revision: Int, modifier: Modifier = Modifier) {
    // Read so a newly created view is attached.
    val created = host.revision
    AndroidView(
        factory = { context -> FrameLayout(context) },
        update = { container ->
            revision + created
            val webView = host.webView(site)
            val shown = (0 until container.childCount).map { container.getChildAt(it) }
            for (child in shown) if (child !== webView && child is WebView) host.park(child)
            if (webView != null && webView.parent !== container) {
                (webView.parent as? ViewGroup)?.removeView(webView)
                container.addView(webView, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
            }
        },
        onRelease = { container ->
            (0 until container.childCount).map { container.getChildAt(it) }.filterIsInstance<WebView>().forEach(host::park)
        },
        modifier = modifier,
    )
}

/** The overflow menu every school page carries. */
@Composable
private fun PageMenu(site: SchoolSite) {
    val web = LocalModel.current.web
    val context = LocalContext.current
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) { Icon(Icons.Outlined.MoreVert, t("More")) }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            val item = @Composable { label: String, icon: ImageVector, action: () -> Unit ->
                DropdownMenuItem(text = { Text(label) }, leadingIcon = { Icon(icon, null) }, onClick = {
                    open = false
                    action()
                })
            }
            item(t("Forward"), Icons.AutoMirrored.Outlined.ArrowForward) { web.forward(site) }
            item(t("Reload"), Icons.Outlined.Refresh) { web.reload(site) }
            item(t("Text size ({{percent}}%)", "percent" to web.browser.textScale), Icons.Outlined.FormatSize) { web.browser.cycleTextScale() }
            web.browser.webView(site)?.url?.let { url ->
                item(t("Open in browser"), Icons.AutoMirrored.Outlined.OpenInNew) {
                    context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
                }
            }
        }
    }
}

/** System back walks the page's history first, as in a browser. */
@Composable
private fun HistoryBack(site: SchoolSite, onExhausted: (() -> Unit)?) {
    val web = LocalModel.current.web
    web.browser.loads
    val canGoBack = web.browser.canGoBack(site)
    BackHandler(enabled = canGoBack || onExhausted != null) {
        if (canGoBack) web.back(site) else onExhausted?.invoke()
    }
}

/** ウェブ tab: one resident view per site, site tabs on top. */
@Composable
fun WebTabScreen() {
    val web = LocalModel.current.web
    DisposableEffect(Unit) {
        web.showTab(web.tabSite)
        web.setVisible(web.tabSite)
        onDispose { web.setVisible(null) }
    }
    HistoryBack(web.tabSite, onExhausted = null)
    Column(Modifier.fillMaxSize().background(Theme.colors.bg).windowInsetsPadding(WindowInsets.statusBars)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            val selected = WebRouter.tabSites.indexOf(web.tabSite).coerceAtLeast(0)
            ScrollableTabRow(
                selectedTabIndex = selected,
                edgePadding = 8.dp,
                containerColor = Theme.colors.bg,
                divider = {},
                modifier = Modifier.weight(1f),
            ) {
                WebRouter.tabSites.forEachIndexed { index, site ->
                    Tab(
                        selected = index == selected,
                        onClick = {
                            web.showTab(site)
                            web.setVisible(site)
                        },
                        text = { Text(WebRouter.name(site), fontWeight = FontWeight.SemiBold) },
                    )
                }
            }
            PageMenu(web.tabSite)
        }
        Box(Modifier.fillMaxSize()) {
            ResidentWebView(web.tabSite, web.browser, web.attachRevision, Modifier.fillMaxSize())
            if (web.loading == web.tabSite && web.browser.webView(web.tabSite) == null) {
                CircularProgressIndicator(Modifier.align(Alignment.Center))
            }
            web.error?.let { ProblemNote(it, Modifier.align(Alignment.TopCenter)) }
        }
    }
}

/** A school page pushed from a course or assignment, like a browser tab:
 * ✕ returns to the opener, back walks the page history first. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WebPageScreen(page: WebRouter.PushedPage) {
    val web = LocalModel.current.web
    val navigator = LocalNavigator.current
    val context = LocalContext.current
    DisposableEffect(page) {
        web.open(page.site, page.target)
        web.setVisible(page.site)
        onDispose { web.setVisible(null) }
    }
    HistoryBack(page.site) { navigator.back() }
    Column(Modifier.fillMaxSize().background(Theme.colors.surface)) {
        TopAppBar(
            navigationIcon = {
                IconButton(onClick = { navigator.back() }) { Icon(Icons.Outlined.Close, t("Close")) }
            },
            title = {
                Column {
                    Text(page.title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(3.dp)) {
                        Icon(Icons.Outlined.Lock, null, tint = Theme.colors.muted, modifier = Modifier.size(12.dp))
                        Text(WebRouter.name(page.site), style = MaterialTheme.typography.labelMedium, color = Theme.colors.muted)
                    }
                }
            },
            actions = {
                web.browser.webView(page.site)?.url?.let { url ->
                    IconButton(onClick = {
                        val share = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, url)
                        context.startActivity(Intent.createChooser(share, null))
                    }) { Icon(Icons.Outlined.Share, t("Share")) }
                }
                PageMenu(page.site)
            },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = Theme.colors.surface),
        )
        Box(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.navigationBars)) {
            ResidentWebView(page.site, web.browser, web.attachRevision, Modifier.fillMaxSize())
            web.error?.let { ProblemNote(it, Modifier.align(Alignment.Center)) }
        }
    }
}

@Composable
private fun ProblemNote(text: String, modifier: Modifier) {
    Text(
        text,
        style = MaterialTheme.typography.bodyMedium,
        modifier = modifier
            .padding(16.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(Theme.colors.dangerSoft)
            .padding(12.dp),
    )
}
