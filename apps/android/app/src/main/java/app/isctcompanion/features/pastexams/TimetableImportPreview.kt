package app.isctcompanion.features.pastexams

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.CalendarToday
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.core.ExamCourseImportPreview
import app.isctcompanion.design.Card
import app.isctcompanion.design.EmptyState
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t

@Composable
fun TimetableImportPreview(onBack: (() -> Unit)? = null) {
    val model = LocalModel.current
    val currentTerm = model.records.currentTerm
    var term by rememberSaveable { mutableStateOf<String?>(null) }
    var terms by remember { mutableStateOf(emptyList<String>()) }
    var preview by remember { mutableStateOf<ExamCourseImportPreview?>(null) }
    var failed by remember { mutableStateOf(false) }
    var retry by remember { mutableStateOf(0) }

    LaunchedEffect(term, currentTerm, model.contentRevision, retry) {
        failed = false
        preview = model.examCourseImport(term)
        failed = preview == null
    }
    LaunchedEffect(currentTerm, model.contentRevision) {
        currentTerm?.let { model.overview(it) }?.let { terms = it.allTerms }
    }
    val shown = preview?.takeIf { it.term == (term ?: currentTerm) }
    TabPage(
        title = t("examShare.importPreview"),
        navigationIcon = {
            onBack?.let {
                IconButton(onClick = it) {
                    Icon(Icons.AutoMirrored.Rounded.ArrowBack, t("common.back"))
                }
            }
        },
        actions = { QuarterMenu(term, terms) { term = it } },
    ) {
        LazyColumn(
            Modifier.fillMaxSize(),
            contentPadding = PaddingValues(start = Theme.gutter, end = Theme.gutter, bottom = 24.dp),
        ) {
            item {
                when {
                    shown == null && failed -> EmptyState(
                        Icons.Rounded.ErrorOutline, t("failure.thisIsNotAvailableRight"),
                        action = t("common.retry") to { retry += 1 },
                    )
                    shown == null -> Box(Modifier.fillMaxWidth().padding(24.dp), Alignment.Center) {
                        CircularProgressIndicator()
                    }
                    shown.courses.isEmpty() -> EmptyState(Icons.Rounded.CalendarToday, t("examShare.emptyTimetable"))
                    else -> Card(Modifier.padding(top = 8.dp)) {
                        shown.courses.forEachIndexed { index, course ->
                            if (index > 0) RowDivider()
                            Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp)) {
                                Text(course.title, style = MaterialTheme.typography.bodyLarge, color = Theme.colors.ink)
                                if (course.instructor.isNotEmpty()) {
                                    Text(course.instructor, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted)
                                }
                                Text(
                                    course.subjectCodes.joinToString(" / ").ifEmpty { t("examShare.noSubjectCode") },
                                    style = MaterialTheme.typography.bodySmall,
                                    color = Theme.colors.muted,
                                    fontFamily = FontFamily.Monospace,
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun QuarterMenu(term: String?, terms: List<String>, pick: (String?) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        TextButton(onClick = { open = true }) {
            Text(term ?: t("examShare.currentQuarter"))
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuItem(text = { Text(t("examShare.currentQuarter")) }, onClick = {
                open = false
                pick(null)
            })
            terms.forEach { id ->
                DropdownMenuItem(text = { Text(id) }, onClick = {
                    open = false
                    pick(id)
                })
            }
        }
    }
}
