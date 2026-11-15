package app.isctcompanion.features.pastexams

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Color
import android.graphics.pdf.PdfRenderer
import android.os.ParcelFileDescriptor
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.horizontalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import app.isctcompanion.LocalModel
import app.isctcompanion.R
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.ExamAccount
import app.isctcompanion.core.ExamAccountStatus
import app.isctcompanion.core.ExamAction
import app.isctcompanion.core.ExamAnswer
import app.isctcompanion.core.ExamDocument
import app.isctcompanion.core.ExamLibrary
import app.isctcompanion.core.ExamLocalDocument
import app.isctcompanion.core.ExamReportReason
import app.isctcompanion.core.ExamRequest
import app.isctcompanion.core.ExamRequestCursor
import app.isctcompanion.core.ExamRequestDraft
import app.isctcompanion.core.ExamRequestPage
import app.isctcompanion.core.ExamSubject
import app.isctcompanion.core.ExamSubjectPage
import app.isctcompanion.core.ExamUploadOptions
import app.isctcompanion.core.ExamUploadPhase
import app.isctcompanion.core.ExamUploadStatus
import app.isctcompanion.core.SdkException
import app.isctcompanion.design.Card
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import java.io.File
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private enum class Tab(val key: String) {
    Browse("examShare.browse"), Requests("examShare.requests"), Library("examShare.library"),
}

private data class Confirmation(val key: String, val action: ExamAction)
internal val examOccasions = listOf("final", "midterm", "makeup", "quiz", "check", "report", "exercise", "textbook", "mock")

