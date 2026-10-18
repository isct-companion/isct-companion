package app.isctcompanion.features.web

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

/** ウェブ tab: one resident view per site, a site switcher on top. */
@Composable
fun WebTabScreen() {
    val web = LocalModel.current.web
    val context = LocalContext.current
    DisposableEffect(Unit) {
        web.showTab(web.tabSite)
        web.setVisible(web.tabSite)
        onDispose { web.setVisible(null) }
    }
    Column(Modifier.fillMaxSize().background(Theme.colors.bg).windowInsetsPadding(WindowInsets.statusBars)) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
        ) {
            Circle(Icons.AutoMirrored.Outlined.KeyboardArrowLeft, "戻る") { web.back(web.tabSite) }
            Row(
                Modifier
                    .weight(1f)
                    .clip(CircleShape)
                    .background(Theme.colors.surface)
                    .horizontalScroll(rememberScrollState())
                    .padding(4.dp),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                for (site in WebRouter.tabSites) {
                    val selected = site == web.tabSite
                    Text(
                        WebRouter.name(site),
                        style = MaterialTheme.typography.bodyMedium,
                        fontWeight = FontWeight.SemiBold,
                        color = if (selected) Color.White else Theme.colors.ink,
                        modifier = Modifier
                            .clip(CircleShape)
                            .background(if (selected) Theme.colors.accent else Color.Transparent)
                            .clickable {
                                web.showTab(site)
                                web.setVisible(site)
                            }
                            .padding(horizontal = 12.dp, vertical = 8.dp),
                    )
                }
            }
            var menu by remember { mutableStateOf(false) }
            Box {
                Circle(Icons.Outlined.MoreHoriz, "その他") { menu = true }
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                    DropdownMenuItem(text = { Text("再読み込み") }, leadingIcon = { Icon(Icons.Outlined.Refresh, null) }, onClick = {
                        menu = false
                        web.reload(web.tabSite)
                    })
                    DropdownMenuItem(
                        text = { Text("進む") }, leadingIcon = { Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, null) },
                        onClick = {
                            menu = false
                            web.forward(web.tabSite)
                        },
                    )
                    web.browser.webView(web.tabSite)?.url?.let { url ->
                        DropdownMenuItem(
                            text = { Text("ブラウザで開く") }, leadingIcon = { Icon(Icons.AutoMirrored.Outlined.OpenInNew, null) },
                            onClick = {
                                menu = false
                                context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
                            },
                        )
                    }
                }
            }
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

/** A school page pushed from a course or assignment; back returns to it. */
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
    BackHandler { navigator.back() }
    Box(Modifier.fillMaxSize().background(Theme.colors.surface)) {
        ResidentWebView(
            page.site, web.browser, web.attachRevision,
            Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.statusBars).padding(top = 56.dp, bottom = 72.dp),
        )
        web.error?.let { ProblemNote(it, Modifier.align(Alignment.Center)) }
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            modifier = Modifier
                .align(Alignment.TopCenter)
                .windowInsetsPadding(WindowInsets.statusBars)
                .padding(horizontal = 12.dp, vertical = 4.dp),
        ) {
            Circle(Icons.AutoMirrored.Outlined.ArrowBack, "戻る") { navigator.back() }
            Glass(Modifier.weight(1f)) {
                Column(Modifier.fillMaxWidth().padding(vertical = 6.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(page.title, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold, maxLines = 1)
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(3.dp)) {
                        Icon(Icons.Outlined.Lock, null, tint = Theme.colors.muted, modifier = Modifier.size(10.dp))
                        Text(WebRouter.name(page.site), style = MaterialTheme.typography.labelSmall, color = Theme.colors.muted)
                    }
                }
            }
            Glass(Modifier.size(40.dp), CircleShape) {
                Box(Modifier.fillMaxSize().clickable { web.browser.cycleTextScale() }, contentAlignment = Alignment.Center) {
                    Text("ぁあ", style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold)
                }
            }
        }
        Glass(
            Modifier
                .align(Alignment.BottomCenter)
                .windowInsetsPadding(WindowInsets.navigationBars)
                .padding(horizontal = 16.dp, vertical = 4.dp)
                .fillMaxWidth(),
        ) {
            Row(
                horizontalArrangement = Arrangement.SpaceEvenly,
                modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
            ) {
                IconButton(onClick = { web.back(page.site) }) {
                    Icon(Icons.AutoMirrored.Outlined.KeyboardArrowLeft, "前のページ", tint = Theme.colors.accent)
                }
                IconButton(onClick = { web.forward(page.site) }) {
                    Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, "次のページ", tint = Theme.colors.accent)
                }
                val url = web.browser.webView(page.site)?.url
                if (url != null) {
                    IconButton(onClick = {
                        val share = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, url)
                        context.startActivity(Intent.createChooser(share, null))
                    }) { Icon(Icons.Outlined.Share, "共有", tint = Theme.colors.accent) }
                    IconButton(onClick = { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }) {
                        Icon(Icons.AutoMirrored.Outlined.OpenInNew, "ブラウザで開く", tint = Theme.colors.accent)
                    }
                }
            }
        }
    }
}

/** Floating control surface over the page (the iOS material capsules). */
@Composable
private fun Glass(modifier: Modifier = Modifier, shape: androidx.compose.ui.graphics.Shape = CircleShape, content: @Composable () -> Unit) {
    Surface(modifier, shape = shape, color = Theme.colors.surface.copy(alpha = 0.92f), shadowElevation = 2.dp, content = content)
}

@Composable
private fun Circle(icon: ImageVector, label: String, onClick: () -> Unit) {
    Glass(Modifier.size(40.dp)) {
        IconButton(onClick = onClick) { Icon(icon, label, tint = Theme.colors.accent) }
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
