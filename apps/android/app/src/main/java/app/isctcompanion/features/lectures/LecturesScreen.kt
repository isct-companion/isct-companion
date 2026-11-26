package app.isctcompanion.features.lectures

import app.isctcompanion.design.CourseSquare
import app.isctcompanion.design.SkeletonRows
import app.isctcompanion.design.Loading
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.School
import app.isctcompanion.design.EmptyState
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.LectureList
import app.isctcompanion.design.Card
import app.isctcompanion.design.Format
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.Theme
import app.isctcompanion.design.courseZoomSource
import app.isctcompanion.design.t

/** 授業 (wide windows): the quarter's courses, as the desktop Lectures
 * sidebar lists them (`LectureList.courses`); a row pushes the
 * course page. */
@Composable
fun LecturesScreen() {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    var lectures by remember { mutableStateOf<LectureList?>(null) }
    val term = model.today?.term
    LaunchedEffect(term, model.contentRevision) { term?.let { lectures = model.lectureList(it) } }
    TabPage(t("common.courses")) {
        val courses = lectures?.courses.orEmpty()
        Loading(
            lectures != null,
            placeholder = { SkeletonRows(6, Modifier.padding(horizontal = Theme.gutter, vertical = Theme.space4)) },
        ) {
            if (courses.isEmpty()) {
                Box(Modifier.fillMaxSize(), Alignment.Center) { EmptyState(Icons.Rounded.School, t("lectures.noCoursesThisTermYet")) }
            } else LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(start = Theme.gutter, end = Theme.gutter, bottom = Theme.space10)) {
                term?.let { item { SectionLabel(Format.quarter(it)) } }
                item {
                    Card(Modifier.padding(top = Theme.space4)) {
                        courses.forEachIndexed { index, course ->
                            val id = course.id
                            if (index > 0) RowDivider(inset = 34.dp)
                            val meeting = lectures?.nextByCourse?.get(id)
                            val timing = meeting?.let {
                                if (it.inClass) t("common.now") else "${Format.dayTitle(it.date)} ${it.startTime}"
                            }
                            val detail = listOfNotNull(course.code, timing, meeting?.room).filter { it.isNotEmpty() }.joinToString(" · ")
                            Row(
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(Theme.rowGap),
                                modifier = Modifier.fillMaxWidth().courseZoomSource("lecture-$id")
                                    .clickable { navigator.openCourse(id, "lecture-$id") }.padding(horizontal = Theme.rowX, vertical = Theme.rowY),
                            ) {
                                CourseSquare(course.color)
                                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(Theme.space1)) {
                                    Text(
                                        course.title, fontWeight = FontWeight.Medium, color = Theme.colors.ink,
                                        maxLines = 2, overflow = TextOverflow.Ellipsis,
                                    )
                                    if (detail.isNotEmpty()) {
                                        Text(detail, style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace, color = Theme.colors.muted)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
