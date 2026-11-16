package app.isctcompanion.features.pastexams

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Public
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.core.AppModel
import app.isctcompanion.design.t
import kotlinx.coroutines.launch

/**
 * Where a past-exams "open on the website" link opens: a Custom Tab over
 * the app, or the system browser. The choice is this device's
 * `examLinkTarget` setting: asked on the first tap, then followed; a long
 * press on a link picks again.
 */
class ExamLinks internal constructor(private val model: AppModel, private val context: Context) {
    internal var asking by mutableStateOf<String?>(null)

    /** Opens [url] as the person chose; [repick] asks again first. */
    fun open(url: String, repick: Boolean = false) {
        val saved = model.records.settings.optString("examLinkTarget")
        if (repick || saved.isEmpty()) asking = url else launch(url, saved)
    }

    internal fun launch(url: String, target: String) {
        val uri = Uri.parse(url)
        try {
            if (target == "app") {
                CustomTabsIntent.Builder().build().launchUrl(context, uri)
            } else {
                context.startActivity(Intent(Intent.ACTION_VIEW, uri))
            }
        } catch (_: ActivityNotFoundException) {
            // No browser handles it; nothing to open.
        }
    }
}

/** The page's link opener, with the dialog that asks how to open. */
@Composable
fun rememberExamLinks(): ExamLinks {
    val model = LocalModel.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val links = remember(model, context) { ExamLinks(model, context) }
    links.asking?.let { url ->
        fun choose(target: String) {
            links.asking = null
            scope.launch { model.updateSettings { it.put("examLinkTarget", target) } }
            links.launch(url, target)
        }
        AlertDialog(
            onDismissRequest = { links.asking = null },
            title = { Text(t("examShare.openOnWeb")) },
            text = { Text(t("examShare.linkChoiceHint")) },
            confirmButton = { TextButton(onClick = { choose("app") }) { Text(t("examShare.openInApp")) } },
            dismissButton = { TextButton(onClick = { choose("browser") }) { Text(t("examShare.openInBrowser")) } },
        )
    }
    return links
}

/** The header's website button: a tap opens as chosen, a long press picks again. */
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
