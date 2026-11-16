package app.isctcompanion.features.pastexams

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Public
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import app.isctcompanion.design.t

/**
 * Past exams' links to the ScienceTokyoExam website, through the system:
 * [open] goes to the default browser (or the system's "Open with" when none
 * is set); [share] shows the system's share sheet.
 */
class ExamLinks internal constructor(private val context: Context) {
    fun open(url: String) = start(view(url))

    fun share(url: String) = start(
        Intent.createChooser(Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, url), null),
    )

    private fun view(url: String) =
        Intent(Intent.ACTION_VIEW, Uri.parse(url)).addCategory(Intent.CATEGORY_BROWSABLE)

    private fun start(intent: Intent) {
        try {
            context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        } catch (_: ActivityNotFoundException) {
            // No browser on the device; nothing opens.
        }
    }
}

@Composable
fun rememberExamLinks(): ExamLinks {
    val context = LocalContext.current
    return remember(context) { ExamLinks(context) }
}

/** The header's website button: a tap opens the link, a long press shares it. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ExamLinkButton(links: ExamLinks, url: () -> String) {
    Box(
        Modifier
            .size(48.dp)
            .clip(CircleShape)
            .combinedClickable(
                onClickLabel = t("examShare.openOnWeb"),
                onLongClickLabel = t("common.share"),
                onLongClick = { links.share(url()) },
                onClick = { links.open(url()) },
            ),
        contentAlignment = Alignment.Center,
    ) {
        Icon(Icons.Rounded.Public, t("examShare.openOnWeb"))
    }
}
