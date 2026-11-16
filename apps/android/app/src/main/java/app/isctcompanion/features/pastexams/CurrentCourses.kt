package app.isctcompanion.features.pastexams

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.core.ExamCourseList
import app.isctcompanion.design.Card
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t

/** This quarter's enrolled courses, read from the device. A course opens its
 * subject only when tapped. */
@Composable
fun CurrentCourses(enabled: Boolean, open: (String) -> Unit) {
    val model = LocalModel.current
    var list by remember { mutableStateOf<ExamCourseList?>(null) }
    var failed by remember { mutableStateOf(false) }
    var retry by remember { mutableStateOf(0) }

    LaunchedEffect(model.contentRevision, retry) {
        failed = false
        list = model.examCourses()
        failed = list == null
    }
    SectionLabel(t("examShare.currentCourses"))
    val shown = list
    when {
        shown == null && failed -> TextButton(onClick = { retry += 1 }) { Text(t("common.retry")) }
        shown == null -> CircularProgressIndicator()
        shown.courses.isEmpty() -> Text(t("examShare.noCurrentCourses"), color = Theme.colors.muted)
        else -> Card(Modifier.padding(top = 8.dp)) {
            shown.courses.forEachIndexed { index, course ->
                if (index > 0) RowDivider()
                val openable = enabled && course.subjectCodes.isNotEmpty()
                Column(
                    Modifier.fillMaxWidth()
                        .clickable(enabled = openable) { open(course.courseId) }
                        .padding(horizontal = 16.dp, vertical = 10.dp),
                ) {
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
