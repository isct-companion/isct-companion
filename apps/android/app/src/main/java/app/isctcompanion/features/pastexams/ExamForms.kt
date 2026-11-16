package app.isctcompanion.features.pastexams

import android.app.Activity
import android.content.Context
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.Uri
import android.provider.OpenableColumns
import android.text.format.Formatter
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.IntentSenderRequest
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.DocumentScanner
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.Image
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.isctcompanion.core.ExamAction
import app.isctcompanion.core.ExamAnswer
import app.isctcompanion.core.ExamReportReason
import app.isctcompanion.core.ExamRequestDraft
import app.isctcompanion.core.ExamSubjectPage
import app.isctcompanion.core.ExamTeacher
import app.isctcompanion.core.ExamUploadDraft
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import com.google.mlkit.vision.documentscanner.GmsDocumentScannerOptions
import com.google.mlkit.vision.documentscanner.GmsDocumentScanning
import com.google.mlkit.vision.documentscanner.GmsDocumentScanningResult
import java.io.File
import java.time.LocalDate
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private val occasions = listOf("final", "midterm", "makeup", "quiz", "check", "report", "exercise", "textbook", "mock")
/** Held several times a term, so the number matters (Core's `NUMBERED`). */
private val numbered = setOf("quiz", "check", "report")
private val roles = listOf("question", "answer", "both", "notes", "other")
private val photoExtensions = setOf("jpg", "jpeg", "png", "webp")
/** Requests and uploads name a recent year; older ones are rare. */
private const val YEARS_BACK = 8

/** What an upload belongs to: a course, or the request it answers. */
data class ExamUploadTarget(
    val subjectCode: String,
    val subjectName: String,
    val teachers: List<ExamTeacher>,
    val mine: List<String>,
    val academicYear: UInt? = null,
    val occasion: String? = null,
    val role: String? = null,
    val requestId: String? = null,
)

private class Picked(val file: File) {
    val isPhoto: Boolean get() = file.extension.lowercase() in photoExtensions
}

/** A private copy Core reads: picked content may vanish once the picker
 * closes. Photos become JPEG, since the site takes no HEIC. */
private fun copyIn(context: Context, uri: Uri, photo: Boolean): File? = runCatching {
    val folder = File(context.cacheDir, "exam-upload/${UUID.randomUUID()}").apply { mkdirs() }
    if (photo) {
        val bitmap = ImageDecoder.decodeBitmap(ImageDecoder.createSource(context.contentResolver, uri)) { decoder, _, _ ->
            decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
        }
        val name = (displayName(context, uri)?.substringBeforeLast('.') ?: "photo") + ".jpg"
        File(folder, name).also { out -> out.outputStream().use { bitmap.compress(Bitmap.CompressFormat.JPEG, 90, it) } }
    } else {
        val name = displayName(context, uri) ?: "file"
        File(folder, name).also { out ->
            context.contentResolver.openInputStream(uri)!!.use { input -> out.outputStream().use { input.copyTo(it) } }
        }
    }
}.getOrNull()

private fun displayName(context: Context, uri: Uri): String? =
    context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
        if (cursor.moveToFirst()) cursor.getString(0) else null
    }

