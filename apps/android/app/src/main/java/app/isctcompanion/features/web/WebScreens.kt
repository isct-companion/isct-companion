package app.isctcompanion.features.web

import app.isctcompanion.design.t
import android.content.Intent
import android.content.Context
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.BottomAppBar
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material.icons.rounded.ArrowDropDown
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
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.rounded.OpenInNew
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Print
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Share
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.FormatSize
import androidx.compose.material.icons.rounded.MoreVert
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.TextButton
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
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
import androidx.swiperefreshlayout.widget.SwipeRefreshLayout
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.SchoolBrowserHost
import app.isctcompanion.core.SchoolSite
import app.isctcompanion.core.WebRouter
import app.isctcompanion.design.Theme

/** Hosts a site's frontmost WebView (its newest popup, else its resident
 * view) while on screen and parks it afterwards, so its page, scroll
 * position and session survive tab and route changes. A pull at the top of
 * the page reloads it through Core, as on iOS. */
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
            val webView = host.front(site)
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
        IconButton(onClick = { open = true }) { Icon(Icons.Rounded.MoreVert, t("More")) }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            val item = @Composable { label: String, icon: ImageVector, action: () -> Unit ->
                DropdownMenuItem(text = { Text(label) }, leadingIcon = { Icon(icon, null) }, onClick = {
                    open = false
                    action()
                })
            }
            item(t("Reload"), Icons.Rounded.Refresh) { web.reload(site) }
            web.browser.front(site)?.url?.let { url ->
                item(t("Share"), Icons.Rounded.Share) { share(context, url) }
                item(t("Open in browser"), Icons.AutoMirrored.Rounded.OpenInNew) { openInBrowser(context, url) }
                item(t("Print"), Icons.Rounded.Print) { web.browser.print(site, context) }
            }
            HorizontalDivider()
            for (percent in SchoolBrowserHost.textScales) {
                DropdownMenuItem(
                    text = { Text(t("Text size ({{percent}}%)", "percent" to percent)) },
                    leadingIcon = { Icon(Icons.Rounded.FormatSize, null) },
                    trailingIcon = { if (percent == web.browser.textScale) Icon(Icons.Rounded.Check, null) },
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
        Icon(Icons.AutoMirrored.Rounded.KeyboardArrowLeft, t("Previous page"))
    }
    IconButton(onClick = { web.forward(site) }, enabled = web.browser.canGoForward(site)) {
        Icon(Icons.AutoMirrored.Rounded.KeyboardArrowRight, t("Next page"))
    }
}

/** System back walks the page's history first, as in a browser, then
 * closes a popup in front. */
@Composable
private fun HistoryBack(site: SchoolSite, onExhausted: (() -> Unit)?) {
    val web = LocalModel.current.web
    web.browser.loads
    val canGoBack = web.browser.canGoBack(site)
    val popup = (web.popups[site] ?: 0) > 0
    BackHandler(enabled = canGoBack || popup || onExhausted != null) {
        when {
            canGoBack -> web.back(site)
            popup -> web.closePopup(site)
            else -> onExhausted?.invoke()
        }
    }
}

/** The page area both web screens share: a close bar while a popup is in
 * front, Core's notice about the page, the page itself, and the dialogs it
 * asks for. A page that did not load is covered by the notice, so WebView's
 * own error page never shows. */
@Composable
private fun SchoolPage(site: SchoolSite, modifier: Modifier = Modifier) {
    val web = LocalModel.current.web
    val context = LocalContext.current
    Column(modifier) {
        if ((web.popups[site] ?: 0) > 0) {
            Row(
                Modifier.fillMaxWidth().background(Theme.colors.surface).padding(start = 16.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(t("Popup window"), style = MaterialTheme.typography.labelLarge, modifier = Modifier.weight(1f))
                IconButton(onClick = { web.closePopup(site) }) { Icon(Icons.Rounded.Close, t("Close window")) }
            }
            HorizontalDivider()
        }
        val notice = web.notices[site]
        if (notice is WebRouter.Notice.Reopened) {
            NoticeBanner(t("Reopened this page without sending the form again."), Theme.colors.accentSoft)
        }
        val failure = notice as? WebRouter.Notice.LoadFailed
        if (failure?.reason == "cleartext") {
            NoticeBanner(t(failureText(failure.reason)), Theme.colors.warnSoft) {
                failure.url?.let { url -> TextButton(onClick = { openInBrowser(context, url) }) { Text(t("Open in browser")) } }
            }
        }
        Box(Modifier.weight(1f).fillMaxWidth()) {
            ResidentWebView(site, web.browser, web.attachRevision, Modifier.fillMaxSize())
            if (failure != null && failure.reason != "cleartext") FailedPage(site, failure)
        }
    }
    PageDialog(web.browser)
}

@Composable
private fun NoticeBanner(text: String, color: Color, actions: @Composable () -> Unit = {}) {
    Row(
        Modifier.fillMaxWidth().background(color).padding(start = 16.dp, end = 8.dp, top = 4.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(text, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f).padding(vertical = 8.dp))
        actions()
    }
}

/** Core's notice in place of a page that did not load: reopen it (a form
 * result is opened again with GET), go back, or try the browser. */
@Composable
private fun FailedPage(site: SchoolSite, failure: WebRouter.Notice.LoadFailed) {
    val web = LocalModel.current.web
    val context = LocalContext.current
    web.browser.loads
    Column(
        Modifier.fillMaxSize().background(Theme.colors.bg).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(t(failureText(failure.reason)), style = MaterialTheme.typography.bodyLarge)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            TextButton(onClick = { web.retry(site) }) { Text(t("Reopen this page")) }
            if (web.browser.canGoBack(site)) TextButton(onClick = { web.back(site) }) { Text(t("Back")) }
            failure.url?.let { url -> TextButton(onClick = { openInBrowser(context, url) }) { Text(t("Open in browser")) } }
        }
    }
}