@Composable
fun PastExamsScreen(courseId: String? = null, onBack: (() -> Unit)? = null) {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    val uri = LocalUriHandler.current
    var tab by rememberSaveable { mutableStateOf(Tab.Browse) }
    var account by remember { mutableStateOf<ExamAccount?>(null) }
    var query by rememberSaveable { mutableStateOf("") }
    var subjects by remember { mutableStateOf<List<ExamSubject>?>(null) }
    var subject by remember { mutableStateOf<ExamSubjectPage?>(null) }
    var courseResolved by remember { mutableStateOf(false) }
    var requests by remember { mutableStateOf<ExamRequestPage?>(null) }
    var mine by rememberSaveable { mutableStateOf(false) }
    var library by remember { mutableStateOf<ExamLibrary?>(null) }
    var file by remember { mutableStateOf<ExamLocalDocument?>(null) }
    var busy by remember { mutableStateOf(false) }
    var connecting by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var notice by remember { mutableStateOf<String?>(null) }
    var requestForm by rememberSaveable { mutableStateOf(false) }
    var reportForm by rememberSaveable { mutableStateOf(false) }
    var uploadForm by remember { mutableStateOf<ExamUploadOptions?>(null) }
    var uploadStatus by remember { mutableStateOf<ExamUploadStatus?>(null) }
    var confirmation by remember { mutableStateOf<Confirmation?>(null) }
    var year by rememberSaveable { mutableStateOf("") }
    var teacher by rememberSaveable { mutableStateOf("") }
    var occasion by rememberSaveable { mutableStateOf("") }

    fun displayAccount(updated: ExamAccount) {
        account = updated
        if (updated.status == ExamAccountStatus.UNAVAILABLE) {
            error = null
        }
        if (updated.status != ExamAccountStatus.CONNECTED) {
            courseResolved = false
            confirmation = null
            subjects = null
            subject = null
            file = null
            requests = null
            library = null
            requestForm = false
            reportForm = false
            uploadForm = null
            uploadStatus = null
        }
    }

    suspend fun refreshAccountSnapshot() {
        try {
            (model.examShare(ExamAction.AccountSnapshot) as? ExamAnswer.Account)?.v1?.let(::displayAccount)
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (_: Exception) {
            // Keep the original failure if the app session has already closed.
        }
    }

    suspend fun call(action: ExamAction): ExamAnswer? {
        if (busy) return null
        busy = true
        connecting = action == ExamAction.Connect
        error = null
        notice = null
        return try {
            model.examShare(action)
        } catch (_: SdkException.Cancelled) {
            null
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error = AppModel.describe(e)
            refreshAccountSnapshot()
            null
        } finally {
            connecting = false
            busy = false
        }
    }

    suspend fun open(id: String, download: Boolean = false) {
        (call(ExamAction.Open(id, download)) as? ExamAnswer.Document)?.v1?.let { file = it }
    }

    suspend fun loadRequests(cursor: ExamRequestCursor? = null) {
        (call(ExamAction.Requests(mine, cursor)) as? ExamAnswer.Requests)?.v1?.let {
            requests = it.copy(items = (if (cursor == null) emptyList() else requests?.items.orEmpty()) + it.items)
        }
    }

    suspend fun reload() {
        if (account?.status != ExamAccountStatus.CONNECTED) return
        when (tab) {
            Tab.Browse -> Unit
            Tab.Requests -> loadRequests()
            Tab.Library -> (call(ExamAction.Library) as? ExamAnswer.Library)?.v1?.let {
                library = it
            }
        }
    }

    suspend fun showSubject(action: ExamAction): Boolean {
        when (val answer = call(action)) {
            is ExamAnswer.Subject -> {
                subject = answer.v1
                year = ""
                teacher = ""
                occasion = ""
            }
            is ExamAnswer.Subjects -> {
                subject = null
                subjects = answer.v1
            }
            else -> return false
        }
        return true
    }

    suspend fun updateAccount(action: ExamAction) {
        val updated = (call(action) as? ExamAnswer.Account)?.v1 ?: return
        displayAccount(updated)
        if (updated.status == ExamAccountStatus.CONNECTED) {
            uploadStatus = model.examUploadStatus()
            if (courseId != null && !courseResolved) {
                tab = Tab.Browse
                courseResolved = showSubject(ExamAction.CourseSubject(courseId))
            }
        }
        reload()
    }

    suspend fun startUpload(code: String, request: ExamRequest? = null) {
        uploadForm = (call(ExamAction.UploadOptions(code, request)) as? ExamAnswer.UploadOptions)?.v1
    }

    suspend fun settledUpload(uploaded: Boolean) {
        uploadStatus = model.examUploadStatus()
        if (!uploaded) return
        requests = null
        library = null
        if (tab == Tab.Browse) {
            subject?.subject?.code?.let { code ->
                (call(ExamAction.Subject(code)) as? ExamAnswer.Subject)?.v1?.let { subject = it }
            }
        } else {
            reload()
        }
        notice = t("examShare.upload.uploaded")
    }
    LifecycleResumeEffect(courseId) {
        val refresh = scope.launch { updateAccount(ExamAction.Account) }
        onPauseOrDispose { refresh.cancel() }
    }

    val editingUpload = uploadForm
    if (editingUpload != null) {
        ExamUploadScreen(options = editingUpload, onBack = {
            uploadForm = null
            scope.launch { uploadStatus = model.examUploadStatus() }
        }, onUnsupported = {
            scope.launch { refreshAccountSnapshot() }
        }, onUploaded = {
            // Use the parent scope so the refresh survives dismissing the form.
            scope.launch {
                uploadForm = null
                settledUpload(uploaded = true)
            }
        })
        return
    }
    if (file != null) androidx.activity.compose.BackHandler { file = null }
    val currentFile = file

    TabPage(
        title = t("examShare.title"),
        navigationIcon = {
            val back = if (currentFile != null) ({ file = null }) else onBack
            if (back != null) IconButton(onClick = back) {
                Icon(Icons.AutoMirrored.Rounded.ArrowBack, t("common.back"))
            }
        },
    ) {
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(Theme.gutter), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            item {
                account?.email?.let { Text(it, color = Theme.colors.muted) }
                error?.let { Text(it, color = Theme.colors.danger) }
                notice?.let { Text(it, color = Theme.colors.muted) }
                if (busy) CircularProgressIndicator()
                if (account?.status == ExamAccountStatus.CONNECTED && uploadStatus?.phase == ExamUploadPhase.PENDING) {
                    ExamUploadProgress(uploadStatus, false)
                    TextButton(enabled = !busy, onClick = {
                        scope.launch { settledUpload(call(ExamAction.RecoverUpload) is ExamAnswer.Uploaded) }
                    }) { Text(t("examShare.upload.recover")) }
                }
                if (connecting) TextButton(onClick = {
                    scope.launch {
                        try {
                            model.examShare(ExamAction.CancelConnect)
                        } catch (cancelled: CancellationException) {
                            throw cancelled
                        } catch (failure: Exception) {
                            error = AppModel.describe(failure)
                        }
                    }
                }) { Text(t("common.cancel")) }
                when (account?.status) {
                    ExamAccountStatus.CONNECTED -> TextButton(enabled = !busy, onClick = {
                        confirmation = Confirmation("examShare.disconnectHint", ExamAction.Disconnect)
                    }) { Text(t("examShare.disconnect")) }
                    ExamAccountStatus.NEEDS_CONSENT -> Row {
                        TextButton(onClick = { uri.openUri("https://sciencetokyoexam.com/terms") }) { Text(t("examShare.terms")) }
                        TextButton(onClick = { uri.openUri("https://sciencetokyoexam.com/privacy") }) { Text(t("examShare.privacy")) }
                        TextButton(enabled = !busy, onClick = { scope.launch { updateAccount(ExamAction.AcceptTerms) } }) { Text(t("examShare.accept")) }
                    }
                    ExamAccountStatus.UNAVAILABLE -> Text(t("error.exam-share-unsupported"))
                    ExamAccountStatus.DISCONNECTED, null -> TextButton(enabled = !busy, onClick = {
                        scope.launch { updateAccount(ExamAction.Connect) }
                    }) { Text(t("examShare.connect")) }
                }
            }
            if (account?.status == ExamAccountStatus.UNAVAILABLE) return@LazyColumn
            item {
                Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Tab.entries.forEach { choice ->
                        FilterChip(selected = tab == choice, enabled = !busy, onClick = {
                            tab = choice; file = null; error = null; scope.launch { reload() }
                        }, label = { Text(t(choice.key)) })
                    }
                }
            }
            if (currentFile != null) {
                item {
                    val id = currentFile.document.id
                    SectionLabel(examLabel(currentFile.document))
                    if (currentFile.offline) Text(t("examShare.offline"))
                    Row(Modifier.horizontalScroll(rememberScrollState())) {
                        TextButton(enabled = !busy && !currentFile.downloaded && !currentFile.offline, onClick = {
                            scope.launch { open(id, true) }
                        }) { Text(t(if (currentFile.downloaded) "examShare.downloaded" else "examShare.download")) }
                        currentFile.saved?.let { saved ->
                            TextButton(enabled = !busy && !currentFile.offline, onClick = {
                                scope.launch {
                                    val answer = call(ExamAction.Save(id, !saved)) as? ExamAnswer.Saved
                                    if (answer != null) file = currentFile.copy(saved = answer.v1)
                                }
                            }) { Text(t(if (saved) "examShare.unsave" else "examShare.save")) }
                        }
                        TextButton(enabled = !busy && !currentFile.offline, onClick = { reportForm = true }) { Text(t("examShare.report")) }
                    }
                }
                item { NativeDocument(currentFile) }
            } else if (tab == Tab.Browse) {
                if (account?.status == ExamAccountStatus.CONNECTED) {
                    item {
                        OutlinedTextField(query, { query = it.take(100) }, label = { Text(t("examShare.search")) }, singleLine = true, modifier = Modifier.fillMaxWidth())
                        TextButton(enabled = !busy && query.isNotBlank(), onClick = {
                            scope.launch { showSubject(ExamAction.Search(query)) }
                        }) { Text(t("examShare.search")) }
                    }
                    val selected = subject
                    if (selected != null) {
                        item {
                            SectionLabel(selected.subject.name)
                            TextButton(onClick = { subject = null }) { Text(t("common.back")) }
                            TextButton(onClick = { uri.openUri(selected.webUrl) }) { Text(t("examShare.openOnWeb")) }
                            TextButton(enabled = !busy, onClick = { requestForm = true }) { Text(t("examShare.newRequest")) }
                            TextButton(enabled = !busy, onClick = {
                                scope.launch { startUpload(selected.subject.code) }
                            }) { Text(t("examShare.upload.title")) }
                            ChoiceRow("examShare.allYears", year, selected.exams.mapNotNull { it.academicYear?.toString() }.distinct().sortedDescending().map { it to it }) { year = it }
                            ChoiceRow("examShare.allTeachers", teacher, selected.teachers.map { it.id to it.name }) { teacher = it }
                            ChoiceRow("examShare.allOccasions", occasion, examOccasions.map { it to t("examShare.occasion.$it") }) { occasion = it }
                        }
                        val exams = selected.exams.filter {
                            (year.isEmpty() || it.academicYear?.toString() == year) &&
                                (teacher.isEmpty() || it.teachers.any { x -> x.id == teacher }) &&
                                (occasion.isEmpty() || it.occasion == occasion)
                        }
                        if (exams.isEmpty()) item { Text(t("examShare.noItems")) }
                        items(exams, key = { it.id }) { exam -> TextButton(enabled = !busy, onClick = { scope.launch { open(exam.id) } }) { Text(examLabel(exam)) } }
                    } else {
                        if (subjects?.isEmpty() == true) item { Text(t("examShare.noItems")) }
                        items(subjects.orEmpty(), key = { it.code }) { hit ->
                            TextButton(enabled = !busy, onClick = {
                                scope.launch { showSubject(ExamAction.Subject(hit.code)) }
                            }) { Text("${hit.name} · ${hit.code}") }
                        }
                        item {
                            CurrentCourses(enabled = !busy) { id ->
                                scope.launch { showSubject(ExamAction.CourseSubject(id)) }
                            }
                        }
                    }
                }
            } else if (account?.status == ExamAccountStatus.CONNECTED && tab == Tab.Requests) {
                item {
                    ChoiceRow("examShare.openRequests", if (mine) "mine" else "", listOf("mine" to t("examShare.myRequests"))) {
                        mine = it == "mine"; requests = null; scope.launch { loadRequests() }
                    }
                    TextButton(onClick = { tab = Tab.Browse }) { Text(t("examShare.newRequest")) }
                }
                if (requests?.items?.isEmpty() == true) item { Text(t("examShare.noItems")) }
                items(requests?.items.orEmpty(), key = { it.id }) { request ->
                    Card {
                        Column(Modifier.padding(12.dp)) {
                            Text(request.subject.name)
                            Text(listOfNotNull(request.academicYear?.toString(), t("examShare.requestStatus.${request.status}"), request.description.ifEmpty { null }).joinToString(" · "), color = Theme.colors.muted)
                            request.answerExamId?.let { id ->
                                TextButton(enabled = !busy, onClick = { scope.launch { open(id) } }) { Text(t("examShare.viewAnswer")) }
                            }
                            if (request.status == "open") {
                                TextButton(enabled = !busy, onClick = {
                                    scope.launch { startUpload(request.subject.code, request) }
                                }) { Text(t("examShare.upload.answerRequest")) }
                            }
                            if (request.mine) Row {
                                TextButton(enabled = !busy, onClick = {
                                    scope.launch { if (call(ExamAction.SetRequestOpen(request.id, request.status != "open")) != null) loadRequests() }
                                }) { Text(t(if (request.status == "open") "examShare.closeRequest" else "examShare.reopenRequest")) }
                                TextButton(enabled = !busy, onClick = { confirmation = Confirmation("examShare.deleteRequest", ExamAction.DeleteRequest(request.id)) }) { Text(t("common.delete")) }
                            }
                        }
                    }
                }
                requests?.cursor?.let { cursor -> item {
                    TextButton(enabled = !busy, onClick = { scope.launch { loadRequests(cursor) } }) { Text(t("examShare.more")) }
                } }
            } else if (account?.status == ExamAccountStatus.CONNECTED) {
                library?.let { content ->
                    val groups = if (content.offline) {
                        item { Text(t("examShare.libraryUnavailable"), color = Theme.colors.muted) }
                        listOf("examShare.downloaded" to content.downloaded)
                    } else {
                        listOf("examShare.saved" to content.saved, "examShare.downloaded" to content.downloaded, "examShare.submitted" to content.submitted)
                    }
                    groups.forEach { (key, exams) ->
                        item { SectionLabel(t(key)); if (exams.isEmpty()) Text(t("examShare.noItems")) }
                        items(exams, key = { "$key:${it.id}" }) { exam ->
                            Row {
                                TextButton(modifier = Modifier.weight(1f), enabled = !busy, onClick = { scope.launch { open(exam.id) } }) { Text(examLabel(exam)) }
                                if (key == "examShare.downloaded" || key == "examShare.submitted") {
                                    TextButton(enabled = !busy, onClick = {
                                        confirmation = if (key == "examShare.downloaded")
                                            Confirmation("examShare.removeDownload", ExamAction.RemoveDownload(exam.id))
                                        else Confirmation("examShare.deleteSubmission", ExamAction.DeleteSubmission(exam.id))
                                    }) { Text(t("common.delete")) }
                                }
                            }
                        }
                    }
                }
            }
            // English only by design: it names the service and its operator.
            item {
                Image(
                    painterResource(R.drawable.past_exams_credit),
                    "Supported by ScienceTokyoExam · teduryu",
                    Modifier.fillMaxWidth().padding(top = 12.dp).height(11.dp),
                    alignment = Alignment.CenterEnd,
                    contentScale = ContentScale.Fit,
                    colorFilter = ColorFilter.tint(Theme.colors.muted),
                )
            }
        }
    }
    confirmation?.let {
        AlertDialog(onDismissRequest = { confirmation = null }, title = { Text(t(it.key)) },
            confirmButton = { TextButton(onClick = {
                confirmation = null
                scope.launch {
                    if (it.action == ExamAction.Disconnect) updateAccount(it.action)
                    else if (call(it.action) != null) reload()
                }
            }) { Text(t("common.ok")) } },
            dismissButton = { TextButton(onClick = { confirmation = null }) { Text(t("common.cancel")) } })
    }
    subject?.takeIf { requestForm }?.let {
        RequestDialog(it, busy, error, { requestForm = false }) { draft ->
            scope.launch { if (call(ExamAction.CreateRequest(draft)) != null) { requestForm = false; notice = t("examShare.requestCreated") } }
        }
    }
    currentFile?.takeIf { reportForm }?.let {
        ReportDialog(busy, error, { reportForm = false }) { reason, detail ->
            scope.launch {
                if (call(ExamAction.Report(it.document.id, reason, detail)) != null) { reportForm = false; notice = t("examShare.reported") }
            }
        }
    }
}

