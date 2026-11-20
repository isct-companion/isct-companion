package app.isctcompanion.features.timetable

import android.content.Context
import android.content.Intent
import android.widget.Toast
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CalendarMonth
import androidx.compose.material.icons.rounded.EditCalendar
import androidx.compose.material.icons.rounded.IosShare
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.FileProvider
import app.isctcompanion.LocalModel
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.CalendarExportScope
import app.isctcompanion.design.Format
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
internal fun ExportButton(term: String) {
    val model = LocalModel.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var menu by remember { mutableStateOf(false) }
    fun export(scope0: CalendarExportScope) = scope.launch {
        menu = false
        runCatching { share(context, term, scope0, model.quarterIcs(term, scope0)) }.onFailure {
            Toast.makeText(context, t("settings.main.exportFailedReason", "reason" to AppModel.describe(it)), Toast.LENGTH_LONG).show()
        }
    }
    Box {
        IconButton(onClick = { menu = true }) { Icon(Icons.Rounded.IosShare, t("common.exportICS"), tint = Theme.colors.accent) }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            Text(
                t("common.exportICS") + " · " + Format.quarter(term), style = MaterialTheme.typography.labelMedium,
                color = Theme.colors.muted, modifier = Modifier.padding(horizontal = Theme.space6, vertical = Theme.space3),
            )
            DropdownMenuItem(
                text = { Text(t("common.coursesOnly")) }, leadingIcon = { Icon(Icons.Rounded.CalendarMonth, null) },
                onClick = { export(CalendarExportScope.COURSES) },
            )
            DropdownMenuItem(
                text = { Text(t("common.coursesWeeklyEvents")) }, leadingIcon = { Icon(Icons.Rounded.EditCalendar, null) },
                onClick = { export(CalendarExportScope.COURSES_AND_EVENTS) },
            )
        }
    }
}

private suspend fun share(context: Context, term: String, scope: CalendarExportScope, text: String) {
    val suffix = if (scope == CalendarExportScope.COURSES_AND_EVENTS) "-with-events" else ""
    val file = withContext(Dispatchers.IO) {
        File(context.cacheDir, "exports").apply { mkdirs() }.resolve("IsctCompanion-$term$suffix.ics").apply { writeText(text) }
    }
    val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
    val send = Intent(Intent.ACTION_SEND).setType("text/calendar").putExtra(Intent.EXTRA_STREAM, uri)
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    context.startActivity(Intent.createChooser(send, file.name))
}
