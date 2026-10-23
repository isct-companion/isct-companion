package app.isctcompanion.features.lectures

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.WorkspaceOverview
import app.isctcompanion.core.text
import app.isctcompanion.design.Card
import app.isctcompanion.design.Format
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.Theme
import app.isctcompanion.design.courseColor
import app.isctcompanion.design.t

/** 授業 (wide windows): the quarter's courses, as the desktop Lectures
 * sidebar lists them (`WorkspaceOverview.termCourseIds`); a row pushes the
 * course page. */
@Composable
fun LecturesScreen() {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    var overview by remember { mutableStateOf<WorkspaceOverview?>(null) }
    val term = model.today?.term
    LaunchedEffect(term, model.contentRevision) { term?.let { overview = model.overview(it) } }
    TabPage(t("Courses")) {
        val ids = overview?.termCourseIds.orEmpty()
        if (overview != null && ids.isEmpty()) {
            Box(Modifier.fillMaxSize(), Alignment.Center) { Text(t("No courses this term yet"), color = Theme.colors.muted) }
            return@TabPage
        }
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp)) {
            overview?.term?.let { item { SectionLabel(Format.quarter(it)) } }
            item {
                Card(Modifier.padding(top = 8.dp)) {
                    ids.forEachIndexed { index, id ->
                        if (index > 0) RowDivider(inset = 34.dp)
                        val course = model.records.courses[id]
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                            modifier = Modifier.fillMaxWidth().clickable { navigator.openCourse(id) }.padding(horizontal = 16.dp, vertical = 10.dp),
                        ) {
                            Box(
                                Modifier.width(6.dp).height(32.dp).clip(RoundedCornerShape(4.dp))
                                    .background(courseColor(overview?.colors?.get(id) ?: model.colors[id])),
                            )
                            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                                Text(
                                    course?.text("title") ?: id, fontWeight = FontWeight.Medium, color = Theme.colors.ink,
                                    maxLines = 2, overflow = TextOverflow.Ellipsis,
                                )
                                course?.text("code")?.let {
                                    Text(it, style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace, color = Theme.colors.muted)
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
