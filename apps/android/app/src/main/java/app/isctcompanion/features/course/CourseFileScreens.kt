package app.isctcompanion.features.course

import app.isctcompanion.design.BackButton
import app.isctcompanion.design.EmptyState
import app.isctcompanion.design.FileUnavailable
import app.isctcompanion.design.FileViewer
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.Theme
import app.isctcompanion.design.rememberMaterialOpening
import app.isctcompanion.design.t
import app.isctcompanion.CampusPrintActivity
import app.isctcompanion.core.openInAnotherApp
import android.text.format.Formatter
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.OpenInNew
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.FolderZip
import androidx.compose.material.icons.rounded.Print
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.ArchiveFile
import app.isctcompanion.core.Material
import app.isctcompanion.core.Viewer
import java.io.File

/** A downloaded course file the app shows itself (`showsInApp`), with
 * another app a tap away. */
@Composable
fun CourseFileScreen(path: String, title: String, viewer: Viewer) {
    val navigator = LocalNavigator.current
    val context = LocalContext.current
    val file = remember(path) { File(path) }
    TabPage(
        title.ifEmpty { file.name },
        navigationIcon = { BackButton { navigator.back() } },
        actions = {
            // A PDF or a picture goes to the campus printer from here.
            if (viewer == Viewer.PDF || viewer == Viewer.IMAGE) {
                IconButton(onClick = { CampusPrintActivity.print(context, file, title.ifEmpty { file.name }) }) {
                    Icon(Icons.Rounded.Print, t("campusPrint.window.printToCampusPrinter"))
                }
            }
            IconButton(onClick = { openInAnotherApp(context, file) }) {
                Icon(Icons.AutoMirrored.Rounded.OpenInNew, t("common.openInDefaultApp"))
            }
        },
    ) {
        FileViewer(file, viewer) { FileUnavailable(file) }
    }
}

/** A downloaded ZIP Core lists as its files (`Material.Archive`), shown as a
 * folder; each file opens like any course item, extracted by Core when it is
 * opened. */
@Composable
fun CourseArchiveScreen(courseId: String, source: String, title: String) {
    val navigator = LocalNavigator.current
    val model = LocalModel.current
    val context = LocalContext.current
    val opening = rememberMaterialOpening()
    var files by remember(source) { mutableStateOf<List<ArchiveFile>?>(null) }
    var failed by remember(source) { mutableStateOf(false) }
    var attempt by remember(source) { mutableIntStateOf(0) }
    // The archive is on disk by now; opening it again reads its listing.
    LaunchedEffect(source, attempt) {
        failed = false
        when (val material = model.openMaterial(courseId, source)) {
            is Material.Archive -> files = material.files
            else -> failed = true
        }
    }
    TabPage(title, navigationIcon = { BackButton { navigator.back() } }) {
        val listed = files
        when {
            listed != null -> LazyColumn(Modifier.fillMaxSize()) {
                items(listed, key = { it.url }) { entry ->
                    val name = entry.name.substringAfterLast('/')
                    val folder = entry.name.substringBeforeLast('/', "")
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .clickable(enabled = !opening.busy) { opening.open(courseId, entry.url, name) }
                            .padding(horizontal = Theme.rowX, vertical = Theme.rowY),
                        horizontalArrangement = Arrangement.spacedBy(Theme.rowGap),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon(Icons.Rounded.Description, null, tint = Theme.colors.muted)
                        Column(Modifier.weight(1f)) {
                            Text(name, color = Theme.colors.ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
                            // A file inside a folder of the archive names that folder.
                            if (folder.isNotEmpty()) Text(
                                folder,
                                style = MaterialTheme.typography.bodySmall,
                                color = Theme.colors.muted,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                        Text(
                            Formatter.formatShortFileSize(context, entry.bytes.toLong()),
                            style = MaterialTheme.typography.bodySmall,
                            color = Theme.colors.muted,
                        )
                    }
                }
            }
            failed -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                EmptyState(Icons.Rounded.FolderZip, t("lmsFile.theFileCouldNotBe"), action = t("common.retry") to { attempt++ })
            }
            else -> Box(Modifier.fillMaxSize(), Alignment.Center) { CircularProgressIndicator() }
        }
    }
}