@Composable
private fun ChoiceRow(empty: String, selected: String, choices: List<Pair<String, String>>, pick: (String) -> Unit) {
    Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        (listOf("" to t(empty)) + choices).forEach { (id, label) ->
            FilterChip(selected = selected == id, onClick = { pick(id) }, label = { Text(label) })
        }
    }
}

@Composable
private fun RequestDialog(subject: ExamSubjectPage, busy: Boolean, error: String?, dismiss: () -> Unit, submit: (ExamRequestDraft) -> Unit) {
    var classId by remember { mutableStateOf("") }
    var year by remember { mutableStateOf("") }
    var occasion by remember { mutableStateOf("") }
    var role by remember { mutableStateOf("") }
    var description by remember { mutableStateOf("") }
    AlertDialog(onDismissRequest = dismiss, title = { Text(t("examShare.newRequest")) },
        text = {
            Column {
                Text(subject.subject.name)
                ChoiceRow("examShare.allClasses", classId, subject.rows.mapNotNull { row ->
                    row.classIds.firstOrNull()?.let { id -> id.toString() to (row.labels + row.teachers.map { it.name } + row.taughtIn).joinToString(" · ") }
                }) { classId = it }
                OutlinedTextField(year, { year = it.take(4) }, label = { Text(t("examShare.year")) }, singleLine = true)
                ChoiceRow("examShare.allOccasions", occasion, examOccasions.map { it to t("examShare.occasion.$it") }) { occasion = it }
                ChoiceRow("examShare.wantedRole", role, listOf("question", "answer").map { it to t("examShare.role.$it") }) { role = it }
                OutlinedTextField(description, { description = it.take(1000) }, label = { Text(t("examShare.description")) })
                error?.let { Text(it, color = Theme.colors.danger) }
            }
        },
        confirmButton = { TextButton(enabled = !busy && (year.isBlank() || year.toUIntOrNull() != null), onClick = {
            submit(ExamRequestDraft(subject.subject.code, classId.toUIntOrNull(), year.toUIntOrNull(), occasion.ifEmpty { null }, role.ifEmpty { null }, description))
        }) { Text(t("examShare.newRequest")) } },
        dismissButton = { TextButton(onClick = dismiss) { Text(t("common.cancel")) } })
}

