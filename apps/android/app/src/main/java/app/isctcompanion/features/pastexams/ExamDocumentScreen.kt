package app.isctcompanion.features.pastexams

import app.isctcompanion.design.BackButton
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.MoreVert
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.ExamAction
import app.isctcompanion.core.ExamAnswer
import app.isctcompanion.core.ExamChoices
import app.isctcompanion.core.ExamLocalDocument
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.FileViewer
import app.isctcompanion.design.shareCopy
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import java.io.File
import kotlinx.coroutines.launch

/** A past exam full screen in the app's file viewers (PDFs scroll and
 * zoom, photos zoom, Markdown formatted, text as text); actions in the menu. */
@Composable
fun ExamDocumentScreen(id: String) {
    val navigator = LocalNavigator.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val links = rememberExamLinks()
    val model = LocalModel.current
    val calls = rememberExamCalls()
    var file by remember(id) { mutableStateOf<ExamLocalDocument?>(null) }
    var menu by remember { mutableStateOf(false) }
    // The report form's choices, read as it opens; set while it is open.
    var reporting by remember { mutableStateOf<ExamChoices?>(null) }

    suspend fun open(download: Boolean = false) {
        (calls.call(ExamAction.Open(id, download)) as? ExamAnswer.Document)?.v1?.let {
            file = it
            if (download) model.notices.show(t("examShare.downloaded"))
        }
    }
    LaunchedEffect(id) { open() }

    val current = file
    ExamFrame(calls) {
        TabPage(
            current?.let { documentTitle(it.document) }.orEmpty(),
            navigationIcon = { BackButton { navigator.back() } },
            actions = {
                if (current != null) Box {
                    IconButton(onClick = { menu = true }) { Icon(Icons.Rounded.MoreVert, t("examShare.more")) }
                    DropdownMenu(menu, { menu = false }) {
                        DropdownMenuItem(
                            { Text(t(if (current.downloaded) "examShare.downloaded" else "examShare.download")) },
                            { menu = false; scope.launch { open(download = true) } },
                            enabled = !calls.busy && !current.downloaded && !current.offline,
                        )
                        current.saved?.let { saved ->
                            DropdownMenuItem(
                                { Text(t(if (saved) "examShare.unsave" else "examShare.save")) },
                                {
                                    menu = false
                                    scope.launch {
                                        (calls.call(ExamAction.Save(id, !saved)) as? ExamAnswer.Saved)?.v1?.let {
                                            file = current.copy(saved = it)
                                        }
                                    }
                                },
                                enabled = !calls.busy && !current.offline,
                            )
                        }
                        // The opened copy under the uploader's name (Core's exportName).
                        DropdownMenuItem({ Text(t("common.share")) }, {
                            menu = false
                            scope.launch {
                                shareCopy(context, File(current.path), current.exportName, current.document.fileType)
                            }
                        })
                        DropdownMenuItem({ Text(t("examShare.openOnWeb")) }, { menu = false; links.open(current.webUrl) })
                        DropdownMenuItem(
                            { Text(t("examShare.report"), color = Theme.colors.danger) },
                            { menu = false; scope.launch { reporting = calls.choices() } },
                            enabled = !calls.busy && !current.offline,
                        )
                    }
                }
            },
        ) {
            when {
                current != null -> Column(Modifier.fillMaxSize()) {
                    // A downloaded copy shown without the site.
                    if (current.offline) Text(
                        t("examShare.offline"),
                        style = MaterialTheme.typography.bodySmall,
                        color = Theme.colors.muted,
                        modifier = Modifier.padding(horizontal = Theme.gutter, vertical = Theme.space2),
                    )
                    Box(Modifier.weight(1f).fillMaxWidth()) { DocumentView(current) }
                }
                calls.busy -> Loading()
                else -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                    TextButton(onClick = { scope.launch { open() } }) { Text(t("common.retry")) }
                }
            }
        }
    }
    val choices = reporting
    if (choices != null && current != null) {
        ReportDialog(choices.reportReasons, choices.limits.reportDetail, calls.busy, calls.error, dismiss = { reporting = null }) { reason, detail ->
            scope.launch {
                if (calls.call(ExamAction.Report(id, reason, detail)) != null) {
                    reporting = null
                    model.notices.show(t("examShare.reported"))
                }
            }
        }
    }
}

@Composable
private fun DocumentView(file: ExamLocalDocument) {
    // How it shows is Core's (`file_preview`): the site stores Markdown as
    // text/plain, and Core names it Markdown by the upload's name.
    val source = remember(file.path) { File(file.path) }
    FileViewer(source, file.viewer) {
        Box(Modifier.fillMaxSize(), Alignment.Center) {
            Text(t("error.unavailable"), color = Theme.colors.muted)
        }
    }
}
