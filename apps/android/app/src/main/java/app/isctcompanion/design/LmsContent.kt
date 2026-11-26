package app.isctcompanion.design

import app.isctcompanion.core.openInAnotherApp
import app.isctcompanion.core.showsInApp
import androidx.compose.material3.TextButton
import android.content.Intent
import android.text.format.Formatter
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AttachFile
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.Forum
import androidx.compose.material.icons.rounded.Link
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.fromHtml
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.net.toUri
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.LessonItemKind
import app.isctcompanion.core.Material
import app.isctcompanion.core.MaterialFailure
import app.isctcompanion.core.SchoolWebTarget
import app.isctcompanion.core.WebRouter
import java.io.File
import kotlinx.coroutines.launch

/** LMS text (Core's sanitized `bodyHtml`) as native text: paragraphs, lists,
 * emphasis and links keep their shape; colours follow the app. A web link
 * opens as Core decides for the course `courseId` (`rememberMaterialOpening`);
 * other links (mail, phone) go to the system. */
@Composable
fun HtmlText(html: String, courseId: String, modifier: Modifier = Modifier, originalHtml: String? = null) {
    var original by remember(originalHtml) { mutableStateOf(false) }
    val shown = if (original) originalHtml ?: html else html
    val context = LocalContext.current
    val opening by rememberUpdatedState(rememberMaterialOpening())
    val accent = Theme.colors.accent
    val text = remember(shown, accent) {
        AnnotatedString.fromHtml(
            shown.trim(),
            linkStyles = TextLinkStyles(SpanStyle(color = accent)),
            linkInteractionListener = { link ->
                val url = (link as? LinkAnnotation.Url)?.url ?: return@fromHtml
                if (url.toUri().scheme?.lowercase() in setOf("http", "https")) {
                    opening.open(courseId, url, "")
                } else {
                    runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, url.toUri())) }
                }
            },
        ).trimmed()
    }
    Column(modifier) {
        if (originalHtml != null) {
            TextButton(onClick = { original = !original }) {
                Text(t(if (original) "lms.usePreferredLanguage" else "lms.showOriginal"))
            }
        }
        SelectionContainer {
            Text(text, style = MaterialTheme.typography.bodyLarge, color = Theme.colors.ink)
        }
        opening.Banner()
    }
}

/** HTML ends with a paragraph break the text does not need, and empty leading
 * blocks (`<p><br></p>`) would pad the card's top. */
private fun AnnotatedString.trimmed(): AnnotatedString {
    val start = text.indexOfFirst { !it.isWhitespace() }.takeIf { it >= 0 } ?: return AnnotatedString("")
    val end = text.indexOfLast { !it.isWhitespace() } + 1
    return subSequence(start, end)
}

/** A course file or page row with its icon; see `rememberMaterialOpening`. */
@Composable
fun MaterialButton(title: String, kind: LessonItemKind, source: String?, courseId: String) {
    val opening = rememberMaterialOpening()
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Theme.rowGap),
        modifier = Modifier
            .fillMaxWidth()
            .clickable(enabled = !opening.busy && source != null) { source?.let { opening.open(courseId, it, title) } }
            .padding(horizontal = Theme.rowX, vertical = Theme.rowY),
    ) {
        Icon(materialIcon(kind), null, tint = Theme.colors.muted, modifier = Modifier.size(18.dp))
        Text(title, color = Theme.colors.ink, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
        if (opening.busy) CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
    }
    opening.Banner()
}

fun materialIcon(kind: LessonItemKind): ImageVector = when (kind) {
    LessonItemKind.FILE -> Icons.Rounded.AttachFile
    LessonItemKind.FOLDER -> Icons.Rounded.Folder
    LessonItemKind.LINK, LessonItemKind.OTHER -> Icons.Rounded.Link
    LessonItemKind.FORUM -> Icons.Rounded.Forum
    LessonItemKind.PAGE, LessonItemKind.LABEL -> Icons.Rounded.Description
}