@Composable
private fun ReportDialog(busy: Boolean, error: String?, dismiss: () -> Unit, submit: (ExamReportReason, String) -> Unit) {
    val reasons = listOf(
        ExamReportReason.COPYRIGHT to "copyright", ExamReportReason.INAPPROPRIATE to "inappropriate",
        ExamReportReason.WRONG_ANSWER to "wrongAnswer", ExamReportReason.BROKEN_FILE to "brokenFile", ExamReportReason.OTHER to "other",
    )
    var reason by remember { mutableStateOf(reasons.first().first) }
    var detail by remember { mutableStateOf("") }
    AlertDialog(onDismissRequest = dismiss, title = { Text(t("examShare.report")) },
        text = { Column {
            reasons.forEach { (value, key) ->
                FilterChip(selected = reason == value, onClick = { reason = value }, label = { Text(t("examShare.reason.$key")) })
            }
            OutlinedTextField(detail, { detail = it.take(900) }, label = { Text(t("examShare.description")) })
            error?.let { Text(it, color = Theme.colors.danger) }
        } },
        confirmButton = { TextButton(enabled = !busy, onClick = { submit(reason, detail) }) { Text(t("examShare.report")) } },
        dismissButton = { TextButton(onClick = dismiss) { Text(t("common.cancel")) } })
}

