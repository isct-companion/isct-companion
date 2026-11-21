package app.isctcompanion.features.pastexams

import app.isctcompanion.design.BackButton
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.ExpandLess
import androidx.compose.material.icons.rounded.ExpandMore
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.ExamAccountStatus
import app.isctcompanion.core.ExamAction
import app.isctcompanion.core.ExamAnswer
import app.isctcompanion.core.ExamSet
import app.isctcompanion.core.ExamSubjectPage
import app.isctcompanion.core.ExamUploadForm
import app.isctcompanion.core.ExamUploadStart
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import kotlinx.coroutines.launch

/** How a subject page was reached: from a course (Core finds its subject and
 * class) or by a subject code. */
sealed interface ExamSubjectEntry {
    data class Course(val courseId: String) : ExamSubjectEntry
    data class Code(val code: String, val classId: UInt?) : ExamSubjectEntry
}

@Composable
internal fun Loading() {
    Box(Modifier.fillMaxSize(), Alignment.Center) { CircularProgressIndicator() }
}

/** One course's sittings as the desktop's panel shows them: the person's own
 * teachers first, the others folded away, a button per document. */
@Composable
fun ExamSubjectScreen(entry: ExamSubjectEntry) {
    val navigator = LocalNavigator.current
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    val links = rememberExamLinks()
    val calls = rememberExamCalls()
    val choices by rememberExamChoices(calls)
    val account = model.examAccount
    val connected = account?.status == ExamAccountStatus.CONNECTED
    var page by remember { mutableStateOf<ExamSubjectPage?>(null) }
    var othersOpen by rememberSaveable { mutableStateOf(false) }
    var menu by remember { mutableStateOf(false) }
    var requesting by rememberSaveable { mutableStateOf(false) }
    var uploadForm by remember { mutableStateOf<ExamUploadForm?>(null) }

    /** The subject again once shown; a course entry resolves it the first time. */
    suspend fun load() {
        val action = page?.let { ExamAction.Subject(it.subject.code, it.classId) } ?: when (entry) {
            is ExamSubjectEntry.Course -> ExamAction.CourseSubject(entry.courseId)
            is ExamSubjectEntry.Code -> ExamAction.Subject(entry.code, entry.classId)
        }
        (calls.call(action) as? ExamAnswer.Subject)?.v1?.let { page = it }
    }

    suspend fun reload(action: ExamAction = ExamAction.Account) {
        if ((calls.call(action) as? ExamAnswer.Account)?.v1?.status == ExamAccountStatus.CONNECTED) load()
    }

    // Returning to the page (from a document, or to the app) reloads it.
    LifecycleResumeEffect(entry) {
        val refresh = scope.launch { reload() }
        onPauseOrDispose { refresh.cancel() }
    }

    val current = page
    ExamFrame(calls) {
        TabPage(
            current?.let { it.subject.name }.orEmpty(),
            navigationIcon = { BackButton { navigator.back() } },
            actions = {
                if (current != null && connected) Box {
                    IconButton(onClick = { menu = true }) { Icon(Icons.Rounded.Add, t("examShare.upload.title")) }
                    DropdownMenu(menu, { menu = false }) {
                        DropdownMenuItem({ Text(t("examShare.upload.title")) }, enabled = !calls.busy && choices != null, onClick = {
                            menu = false
                            scope.launch {
                                uploadForm = (calls.call(ExamAction.UploadForm(ExamUploadStart.Subject(current.subject.code))) as? ExamAnswer.UploadForm)?.v1
                            }
                        })
                        DropdownMenuItem({ Text(t("examShare.newRequest")) }, enabled = choices != null, onClick = { menu = false; requesting = true })
                        DropdownMenuItem({ Text(t("examShare.openOnWeb")) }, onClick = { menu = false; links.open(current.webUrl) })
                    }
                }
            },
        ) {
            when {
                account == null -> Loading()
                !connected -> AccountGate(calls) { reload(it) }
                current != null -> {
                    // A class of one's own puts its teachers' sittings first.
                    val grouped = current.classId != null
                    val mine = current.sets.filter { !grouped || it.mine }
                    val others = if (grouped) current.sets.filter { !it.mine } else emptyList()
                    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(Theme.gutter), verticalArrangement = Arrangement.spacedBy(Theme.space4)) {
                        if (current.sets.isEmpty()) {
                            item { Text(t("examShare.noItems"), color = Theme.colors.muted) }
                        } else {
                            if (mine.isEmpty()) item { Text(t("examShare.noOwnTeacher"), color = Theme.colors.muted) }
                            else cardRows(mine, { it.documents.first().id }) { SetRow(it, withTeachers = !it.mine) }
                            if (others.isNotEmpty()) {
                                item(key = "others") {
                                    Row(
                                        Modifier.fillMaxWidth().clickable { othersOpen = !othersOpen }.padding(vertical = Theme.space4),
                                        verticalAlignment = Alignment.CenterVertically,
                                        horizontalArrangement = Arrangement.spacedBy(Theme.space2),
                                    ) {
                                        Icon(if (othersOpen) Icons.Rounded.ExpandLess else Icons.Rounded.ExpandMore, null, tint = Theme.colors.muted)
                                        Text(t("examShare.otherTeachers", "count" to others.size), color = Theme.colors.muted)
                                    }
                                }
                                if (othersOpen) cardRows(others, { "other:" + it.documents.first().id }) { SetRow(it, withTeachers = true) }
                            }
                        }
                        item { ExamCredit() }
                    }
                }
                calls.busy -> Loading()
                else -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                    TextButton(onClick = { scope.launch { load() } }) { Text(t("common.retry")) }
                }
            }
        }
    }
    val form = uploadForm
    val formChoices = choices
    if (form != null && formChoices != null) {
        UploadSheet(form, formChoices, dismiss = { uploadForm = null }) {
            uploadForm = null
            ExamViews.notice = t("examShare.upload.uploaded")
            scope.launch { load() }
        }
    }
    if (requesting && current != null && formChoices != null) {
        RequestDialog(current, formChoices, calls.busy, calls.error, dismiss = { requesting = false }) { draft ->
            scope.launch {
                if (calls.call(ExamAction.CreateRequest(draft)) != null) {
                    requesting = false
                    ExamViews.notice = t("examShare.requestCreated")
                }
            }
        }
    }
}