/** Opens a course item or link; `busy` while Core answers. */
class MaterialOpening(
    val busy: Boolean,
    val open: (courseId: String, source: String, title: String) -> Unit,
    val Banner: @Composable () -> Unit,
)

/** Opens an item or link as Core decides (`open_material`): a file Core
 * fetches with the signed-in LMS session (or from the course folder) shows
 * as its viewer says, a ZIP as a folder of its files, a school page opens in its in-app view, any other page
 * in the browser. When the file is not shown, the banner offers the LMS page
 * Core names for it. A download that only failed this time asks instead of
 * showing the LMS page: silently doing so made Share hand out a link where it
 * had handed out the file. */
@Composable
fun rememberMaterialOpening(): MaterialOpening {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }
    var retry by remember { mutableStateOf<Triple<String, String, String>?>(null) }
    var notice by remember { mutableStateOf<String?>(null) }
    /** The LMS page Core named for the item last opened. */
    var fallback by remember { mutableStateOf<WebRouter.PushedPage?>(null) }
    fun show(page: SchoolWebTarget, title: String) = navigator.openPage(WebRouter.PushedPage(page.site, page.url, title))
    lateinit var open: (String, String, String) -> Unit
    open = { courseId, source, title ->
        scope.launch {
            busy = true
            retry = null
            notice = null
            fallback = null
            fun offer(page: SchoolWebTarget?) {
                fallback = page?.let { WebRouter.PushedPage(it.site, it.url, title) }
            }
            try {
                when (val material = model.openMaterial(courseId, source)) {
                    // How a file shows is Core's (`file_preview`): in the app's
                    // own viewer, or in the app that opens its type.
                    is Material.Ready -> {
                        val file = material.file
                        if (showsInApp(file.viewer)) navigator.openCourseFile(file.path, title, file.viewer)
                        else if (!openInAnotherApp(context, File(file.path))) show(material.page, title)
                    }
                    // A ZIP Core lists opens as a folder of its files.
                    is Material.Archive -> navigator.openCourseArchive(courseId, source, title)
                    is Material.Page -> navigator.openPage(WebRouter.PushedPage(material.site, material.url, title))
                    is Material.External ->
                        runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, material.url.toUri())) }
                    is Material.TooLarge -> {
                        offer(material.page)
                        notice = t("lmsFile.thisFileIsTooLarge", "size" to Formatter.formatShortFileSize(context, material.bytes.toLong()))
                    }
                    is Material.Failed -> when (material.reason) {
                        MaterialFailure.LOGIN -> {
                            offer(material.page)
                            notice = t("lmsFile.signInToTheLMS")
                        }
                        MaterialFailure.NETWORK, MaterialFailure.TIMEOUT, MaterialFailure.CANCELLED -> {
                            offer(material.page)
                            retry = Triple(courseId, source, title)
                        }
                        // What Core could not read (it logged why) still opens on its LMS page.
                        else -> material.page?.let { show(it, title) } ?: run { notice = t("lmsFile.thisCantBeShownHere") }
                    }
                    // A folder's files are listed on its LMS page.
                    is Material.Folder -> show(material.page, title)
                    is Material.Missing -> show(material.page, title)
                    null -> notice = t("lmsFile.thisCantBeShownHere")
                }
            } finally {
                busy = false
            }
        }
    }
    return MaterialOpening(busy, open) {
        val message = notice ?: retry?.let { t("lmsFile.theFileCouldNotBe") }
        if (message != null) {
            NoticeBanner(Theme.colors.dangerSoft) {
                Text(message, color = Theme.colors.ink)
                Row {
                    retry?.let { (courseId, source, title) ->
                        TextButton(onClick = { open(courseId, source, title) }) { Text(t("common.tryAgain")) }
                    }
                    fallback?.let { page ->
                        TextButton(onClick = {
                            notice = null
                            retry = null
                            navigator.openPage(page)
                        }) { Text(t("common.openInLMS")) }
                    }
                    TextButton(onClick = { notice = null; retry = null }) { Text(t("common.close")) }
                }
            }
        }
    }
}