@Composable
private fun NativeDocument(file: ExamLocalDocument) {
    if (file.document.fileType == "text/plain") {
        val content by produceState<String?>(null, file.path) {
            value = withContext(Dispatchers.IO) {
                runCatching { File(file.path).readText() }.getOrElse { t("error.exam-share-response") }
            }
        }
        Text(content ?: t("common.loading"))
        return
    }
    val isPdf = file.document.fileType == "application/pdf"
    var page by rememberSaveable(file.path) { mutableStateOf(0) }
    val result by produceState<Result<Pair<Bitmap, Int>>?>(null, file.path, page) {
        value = withContext(Dispatchers.IO) {
            runCatching {
                if (!isPdf) {
                    val options = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                    BitmapFactory.decodeFile(file.path, options)
                    options.inJustDecodeBounds = false
                    options.inSampleSize = maxOf(1, maxOf(options.outWidth, options.outHeight) / 1600)
                    val bitmap = checkNotNull(BitmapFactory.decodeFile(file.path, options))
                    bitmap to 1
                }
                else ParcelFileDescriptor.open(File(file.path), ParcelFileDescriptor.MODE_READ_ONLY).use { descriptor ->
                    PdfRenderer(descriptor).use { pdf ->
                        pdf.openPage(page).use { source ->
                            val scale = (1400f / maxOf(source.width, source.height)).coerceAtMost(2f)
                            val bitmap = Bitmap.createBitmap((source.width * scale).toInt().coerceAtLeast(1), (source.height * scale).toInt().coerceAtLeast(1), Bitmap.Config.ARGB_8888)
                            bitmap.eraseColor(Color.WHITE)
                            source.render(bitmap, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
                            bitmap to pdf.pageCount
                        }
                    }
                }
            }
        }
    }
    val preview = result
    val shown = preview?.getOrNull()
    if (shown == null) {
        Text(t(if (preview == null) "common.loading" else "error.exam-share-response"))
    } else {
        val (bitmap, pages) = shown
        if (pages > 1) Row {
            TextButton(enabled = page > 0, onClick = { page-- }) { Text(t("common.back")) }
            Text("${page + 1} / $pages", modifier = Modifier.padding(top = 12.dp))
            TextButton(enabled = page + 1 < pages, onClick = { page++ }) { Text(t("examShare.more")) }
        }
        Image(bitmap.asImageBitmap(), file.document.fileName, Modifier.fillMaxWidth().aspectRatio(bitmap.width.toFloat() / bitmap.height))
    }
}

private fun examLabel(exam: ExamDocument): String = listOfNotNull(
    exam.subjects.joinToString(" / ") { it.name },
    exam.academicYear?.toString(),
    exam.occasion?.let {
        val label = t("examShare.occasion.$it")
        if (exam.occasionNumber > 1u) "$label ${exam.occasionNumber}" else label
    },
    exam.variant,
    exam.role?.let { t("examShare.role.$it") },
    exam.teachers.joinToString(" / ") { it.name },
    exam.note,
).filter { it.isNotBlank() }.joinToString(" · ").ifEmpty { exam.fileName }
