package app.isctcompanion.features.pastexams

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.drawable.Drawable
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Public
import androidx.compose.material.icons.rounded.Web
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.graphics.drawable.toBitmap
import app.isctcompanion.LocalModel
import app.isctcompanion.core.AppModel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import kotlinx.coroutines.launch

/** A browser on this device that opens web links. */
data class ExamBrowser(val packageName: String, val label: String, val icon: Drawable)

/**
 * Where a past-exams "open on the website" link opens: a Custom Tab over
 * the app, or one of the browsers on this device. The choice is this
 * device's `examLinkTarget` and `examLinkBrowser` settings: asked on the
 * first tap, then followed; a long press on a link asks again.
 */
class ExamLinks internal constructor(private val model: AppModel, private val context: Context) {
    internal var asking by mutableStateOf<String?>(null)

    /** The browsers that open https links, as the system lists them. */
    internal fun browsers(): List<ExamBrowser> {
        val pm = context.packageManager
        val view = Intent(Intent.ACTION_VIEW, Uri.parse("https://sciencetokyoexam.com"))
            .addCategory(Intent.CATEGORY_BROWSABLE)
        return pm.queryIntentActivities(view, PackageManager.MATCH_ALL)
            .map { it.activityInfo.applicationInfo }
            .distinctBy { it.packageName }
            .filter { it.packageName != context.packageName }
            .map { ExamBrowser(it.packageName, pm.getApplicationLabel(it).toString(), pm.getApplicationIcon(it)) }
            .sortedBy { it.label.lowercase() }
    }

    /** The saved choice: "" for the app's own tab, a package, or null when
     * none is saved or the browser is gone. */
    internal fun current(): String? {
        val settings = model.records.settings
        return when (settings.optString("examLinkTarget")) {
            "app" -> ""
            "browser" -> settings.optString("examLinkBrowser")
                .takeIf { saved -> saved.isNotEmpty() && browsers().any { it.packageName == saved } }
            else -> null
        }
    }

    /** Opens [url] as the person chose; [repick] asks again first. */
    fun open(url: String, repick: Boolean = false) {
        val saved = if (repick) null else current()
        if (saved == null) asking = url else launch(url, saved)
    }

    /** "" opens the app's own tab; otherwise that browser's package. */
    internal fun launch(url: String, browser: String) {
        val uri = Uri.parse(url)
        try {
            if (browser.isEmpty()) {
                CustomTabsIntent.Builder().build().launchUrl(context, uri)
            } else {
                context.startActivity(
                    Intent(Intent.ACTION_VIEW, uri).setPackage(browser).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
            }
        } catch (_: ActivityNotFoundException) {
            // The browser went away between choosing and opening.
        }
    }
}

/** The page's link opener, with the sheet that asks how to open. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun rememberExamLinks(): ExamLinks {
    val model = LocalModel.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val links = remember(model, context) { ExamLinks(model, context) }
    links.asking?.let { url ->
        val browsers = remember(url) { links.browsers() }
        val current = remember(url) { links.current() }
        fun choose(browser: String) {
            links.asking = null
            scope.launch {
                model.updateSettings {
                    it.put("examLinkTarget", if (browser.isEmpty()) "app" else "browser")
                    if (browser.isEmpty()) it.remove("examLinkBrowser") else it.put("examLinkBrowser", browser)
                }
            }
            links.launch(url, browser)
        }
        ModalBottomSheet(onDismissRequest = { links.asking = null }, containerColor = Theme.colors.bg) {
            Column(Modifier.navigationBarsPadding().padding(bottom = 12.dp)) {
                Text(
                    t("examShare.linkChoice"),
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
                )
                ChoiceRow(t("examShare.inAppBrowser"), selected = current == "", icon = {
                    Icon(Icons.Rounded.Web, null, tint = Theme.colors.accent)
                }) { choose("") }
                for (browser in browsers) {
                    ChoiceRow(browser.label, selected = current == browser.packageName, icon = {
                        Image(browser.icon.toBitmap(96, 96).asImageBitmap(), null, Modifier.size(24.dp))
                    }) { choose(browser.packageName) }
                }
                Text(
                    t("examShare.linkChoiceHint"),
                    color = Theme.colors.muted,
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
                )
            }
        }
    }
    return links
}

@Composable
private fun ChoiceRow(label: String, selected: Boolean, icon: @Composable () -> Unit, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 24.dp, vertical = 14.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        icon()
        Text(label, Modifier.weight(1f))
        if (selected) Icon(Icons.Rounded.Check, null, tint = Theme.colors.accent)
    }
}

/** The header's website button: a tap opens as chosen, a long press asks again. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ExamLinkButton(links: ExamLinks, url: () -> String) {
    Box(
        Modifier
            .size(48.dp)
            .clip(CircleShape)
            .combinedClickable(
                onClickLabel = t("examShare.openOnWeb"),
                onLongClickLabel = t("examShare.linkChoice"),
                onLongClick = { links.open(url(), repick = true) },
                onClick = { links.open(url()) },
            ),
        contentAlignment = Alignment.Center,
    ) {
        Icon(Icons.Rounded.Public, t("examShare.openOnWeb"))
    }
}
