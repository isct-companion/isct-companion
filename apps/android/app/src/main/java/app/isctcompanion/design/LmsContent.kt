package app.isctcompanion.design

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.TextButton
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.text.format.Formatter
import android.webkit.MimeTypeMap
import android.widget.Toast
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.AttachFile
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.Link
import androidx.compose.material.icons.outlined.PlayCircleOutline
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
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
import app.isctcompanion.core.CourseKey
import app.isctcompanion.core.Material
import app.isctcompanion.core.MaterialFailure
import app.isctcompanion.core.SchoolSite
import app.isctcompanion.core.WebRouter
import java.io.File
import kotlinx.coroutines.launch

/** LMS text (Core's sanitized `bodyHtml`) as native text: paragraphs, lists,
 * emphasis and links keep their shape; colours follow the app. LMS links open
 * in the in-app school page, others in the browser. */
@Composable
fun HtmlText(html: String, modifier: Modifier = Modifier) {
    val navigator = LocalNavigator.current
    val context = LocalContext.current
    val accent = Theme.colors.accent
    val text = remember(html, accent) {
        AnnotatedString.fromHtml(
            html.trim(),
            linkStyles = TextLinkStyles(SpanStyle(color = accent)),
            linkInteractionListener = { link ->
                val url = (link as? LinkAnnotation.Url)?.url ?: return@fromHtml
                if (url.toUri().host == "lms.s.isct.ac.jp") {
                    navigator.openPage(WebRouter.PushedPage(SchoolSite.SCIENCE_LMS, url, ""))
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

/** A course file or page row with its icon; see `rememberMaterialOpener`. */
@Composable
fun MaterialButton(title: String, kind: String, source: String?, files: CourseKey?, courseId: String? = null) {
    val opener = rememberMaterialOpener(title, source, files, courseId)
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        modifier = Modifier
            .fillMaxWidth()
            .clickable(enabled = !opener.opening) { opener.open() }
            .padding(horizontal = 16.dp, vertical = 12.dp),
    ) {
        Icon(materialIcon(kind), null, tint = Theme.colors.muted, modifier = Modifier.size(18.dp))
        Text(title, color = Theme.colors.ink, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
        if (opener.opening) CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
    }
}

fun materialIcon(kind: String): ImageVector = when (kind) {
    "pdf" -> Icons.Outlined.Description
    "video" -> Icons.Outlined.PlayCircleOutline
    "file" -> Icons.Outlined.AttachFile
    "folder" -> Icons.Outlined.Folder
    "link" -> Icons.Outlined.Link
    else -> Icons.Outlined.Description
}

class MaterialOpener(val opening: Boolean, val open: () -> Unit)

/** Opens a course file or page wherever it is listed. Core fetches it with
 * the signed-in LMS session (or from the course folder) and the system viewer
 * shows it; what is not a file opens in the in-app LMS page. A download that
 * only failed this time (offline, timed out) asks instead: silently showing
 * the LMS page made Share hand out a link where it had handed out the file.
 * `courseId` names the course whose folder Core uses when `files` is not
 * known yet. */
@Composable
fun rememberMaterialOpener(title: String, source: String?, files: CourseKey?, courseId: String? = null): MaterialOpener {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var opening by remember { mutableStateOf(false) }
    var retry by remember { mutableStateOf(false) }
    fun push(url: String) = navigator.openPage(WebRouter.PushedPage(SchoolSite.SCIENCE_LMS, url, title))
    lateinit var open: () -> Unit
    open = open@{
        val target = source ?: return@open
        scope.launch {
            opening = true
            try {
                val folder = files ?: courseId?.let {
                    // Core names a folder for every course; none means the read failed.
                    model.courseDetail(it)?.files ?: return@launch run { retry = true }
                }
                if (folder == null) return@launch push(target)
                when (val material = model.openMaterial(folder, target)) {
                    is Material.Ready -> if (!view(context, File(material.file.path))) push(target)
                    is Material.TooLarge -> notice(
                        context,
                        t("This file is too large ({{size}}). Open it in the LMS.", "size" to Formatter.formatShortFileSize(context, material.bytes.toLong())),
                    )
                    is Material.Failed -> when (material.reason) {
                        MaterialFailure.LOGIN -> notice(context, t("Sign in to the LMS again. Check your school account in Settings."))
                        MaterialFailure.NETWORK, MaterialFailure.TIMEOUT, MaterialFailure.CANCELLED -> retry = true
                        else -> push(target)
                    }
                    null -> retry = true
                    is Material.External -> push(material.url)
                    // Folders, pages and anything Core could not fetch: the LMS page itself.
                    else -> push(target)
                }
            } finally {
                opening = false
            }
        }
    }
    if (retry) {
        AlertDialog(
            onDismissRequest = { retry = false },
            text = { Text(t("The file could not be downloaded. Try again, or open it in the LMS.")) },
            confirmButton = {
                TextButton(onClick = {
                    retry = false
                    open()
                }) { Text(t("Try again")) }
            },
            dismissButton = {
                TextButton(onClick = {
                    retry = false
                    source?.let(::push)
                }) { Text(t("Open in LMS")) }
            },
        )
    }
    return MaterialOpener(opening, open)
}

/** Hands a downloaded file to the app that views its type. */
private fun view(context: Context, file: File): Boolean {
    val uri = runCatching { FileProvider.getUriForFile(context, "${context.packageName}.files", file) }.getOrNull() ?: return false
    val type = MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase()) ?: "*/*"
    return try {
        context.startActivity(Intent(Intent.ACTION_VIEW).setDataAndType(uri, type).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
        true
    } catch (error: ActivityNotFoundException) {
        false
    }
}

private fun notice(context: Context, text: String) = Toast.makeText(context, text, Toast.LENGTH_LONG).show()