/** What the notice says when a page did not load, as the desktop's. */
private fun failureText(reason: String): String = when (reason) {
    "offline" -> "This page could not be opened: there is no network connection."
    "host-not-found" ->
        "This page could not be opened: its server was not found. Campus-only services need the campus network or VPN."
    "unreachable" ->
        "This page could not be opened: its server did not answer. Campus-only services need the campus network or VPN."
    "insecure" -> "This page could not be opened: the secure connection failed."
    "cleartext" -> "This page uses an unencrypted connection, so it opens only in your browser."
    else -> "This page could not be opened."
}

/** A page's alert, confirm, prompt or leave question, titled with the asking
 * frame's origin. The host asks only for the page on screen; leaving the
 * screen answers Cancel. */
@Composable
private fun PageDialog(host: SchoolBrowserHost) {
    val dialog = host.dialog ?: return
    key(dialog) {
        var text by remember { mutableStateOf(dialog.value.orEmpty()) }
        DisposableEffect(Unit) { onDispose { dialog.answer(false, null) } }
        val leave = dialog.kind == SchoolBrowserHost.PageDialog.Kind.LEAVE
        AlertDialog(
            onDismissRequest = { dialog.answer(dialog.kind == SchoolBrowserHost.PageDialog.Kind.ALERT, null) },
            title = { Text(if (leave) t("Leave this page?") else dialog.origin) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    if (dialog.message.isNotEmpty()) Text(dialog.message)
                    if (dialog.kind == SchoolBrowserHost.PageDialog.Kind.PROMPT) {
                        OutlinedTextField(text, { text = it }, singleLine = true, modifier = Modifier.fillMaxWidth())
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { dialog.answer(true, text) }) { Text(if (leave) t("Leave") else t("OK")) }
            },
            dismissButton = if (dialog.kind == SchoolBrowserHost.PageDialog.Kind.ALERT) null else {
                { TextButton(onClick = { dialog.answer(false, null) }) { Text(if (leave) t("Stay") else t("Cancel")) } }
            },
        )
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
            SchoolPage(web.tabSite, Modifier.fillMaxSize())
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
            Icon(Icons.Rounded.ArrowDropDown, t("Switch sites"))
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            for (site in WebRouter.tabSites) {
                DropdownMenuItem(
                    text = { Text(WebRouter.name(site)) },
                    trailingIcon = { if (site == web.tabSite) Icon(Icons.Rounded.Check, null) },
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
                IconButton(onClick = { navigator.back() }) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, t("Back")) }
            },
            title = { Text(page.title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            actions = { PageMenu(page.site) },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = Theme.colors.surface),
        )
        Box(Modifier.weight(1f).fillMaxWidth()) {
            SchoolPage(page.site, Modifier.fillMaxSize())
            web.error?.let { ProblemNote(it, Modifier.align(Alignment.Center)) }
        }
        BottomAppBar(containerColor = Theme.colors.surface) {
            web.browser.loads
            val url = web.browser.front(page.site)?.url
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceEvenly) {
                HistoryButtons(page.site)
                IconButton(onClick = { url?.let { share(context, it) } }, enabled = url != null) {
                    Icon(Icons.Rounded.Share, t("Share"))
                }
                IconButton(onClick = { url?.let { openInBrowser(context, it) } }, enabled = url != null) {
                    Icon(Icons.AutoMirrored.Rounded.OpenInNew, t("Open in browser"))
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