/**
 * Shares a past exam. Pages come from ML Kit's document scanner (it finds
 * the edges and straightens each page), the photo picker or Files; Core
 * checks the site's rules again and turns photos into one PDF.
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
internal fun UploadSheet(target: ExamUploadTarget, dismiss: () -> Unit, done: () -> Unit) {
    val store = LocalExamStore.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val files = remember { mutableStateListOf<Picked>() }
    var role by remember { mutableStateOf(target.role ?: "question") }
    val thisYear = LocalDate.now().year
    var year by remember { mutableStateOf<UInt?>(target.academicYear ?: thisYear.toUInt()) }
    var occasion by remember { mutableStateOf(target.occasion ?: "") }
    var number by remember { mutableIntStateOf(1) }
    val teachers = remember { mutableStateListOf<String>().apply { addAll(target.mine) } }
    var provenance by remember { mutableStateOf("original") }
    var answerSource by remember { mutableStateOf("unknown") }
    var note by remember { mutableStateOf("") }
    var sending by remember { mutableStateOf(false) }

    fun add(uris: List<Uri>, photo: Boolean) {
        scope.launch {
            val copied = withContext(Dispatchers.IO) { uris.mapNotNull { copyIn(context, it, photo || isPhotoUri(context, it)) } }
            files.addAll(copied.map(::Picked))
        }
    }
    val photos = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(20)) { add(it, photo = true) }
    val documents = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { add(it, photo = false) }
    val scanner = rememberLauncherForActivityResult(ActivityResultContracts.StartIntentSenderForResult()) { result ->
        if (result.resultCode != Activity.RESULT_OK) return@rememberLauncherForActivityResult
        val pages = GmsDocumentScanningResult.fromActivityResultIntent(result.data)?.pages.orEmpty()
        add(pages.map { it.imageUri }, photo = true)
    }
    fun scan() {
        val activity = context as? Activity ?: return
        val options = GmsDocumentScannerOptions.Builder()
            .setScannerMode(GmsDocumentScannerOptions.SCANNER_MODE_FULL)
            .setResultFormats(GmsDocumentScannerOptions.RESULT_FORMAT_JPEG)
            .setGalleryImportAllowed(true)
            .build()
        GmsDocumentScanning.getClient(options).getStartScanIntent(activity)
            .addOnSuccessListener { scanner.launch(IntentSenderRequest.Builder(it).build()) }
            .addOnFailureListener { store.problem = it.localizedMessage }
    }

    val sitting = role != "notes" && role != "other"
    val question = role == "question" || role == "both"
    val answer = role == "answer" || role == "both"
    val mixed = files.size > 1 && !files.all { it.isPhoto }
    val ready = files.isNotEmpty() && !mixed && (!sitting || occasion.isNotEmpty()) && !sending

    ModalBottomSheet(onDismissRequest = { if (!sending) dismiss() }, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = Theme.gutter).padding(bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(t("examShare.uploadTitle", "subject" to target.subjectName), style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold)

            SectionLabel(t("examShare.uploadFiles"))
            files.forEach { picked ->
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Icon(if (picked.isPhoto) Icons.Rounded.Image else Icons.Rounded.Description, null, tint = Theme.colors.muted)
                    Text(picked.file.name, Modifier.weight(1f), maxLines = 1)
                    Text(Formatter.formatShortFileSize(context, picked.file.length()), color = Theme.colors.muted, style = MaterialTheme.typography.bodySmall)
                    IconButton(onClick = { files.remove(picked) }) { Icon(Icons.Rounded.Close, t("examShare.removeFile")) }
                }
            }
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = ::scan) { Icon(Icons.Rounded.DocumentScanner, null); Text(" " + t("examShare.scan")) }
                OutlinedButton(onClick = {
                    photos.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
                }) { Icon(Icons.Rounded.Image, null); Text(" " + t("examShare.photos")) }
                OutlinedButton(onClick = {
                    documents.launch(arrayOf("application/pdf", "image/jpeg", "image/png", "image/webp", "text/plain"))
                }) { Icon(Icons.Rounded.Folder, null); Text(" " + t("examShare.files")) }
            }
            Text(
                t(if (mixed) "examShare.onlyPhotosMerge" else "examShare.uploadFilesHint"),
                color = if (mixed) Theme.colors.danger else Theme.colors.muted,
                style = MaterialTheme.typography.bodySmall,
            )

            Choices(t("examShare.kind"), roles.map { it to t("examShare.role.$it") }, role) { role = it }
            if (sitting) {
                Choices(
                    t("examShare.year"),
                    (0 until YEARS_BACK).map { "${thisYear - it}" to "${thisYear - it}" } + ("" to t("examShare.unknownYear")),
                    year?.toString() ?: "",
                ) { year = it.toUIntOrNull() }
                Choices(t("examShare.occasion"), (occasions + "unknown").map { it to t("examShare.occasion.$it") }, occasion) { occasion = it }
                if (occasion in numbered) {
                    Choices(t("examShare.occasionNumber"), (1..15).map { "$it" to "$it" }, "$number") { number = it.toInt() }
                }
            }
            if (target.teachers.isNotEmpty()) {
                SectionLabel(t("examShare.teachers"))
                FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    target.teachers.forEach { teacher ->
                        FilterChip(
                            selected = teacher.id in teachers,
                            onClick = { if (!teachers.remove(teacher.id)) teachers.add(teacher.id) },
                            label = { Text(teacher.name) },
                        )
                    }
                }
            }
            if (question) Choices(t("examShare.provenance"), listOf("original", "reconstructed").map { it to t("examShare.provenance.$it") }, provenance) { provenance = it }
            if (answer) Choices(t("examShare.answerSource"), listOf("official", "student", "unknown").map { it to t("examShare.answerSource.$it") }, answerSource) { answerSource = it }
            OutlinedTextField(note, { note = it.take(40) }, label = { Text(t("examShare.note")) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            Text(t("examShare.uploadPublic"), color = Theme.colors.muted, style = MaterialTheme.typography.bodySmall)
            store.problem?.let { Text(it, color = Theme.colors.danger) }

            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End)) {
                TextButton(onClick = dismiss, enabled = !sending) { Text(t("common.cancel")) }
                Button(enabled = ready, onClick = {
                    sending = true
                    scope.launch {
                        val draft = ExamUploadDraft(
                            files = files.map { it.file.path },
                            subjectCodes = listOf(target.subjectCode),
                            teacherIds = target.teachers.map { it.id }.filter { it in teachers },
                            academicYear = if (sitting) year else null,
                            occasion = if (sitting) occasion else null,
                            occasionNumber = if (occasion in numbered) number.toUInt() else 0u,
                            variant = "",
                            role = role,
                            provenance = if (question) provenance else null,
                            answerSource = if (answer) answerSource else null,
                            note = note,
                            requestId = target.requestId,
                        )
                        val uploaded = store.call(ExamAction.Upload(draft)) is ExamAnswer.Uploaded
                        sending = false
                        if (uploaded) {
                            store.notice = t("examShare.uploaded")
                            dismiss()
                            done()
                        }
                    }
                }) {
                    if (sending) CircularProgressIndicator(Modifier.padding(end = 8.dp).then(Modifier), strokeWidth = 2.dp, color = Theme.colors.accentInk)
                    Text(t("examShare.upload"))
                }
            }
        }
    }
}

private fun isPhotoUri(context: Context, uri: Uri): Boolean =
    context.contentResolver.getType(uri)?.startsWith("image/") == true

/** A labelled single choice as wrapping chips. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Choices(label: String, options: List<Pair<String, String>>, selected: String, pick: (String) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(label, style = MaterialTheme.typography.labelLarge, color = Theme.colors.muted)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            options.forEach { (value, text) ->
                FilterChip(selected = selected == value, onClick = { pick(value) }, label = { Text(text) })
            }
        }
    }
}

/** Asks for documents of one subject; every field but the subject is optional. */
@Composable
internal fun RequestDialog(subject: ExamSubjectPage, dismiss: () -> Unit, submit: (ExamRequestDraft) -> Unit) {
    val store = LocalExamStore.current
    var classId by remember { mutableStateOf("") }
    var year by remember { mutableStateOf("") }
    var occasion by remember { mutableStateOf("") }
    var role by remember { mutableStateOf("") }
    var description by remember { mutableStateOf("") }
    val thisYear = LocalDate.now().year
    AlertDialog(
        onDismissRequest = dismiss,
        title = { Text(t("examShare.newRequest")) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(store.subjectName(subject.subject), fontWeight = FontWeight.SemiBold)
                if (subject.rows.size > 1) Choices(
                    t("examShare.class"),
                    listOf("" to t("examShare.allClasses")) + subject.rows.mapNotNull { row ->
                        row.classIds.firstOrNull()?.let { id ->
                            id.toString() to (row.labels + row.teachers.map { it.name } + row.taughtIn).filter { it.isNotBlank() }.joinToString(" · ")
                        }
                    },
                    classId,
                ) { classId = it }
                Choices(t("examShare.year"), listOf("" to t("examShare.anyYear")) + (0 until YEARS_BACK).map { "${thisYear - it}" to "${thisYear - it}" }, year) { year = it }
                Choices(t("examShare.occasion"), listOf("" to t("examShare.allOccasions")) + occasions.map { it to t("examShare.occasion.$it") }, occasion) { occasion = it }
                Choices(t("examShare.fileType"), listOf("" to t("examShare.anyRole"), "question" to t("examShare.role.question"), "answer" to t("examShare.role.answer")), role) { role = it }
                OutlinedTextField(description, { description = it.take(1000) }, label = { Text(t("examShare.description")) }, minLines = 3, modifier = Modifier.fillMaxWidth())
                store.problem?.let { Text(it, color = Theme.colors.danger) }
            }
        },
        confirmButton = {
            TextButton(enabled = !store.busy, onClick = {
                submit(ExamRequestDraft(subject.subject.code, classId.toUIntOrNull(), year.toUIntOrNull(), occasion.ifEmpty { null }, role.ifEmpty { null }, description.trim()))
            }) { Text(t("examShare.sendRequest")) }
        },
        dismissButton = { TextButton(onClick = dismiss) { Text(t("common.cancel")) } },
    )
}

