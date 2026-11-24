package app.isctcompanion.features.pastexams

import app.isctcompanion.design.BackButton
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.lifecycle.compose.LifecycleResumeEffect
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.ExamAction
import app.isctcompanion.core.ExamAnswer
import app.isctcompanion.core.ExamRequest
import app.isctcompanion.core.ExamUploadForm
import app.isctcompanion.core.ExamUploadStart
import app.isctcompanion.design.Card
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import kotlinx.coroutines.launch

/** One request in full, then the one thing to do with it: share a file for
 * it, see the shared one, or manage one's own. [mine] names the board it was
 * listed on, which Core answers again after a change. */
@Composable
fun ExamRequestScreen(id: String, mine: Boolean) {
    val navigator = LocalNavigator.current
    val scope = rememberCoroutineScope()
    val links = rememberExamLinks()
    val calls = rememberExamCalls()
    val choices by rememberExamChoices(calls)
    var request by remember(id) { mutableStateOf(ExamViews.requests[id]) }
    var missing by remember { mutableStateOf(false) }
    var uploadForm by remember { mutableStateOf<ExamUploadForm?>(null) }
    var deleting by remember { mutableStateOf(false) }

    /** Takes this request from a board Core answered, when it is listed. */
    fun take(answer: ExamAnswer?): Boolean {
        val board = (answer as? ExamAnswer.Requests)?.v1 ?: return false
        ExamViews.board(board)
        board.items.firstOrNull { it.id == id }?.let { request = it }
        return true
    }

    suspend fun refresh() {
        if (take(calls.call(ExamAction.Requests(mine, null)))) missing = request == null
    }

    // Returning to the page (from the answer's document, or to the app) reads the board again.
    LifecycleResumeEffect(id) {
        val job = scope.launch { refresh() }
        onPauseOrDispose { job.cancel() }
    }

    ExamFrame(calls) {
        TabPage(t("examShare.requests"), navigationIcon = { BackButton { navigator.back() } }) {
            val item = request
            when {
                item != null -> Column(
                    Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(Theme.gutter),
                    verticalArrangement = Arrangement.spacedBy(Theme.space6),
                ) {
                    RequestDetail(item)
                    item.answerExamId?.let { answer ->
                        Button(onClick = { navigator.openExamDocument(answer) }, modifier = Modifier.fillMaxWidth()) {
                            Text(t("examShare.viewAnswer"))
                        }
                    }
                    if (!item.mine && item.status == "open") {
                        Button(enabled = !calls.busy && choices != null, onClick = {
                            scope.launch {
                                uploadForm = (calls.call(ExamAction.UploadForm(ExamUploadStart.Request(item))) as? ExamAnswer.UploadForm)?.v1
                            }
                        }, modifier = Modifier.fillMaxWidth()) { Text(t("examShare.upload.answerRequest")) }
                    }
                    if (item.mine) {
                        OutlinedButton(
                            enabled = !calls.busy,
                            onClick = {
                                scope.launch {
                                    val open = item.status != "open"
                                    val answer = calls.call(ExamAction.SetRequestOpen(item.id, open, mine))
                                    // A closed request may leave the open board; its status is then the one asked for.
                                    if (take(answer) && request?.status == item.status) request = item.copy(status = if (open) "open" else "closed")
                                }
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) { Text(t(if (item.status == "open") "examShare.closeRequest" else "examShare.reopenRequest")) }
                        TextButton(enabled = !calls.busy, onClick = { deleting = true }, modifier = Modifier.fillMaxWidth()) {
                            Text(t("common.delete"), color = Theme.colors.danger)
                        }
                    }
                    choices?.site?.let { site ->
                        TextButton(onClick = { links.open(site.requests) }, modifier = Modifier.fillMaxWidth()) {
                            Text(t("examShare.openOnWeb"))
                        }
                    }
                }
                missing -> Box(Modifier.fillMaxSize(), Alignment.Center) { Text(t("error.unavailable"), color = Theme.colors.muted) }
                calls.busy -> Loading()
                else -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                    TextButton(onClick = { scope.launch { refresh() } }) { Text(t("common.retry")) }
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
            navigator.back()
        }
    }
    if (deleting) {
        ConfirmDialog(Confirmation("examShare.deleteRequest", ExamAction.DeleteRequest(id, mine)), { deleting = false }) { action ->
            scope.launch {
                if (take(calls.call(action))) {
                    ExamViews.requests.remove(id)
                    navigator.back()
                }
            }
        }
    }
}

@Composable
private fun RequestDetail(item: ExamRequest) {
    Card {
        Column(Modifier.padding(Theme.space8), verticalArrangement = Arrangement.spacedBy(Theme.space3)) {
            Text(item.subject.code, style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
            Text(item.subject.name, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold)
            StatusTag(item.status)
            requestFacts(item).ifBlank { null }?.let { Text(it, color = Theme.colors.muted) }
            if (item.teachers.isNotEmpty()) Text(item.teachers.joinToString(" / ") { it.name }, color = Theme.colors.muted)
        }
    }
    if (item.description.isNotBlank()) Card {
        Text(item.description, color = Theme.colors.ink, modifier = Modifier.padding(Theme.space8))
    }
}