/** One sitting: a label, and a button per document (question, answer) — the
 * buttons are what opens, as on the desktop and iOS. */
@Composable
private fun SetRow(set: ExamSet, withTeachers: Boolean) {
    val navigator = LocalNavigator.current
    ExamRow(
        sittingLabel(set.academicYear, set.occasion, set.occasionNumber, set.variant).ifBlank { set.documents.first().fileName },
        if (withTeachers) set.teachers.joinToString(" / ") { it.name } else null,
        onClick = null,
        leading = { Icon(Icons.Rounded.Description, null, tint = Theme.colors.accent, modifier = Modifier.size(20.dp)) },
        trailing = {
            Row(horizontalArrangement = Arrangement.spacedBy(Theme.space3)) {
                set.documents.forEach { document ->
                    val answer = document.role == "answer"
                    val ink = if (answer) Theme.colors.ok else Theme.colors.accent
                    Text(
                        // Core numbers several documents of one role in a sitting.
                        listOfNotNull(
                            document.role?.let { t("examShare.role.$it") } ?: document.fileName,
                            document.number?.toString(),
                        ).joinToString(" "),
                        color = ink,
                        style = MaterialTheme.typography.labelLarge,
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 1,
                        modifier = Modifier
                            .background(if (answer) Theme.colors.okSoft else Theme.colors.accentSoft, CircleShape)
                            .border(1.dp, ink.copy(alpha = 0.35f), CircleShape)
                            .clickable { navigator.openExamDocument(document.id) }
                            .padding(horizontal = Theme.space6, vertical = 5.dp),
                    )
                }
            }
        },
    )
}
