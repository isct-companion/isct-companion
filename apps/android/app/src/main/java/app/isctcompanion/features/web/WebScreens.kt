package app.isctcompanion.features.web

import app.isctcompanion.design.t
import android.content.Intent
import android.content.Context
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.BottomAppBar
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material.icons.outlined.ArrowDropDown
import app.isctcompanion.design.GuideTip
import android.net.Uri
import android.view.ViewGroup
import android.webkit.WebView
import android.widget.FrameLayout
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.outlined.OpenInNew
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.Share
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.FormatSize
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
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
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.swiperefreshlayout.widget.SwipeRefreshLayout
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.SchoolBrowserHost
import app.isctcompanion.core.SchoolSite
import app.isctcompanion.core.WebRouter
import app.isctcompanion.design.Theme

/** Hosts a site's resident WebView while on screen and parks it afterwards,
 * so its page, scroll position and session survive tab and route changes.
 * A pull at the top of the page reloads it through Core, as on iOS. */
@Composable
private fun ResidentWebView(site: SchoolSite, host: SchoolBrowserHost, revision: Int, modifier: Modifier = Modifier) {
    val web = LocalModel.current.web
    // Read so a newly created view is attached.
    val created = host.revision
    AndroidView(
        factory = { context ->
            val container = FrameLayout(context)
            SwipeRefreshLayout(context).apply {
                addView(container)
                // The layout's own spinner is a child too, so keep the container.
                tag = container
                // The container never scrolls; the page inside it does.
                setOnChildScrollUpCallback { _, _ -> (container.getChildAt(0) as? WebView)?.canScrollVertically(-1) == true }
            }
        },
        update = { pull ->
            val container = pull.tag as FrameLayout
            pull.setOnRefreshListener {
                web.reload(site)
                // The page shows its own progress; the spinner only answers the pull.
                pull.postDelayed({ pull.isRefreshing = false }, 600)
            }
            revision + created
            val webView = host.webView(site)
            val shown = (0 until container.childCount).map { container.getChildAt(it) }
            for (child in shown) if (child !== webView && child is WebView) host.park(child)
            if (webView != null && webView.parent !== container) {
                (webView.parent as? ViewGroup)?.removeView(webView)
                container.addView(webView, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
            }
        },
        onRelease = { pull ->
            val container = pull.tag as FrameLayout
            (0 until container.childCount).map { container.getChildAt(it) }.filterIsInstance<WebView>().forEach(host::park)
        },
        modifier = modifier,
    )
}

/** The page menu both web screens share, as on iOS: reload, share, open in
 * the browser and the text size. Back and forward sit in the bars. */
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
            item(t("Reload"), Icons.Outlined.Refresh) { web.reload(site) }
            web.browser.webView(site)?.url?.let { url ->
                item(t("Share"), Icons.Outlined.Share) { share(context, url) }
                item(t("Open in browser"), Icons.AutoMirrored.Outlined.OpenInNew) { openInBrowser(context, url) }
            }
            HorizontalDivider()
            for (percent in SchoolBrowserHost.textScales) {
                DropdownMenuItem(
                    text = { Text(t("Text size ({{percent}}%)", "percent" to percent)) },
                    leadingIcon = { Icon(Icons.Outlined.FormatSize, null) },
                    trailingIcon = { if (percent == web.browser.textScale) Icon(Icons.Outlined.Check, null) },
                    onClick = {
                        open = false
                        web.browser.chooseTextScale(percent)
                    },
                )
            }
        }
    }
}

private fun share(context: Context, url: String) {
    val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, url)
    context.startActivity(Intent.createChooser(send, null))
}

private fun openInBrowser(context: Context, url: String) {
    context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
}