@Composable
internal fun ReportDialog(dismiss: () -> Unit, submit: (ExamReportReason, String) -> Unit) {
    val store = LocalExamStore.current
    val reasons = listOf(
        ExamReportReason.COPYRIGHT to "copyright", ExamReportReason.INAPPROPRIATE to "inappropriate",
        ExamReportReason.WRONG_ANSWER to "wrongAnswer", ExamReportReason.BROKEN_FILE to "brokenFile", ExamReportReason.OTHER to "other",
    )
    var reason by remember { mutableStateOf(reasons.first().first) }
    var detail by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = dismiss,
        title = { Text(t("examShare.report")) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Choices(t("examShare.reportReason"), reasons.map { it.second to t("examShare.reason.${it.second}") }, reasons.first { it.first == reason }.second) { key ->
                    reason = reasons.first { it.second == key }.first
                }
                OutlinedTextField(detail, { detail = it.take(900) }, label = { Text(t("examShare.description")) }, minLines = 3, modifier = Modifier.fillMaxWidth())
                store.problem?.let { Text(it, color = Theme.colors.danger) }
            }
        },
        confirmButton = {
            TextButton(enabled = !store.busy, onClick = { submit(reason, detail.trim()) }) {
                Text(t("examShare.sendReport"), color = Theme.colors.danger)
            }
        },
        dismissButton = { TextButton(onClick = dismiss) { Text(t("common.cancel")) } },
    )
}
