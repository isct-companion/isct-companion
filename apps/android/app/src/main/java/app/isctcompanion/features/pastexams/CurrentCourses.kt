package app.isctcompanion.features.pastexams

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import app.isctcompanion.LocalModel
import app.isctcompanion.core.ExamCourseList
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t

/** This quarter's enrolled courses, read from the device. */
@Stable
internal class CurrentCourses {
    var list by mutableStateOf<ExamCourseList?>(null)
    var failed by mutableStateOf(false)
    var attempt by mutableIntStateOf(0)
}

@Composable
internal fun rememberCurrentCourses(): CurrentCourses {
    val model = LocalModel.current
    val state = remember { CurrentCourses() }
    LaunchedEffect(model.contentRevision, state.attempt) {
        state.failed = false
        state.list = model.examCourses()
        state.failed = state.list == null
    }
    return state
}

/** The courses as one card; a course with a subject code opens its subject
 * (Core finds it and the class from the course). */
internal fun LazyListScope.currentCourses(state: CurrentCourses, open: (String) -> Unit) {
    item(key = "courses-label") { SectionLabel(t("examShare.currentCourses")) }
    val shown = state.list
    when {
        shown == null && state.failed -> item(key = "courses-retry") {
            TextButton(onClick = { state.attempt += 1 }) { Text(t("common.retry")) }
        }
        shown == null -> item(key = "courses-loading") {
            Box(Modifier.fillMaxWidth().padding(Theme.space10), Alignment.Center) { CircularProgressIndicator() }
        }
        shown.courses.isEmpty() -> item(key = "courses-empty") {
            Text(t("examShare.noCurrentCourses"), color = Theme.colors.muted)
        }
        else -> cardRows(shown.courses, { it.courseId }) { course ->
            ExamRow(
                course.title,
                if (course.subjectCodes.isEmpty()) t("examShare.noSubjectCode")
                else listOf(course.subjectCodes.joinToString(" / "), course.instructor).filter { it.isNotBlank() }.joinToString(" · "),
                onClick = if (course.subjectCodes.isEmpty()) null else ({ open(course.courseId) }),
            )
        }
    }
}
