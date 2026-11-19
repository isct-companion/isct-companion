package app.isctcompanion.design

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.TextButton
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.text.format.Formatter
import android.webkit.MimeTypeMap
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
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
import androidx.core.content.FileProvider
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
fun HtmlText(html: String, courseId: String, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val opening by rememberUpdatedState(rememberMaterialOpening())
    val accent = Theme.colors.accent
    val text = remember(html, accent) {
        AnnotatedString.fromHtml(
            html.trim(),
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
    SelectionContainer(modifier) {
        Text(text, style = MaterialTheme.typography.bodyLarge, color = Theme.colors.ink)
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
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        modifier = Modifier
            .fillMaxWidth()
            .clickable(enabled = !opening.busy && source != null) { source?.let { opening.open(courseId, it, title) } }
            .padding(horizontal = 16.dp, vertical = 12.dp),
    ) {
        Icon(materialIcon(kind), null, tint = Theme.colors.muted, modifier = Modifier.size(18.dp))
        Text(title, color = Theme.colors.ink, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
        if (opening.busy) CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
    }
}

fun materialIcon(kind: LessonItemKind): ImageVector = when (kind) {
    LessonItemKind.FILE -> Icons.Rounded.AttachFile
    LessonItemKind.FOLDER -> Icons.Rounded.Folder
    LessonItemKind.LINK, LessonItemKind.OTHER -> Icons.Rounded.Link
    LessonItemKind.FORUM -> Icons.Rounded.Forum
    LessonItemKind.PAGE, LessonItemKind.LABEL -> Icons.Rounded.Description
}

/** Opens a course item or link; `busy` while Core answers. */
class MaterialOpening(val busy: Boolean, val open: (courseId: String, source: String, title: String) -> Unit)

/** Opens an item or link as Core decides (`open_material`): a file Core
 * fetches with the signed-in LMS session (or from the course folder) opens in
 * the app's PDF reader when it is a PDF and in the system viewer otherwise,
 * a school page opens in its in-app view, any other page in the browser. When the file is not shown, the dialog offers the LMS page
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
            fallback = null
            fun offer(page: SchoolWebTarget?) {
                fallback = page?.let { WebRouter.PushedPage(it.site, it.url, title) }
            }
            try {
                when (val material = model.openMaterial(courseId, source)) {
                    is Material.Ready -> {
                        val file = File(material.file.path)
                        if (file.extension.equals("pdf", ignoreCase = true)) navigator.openPdf(file.path, title)
                        else if (!viewInAnotherApp(context, file)) show(material.page, title)
                    }
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
    retry?.let { (courseId, source, title) ->
        AlertDialog(
            onDismissRequest = { retry = null },
            text = { Text(t("lmsFile.theFileCouldNotBe")) },
            confirmButton = {
                TextButton(onClick = {
                    retry = null
                    open(courseId, source, title)
                }) { Text(t("common.tryAgain")) }
            },
            dismissButton = {
                fallback?.let { page ->
                    TextButton(onClick = {
                        retry = null
                        navigator.openPage(page)
                    }) { Text(t("common.openInLMS")) }
                }
            },
        )
    }
    notice?.let { text ->
        AlertDialog(
            onDismissRequest = { notice = null },
            text = { Text(text) },
            confirmButton = {
                fallback?.let { page ->
                    TextButton(onClick = {
                        notice = null
                        navigator.openPage(page)
                    }) { Text(t("common.openInLMS")) }
                }
            },
            dismissButton = {
                TextButton(onClick = { notice = null }) { Text(t("common.cancel")) }
            },
        )
    }
    return MaterialOpening(busy, open)
}

/** Hands a downloaded file to the app that views its type. */
fun viewInAnotherApp(context: Context, file: File): Boolean {
    val uri = runCatching { FileProvider.getUriForFile(context, "${context.packageName}.files", file) }.getOrNull() ?: return false
    val type = MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase()) ?: "*/*"
    return try {
        context.startActivity(Intent(Intent.ACTION_VIEW).setDataAndType(uri, type).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
        true
    } catch (error: ActivityNotFoundException) {
        false
    }
}