/** Back and forward through the page's history, enabled as it allows. */
@Composable
private fun HistoryButtons(site: SchoolSite) {
    val web = LocalModel.current.web
    web.browser.loads
    IconButton(onClick = { web.back(site) }, enabled = web.browser.canGoBack(site)) {
        Icon(Icons.AutoMirrored.Outlined.KeyboardArrowLeft, t("Previous page"))
    }
    IconButton(onClick = { web.forward(site) }, enabled = web.browser.canGoForward(site)) {
        Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, t("Next page"))
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

/** ウェブ tab, as on iOS: the page fills the screen under one bar. The title
 * is the shown site and opens the site list (picking the shown site again
 * returns it to its start page); back and forward lead, the page menu
 * trails. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WebTabScreen() {
    val web = LocalModel.current.web
    DisposableEffect(Unit) {
        web.showTab(web.tabSite)
        web.setVisible(web.tabSite)
        onDispose { web.setVisible(null) }
    }
    HistoryBack(web.tabSite, onExhausted = null)
    Column(Modifier.fillMaxSize().background(Theme.colors.bg)) {
        CenterAlignedTopAppBar(
            navigationIcon = { Row { HistoryButtons(web.tabSite) } },
            title = { SiteTitle() },
            actions = { PageMenu(web.tabSite) },
            colors = TopAppBarDefaults.centerAlignedTopAppBarColors(containerColor = Theme.colors.bg),
        )
        Box(Modifier.fillMaxSize()) {
            ResidentWebView(web.tabSite, web.browser, web.attachRevision, Modifier.fillMaxSize())
            if (web.loading == web.tabSite && web.browser.webView(web.tabSite) == null) {
                CircularProgressIndicator(Modifier.align(Alignment.Center))
            }
            web.error?.let { ProblemNote(it, Modifier.align(Alignment.TopCenter)) }
            // The one thing drawn over the page, once: how to reach the other sites.
            GuideTip(
                "web-sites", t("Switch sites"), t("Tap the title to switch school sites."),
                Modifier.align(Alignment.BottomCenter).padding(12.dp),
            )
        }
    }
}

/** The shown site's name with a drop-down mark; the list checks the shown site. */
@Composable
private fun SiteTitle() {
    val web = LocalModel.current.web
    var open by remember { mutableStateOf(false) }
    Box {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.clip(RoundedCornerShape(Theme.radiusChip)).clickable { open = true }
                .padding(horizontal = 8.dp, vertical = 4.dp),
        ) {
            Text(WebRouter.name(web.tabSite), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            Icon(Icons.Outlined.ArrowDropDown, t("Switch sites"))
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            for (site in WebRouter.tabSites) {
                DropdownMenuItem(
                    text = { Text(WebRouter.name(site)) },
                    trailingIcon = { if (site == web.tabSite) Icon(Icons.Outlined.Check, null) },
                    onClick = {
                        open = false
                        web.choose(site)
                    },
                )
            }
        }
    }
}

/** A school page pushed from a course or assignment, as on iOS: back to the
 * opener and the page title on top; back, forward, share and the browser in
 * the bottom bar. System back walks the page history first. */
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
                IconButton(onClick = { navigator.back() }) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, t("Back")) }
            },
            title = { Text(page.title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            actions = { PageMenu(page.site) },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = Theme.colors.surface),
        )
        Box(Modifier.weight(1f).fillMaxWidth()) {
            ResidentWebView(page.site, web.browser, web.attachRevision, Modifier.fillMaxSize())
            web.error?.let { ProblemNote(it, Modifier.align(Alignment.Center)) }
        }
        BottomAppBar(containerColor = Theme.colors.surface) {
            web.browser.loads
            val url = web.browser.webView(page.site)?.url
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceEvenly) {
                HistoryButtons(page.site)
                IconButton(onClick = { url?.let { share(context, it) } }, enabled = url != null) {
                    Icon(Icons.Outlined.Share, t("Share"))
                }
                IconButton(onClick = { url?.let { openInBrowser(context, it) } }, enabled = url != null) {
                    Icon(Icons.AutoMirrored.Outlined.OpenInNew, t("Open in browser"))
                }
            }
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
