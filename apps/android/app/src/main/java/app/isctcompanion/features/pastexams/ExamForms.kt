package app.isctcompanion.features.pastexams

import android.app.Activity
import android.content.Context
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.Uri
import android.text.format.Formatter
import androidx.activity.compose.LocalActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.IntentSenderRequest
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.ArrowDropDown
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.DocumentScanner
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.Image
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.KeyboardArrowUp
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SheetValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.isctcompanion.IsctApplication
import app.isctcompanion.LocalModel
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.ExamAction
import app.isctcompanion.core.ExamAnswer
import app.isctcompanion.core.ExamChoices
import app.isctcompanion.core.ExamNewTeacher
import app.isctcompanion.core.ExamOccasion
import app.isctcompanion.core.ExamReportReason
import app.isctcompanion.core.ExamRequestDraft
import app.isctcompanion.core.ExamRole
import app.isctcompanion.core.ExamSubject
import app.isctcompanion.core.ExamSubjectPage
import app.isctcompanion.core.ExamUploadDraft
import app.isctcompanion.core.ExamUploadEdit
import app.isctcompanion.core.ExamUploadFile
import app.isctcompanion.core.ExamUploadForm
import app.isctcompanion.core.ExamUploadPhase
import app.isctcompanion.core.ExamUploadStatus
import app.isctcompanion.core.SdkException
import app.isctcompanion.core.SharedInbox
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import com.google.android.gms.common.ConnectionResult
import com.google.android.gms.common.GoogleApiAvailability
import com.google.mlkit.vision.documentscanner.GmsDocumentScannerOptions
import com.google.mlkit.vision.documentscanner.GmsDocumentScanning
import com.google.mlkit.vision.documentscanner.GmsDocumentScanningResult
import java.io.File
import java.io.IOException
import java.util.UUID
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlin.math.roundToInt

private val acceptedDocuments = arrayOf("application/pdf", "text/plain", "image/jpeg", "image/png", "image/webp", "image/heic", "image/heif")
private val photoExtensions = setOf("jpg", "jpeg", "png", "webp")
private const val maxSourceBytes = 25 * 1024 * 1024
/** Year and number chips shown before the rest fold into a menu. */
private const val YEARS_SHOWN = 8
private const val NUMBERS_SHOWN = 10

/** A picked file larger than the site takes. */
private class TooLarge : IOException()

private val ExamUploadFile.isPhoto: Boolean get() = File(path).extension.lowercase() in photoExtensions

/** Recent years as chips, older ones (down to Core's earliest) in the menu. */
private fun years(choices: ExamChoices): Pair<List<UInt>, List<UInt>> {
    val all = (choices.limits.latestYear.toInt() downTo choices.limits.earliestYear.toInt()).map { it.toUInt() }
    return all.take(YEARS_SHOWN) to all.drop(YEARS_SHOWN)
}

/** The upload's phase as one muted line, with its progress while it runs. */
@Composable
internal fun ExamUploadProgress(status: ExamUploadStatus?, sending: Boolean) {
    val phase = status?.phase
    val label = when (phase) {
        ExamUploadPhase.PREPARING -> t("examShare.upload.preparing")
        ExamUploadPhase.TRANSFERRING -> t("examShare.upload.transferring")
        ExamUploadPhase.REGISTERING -> t("examShare.upload.registering")
        ExamUploadPhase.CLEANING -> t("examShare.upload.cleaning")
        ExamUploadPhase.PENDING -> t("examShare.upload.pending")
        else -> if (sending) t("examShare.upload.preparing") else return
    }
    Column(verticalArrangement = Arrangement.spacedBy(5.dp)) {
        Text(label, color = Theme.colors.muted, style = MaterialTheme.typography.bodySmall)
        if (sending) {
            if (phase == ExamUploadPhase.TRANSFERRING && status.total > 0u) {
                val progress = (status.sent.toFloat() / status.total.toFloat()).coerceIn(0f, 1f)
                LinearProgressIndicator(progress = { progress }, modifier = Modifier.fillMaxWidth())
            } else {
                LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
            }
        }
    }
}

/**
 * Shares a past exam. Pages come from ML Kit's document scanner, the photo
 * picker or Files; Core's form decides the subjects, teachers and kinds, and
 * Core checks the site's rules again and turns photos into one PDF. The
 * upload runs in the process's scope, so it outlives the sheet.
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
internal fun UploadSheet(
    form: ExamUploadForm,
    choices: ExamChoices,
    dismiss: () -> Unit,
    /** The upload finished, with the new document's id when the site named it. */
    uploaded: (String?) -> Unit,
) {
    val model = LocalModel.current
    val context = LocalContext.current
    val app = context.applicationContext as IsctApplication
    val scope = rememberCoroutineScope()
    val staging = remember { File(app.cacheDir, "exam-upload/${UUID.randomUUID()}") }
    var files by remember { mutableStateOf(emptyList<ExamUploadFile>()) }
    var current by remember(form) { mutableStateOf(form) }
    val draft = current.draft
    var familyName by remember { mutableStateOf("") }
    var givenName by remember { mutableStateOf("") }
    var addingTeacher by remember { mutableStateOf(false) }
    var query by remember { mutableStateOf("") }
    var hits by remember { mutableStateOf<List<ExamSubject>?>(null) }
    var loading by remember { mutableStateOf(false) }
    var picking by remember { mutableStateOf(false) }
    var sending by remember { mutableStateOf(false) }
    var disposed by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    // Core's `exam-account` events carry the upload's progress.
    val progress = model.examAccount?.upload
    val busy = loading || picking || sending
    val pending = progress?.phase == ExamUploadPhase.PENDING
    val canEdit = !busy && !pending
    val limits = choices.limits
    val role = choices.roles.first { it.role == draft.role }
    val numbered = draft.occasion in choices.numbered
    val mixed = files.size > 1 && !files.all { it.isPhoto }
    val held by rememberUpdatedState(busy)

    /** A plain field: the draft changes here. */
    fun edit(update: (ExamUploadDraft) -> ExamUploadDraft) {
        current = current.copy(draft = update(current.draft))
    }

    DisposableEffect(staging) {
        onDispose {
            disposed = true
            // The process-owned upload still owns the files if the sheet
            // leaves before Core finishes reading them.
            if (!sending) app.scope.launch(Dispatchers.IO) { staging.deleteRecursively() }
        }
    }

    /** Copies [uris] in; Files replaces the selection, photos and scanned
     * pages add to it. */
    fun stage(uris: List<Uri>, replace: Boolean) {
        if (uris.isEmpty()) return
        scope.launch {
            picking = true
            error = null
            val selection = File(staging, UUID.randomUUID().toString())
            try {
                val next = withContext(Dispatchers.IO) { stageFiles(app, selection, uris) }
                if (replace) {
                    files.mapNotNull { File(it.path).parentFile }.distinct().forEach { it.deleteRecursively() }
                    files = next
                } else {
                    files = files + next
                }
            } catch (cancelled: CancellationException) {
                selection.deleteRecursively()
                throw cancelled
            } catch (_: TooLarge) {
                selection.deleteRecursively()
                error = t("error.exam-share-upload-size")
            } catch (_: Exception) {
                selection.deleteRecursively()
                error = t("examShare.upload.failedFiles")
            } finally {
                picking = false
            }
        }
    }
    val documents = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { stage(it, replace = true) }
    val photos = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia()) { stage(it, replace = false) }
    val activity = LocalActivity.current
    // ML Kit's scanner runs in Google Play services; without them it has no button.
    val scans = remember(activity) {
        activity != null && GoogleApiAvailability.getInstance().isGooglePlayServicesAvailable(activity) == ConnectionResult.SUCCESS
    }
    val scanner = rememberLauncherForActivityResult(ActivityResultContracts.StartIntentSenderForResult()) { result ->
        if (result.resultCode != Activity.RESULT_OK) return@rememberLauncherForActivityResult
        val pages = GmsDocumentScanningResult.fromActivityResultIntent(result.data)?.pages.orEmpty()
        stage(pages.map { it.imageUri }, replace = false)
    }
    fun scan() {
        val host = activity ?: return
        val options = GmsDocumentScannerOptions.Builder()
            .setScannerMode(GmsDocumentScannerOptions.SCANNER_MODE_FULL)
            .setResultFormats(GmsDocumentScannerOptions.RESULT_FORMAT_JPEG)
            .setGalleryImportAllowed(false)
            .build()
        GmsDocumentScanning.getClient(options).getStartScanIntent(host)
            .addOnSuccessListener { scanner.launch(IntentSenderRequest.Builder(it).build()) }
            .addOnFailureListener { error = AppModel.describe(it) }
    }

    fun move(index: Int, offset: Int) {
        files = files.toMutableList().apply { add(index + offset, removeAt(index)) }
    }

    /** An edit with rules: Core answers the form it leads to. */
    fun change(step: ExamUploadEdit, then: () -> Unit = {}) {
        scope.launch {
            loading = true
            error = null
            try {
                current = (model.examShare(ExamAction.EditUpload(current, step)) as ExamAnswer.UploadForm).v1
                then()
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (failure: Exception) {
                error = AppModel.describe(failure)
            } finally {
                loading = false
            }
        }
    }

    // Subjects to add, as the browse search finds them.
    LaunchedEffect(query) {
        val trimmed = query.trim()
        hits = null
        if (trimmed.isEmpty()) return@LaunchedEffect
        delay(300)
        hits = try {
            (model.examShare(ExamAction.Search(trimmed)) as ExamAnswer.Subjects).v1
                .filterNot { it.code in current.draft.subjectCodes }
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (failure: Exception) {
            error = AppModel.describe(failure)
            null
        }
    }

    fun send(action: ExamAction) {
        sending = true
        error = null
        app.scope.launch {
            try {
                val answer = model.examShare(action) as? ExamAnswer.Uploaded
                if (answer != null) {
                    withContext(Dispatchers.IO) { staging.deleteRecursively() }
                    if (!disposed) uploaded(answer.v1)
                }
            } catch (_: SdkException.Cancelled) {
                // A deliberate transfer cancellation leaves the form available.
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (failure: Exception) {
                if (!disposed) error = AppModel.describe(failure)
            } finally {
                sending = false
                if (disposed) withContext(Dispatchers.IO) { staging.deleteRecursively() }
            }
        }
    }

    val ready = canEdit && files.isNotEmpty() && !mixed && familyName.isBlank() && givenName.isBlank()

    ModalBottomSheet(
        onDismissRequest = { if (!held) dismiss() },
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true, confirmValueChange = { it != SheetValue.Hidden || !held }),
    ) {
        Column(
            Modifier.fillMaxWidth().imePadding().verticalScroll(rememberScrollState()).padding(horizontal = Theme.gutter).padding(bottom = Theme.space10),
            verticalArrangement = Arrangement.spacedBy(Theme.space6),
        ) {
            Text(
                t(if (current.request == null) "examShare.upload.title" else "examShare.upload.answerRequest"),
                style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold,
            )

            // Files, in the order their pages are sent.
            if (files.size > 1) SectionLabel(t("examShare.upload.fileOrder"))
            files.forEachIndexed { index, picked ->
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Theme.space4)) {
                    Icon(if (picked.isPhoto) Icons.Rounded.Image else Icons.Rounded.Description, null, tint = Theme.colors.muted)
                    Text(picked.name, Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(Formatter.formatShortFileSize(context, File(picked.path).length()), color = Theme.colors.muted, style = MaterialTheme.typography.bodySmall)
                    if (files.size > 1) {
                        IconButton(enabled = canEdit && index > 0, onClick = { move(index, -1) }) {
                            Icon(Icons.Rounded.KeyboardArrowUp, t("examShare.upload.moveUp"))
                        }
                        IconButton(enabled = canEdit && index < files.lastIndex, onClick = { move(index, 1) }) {
                            Icon(Icons.Rounded.KeyboardArrowDown, t("examShare.upload.moveDown"))
                        }
                    }
                    IconButton(enabled = canEdit, onClick = {
                        File(picked.path).delete()
                        files = files - picked
                    }) { Icon(Icons.Rounded.Close, t("common.remove")) }
                }
            }
            FlowRow(horizontalArrangement = Arrangement.spacedBy(Theme.space4), verticalArrangement = Arrangement.spacedBy(Theme.space4)) {
                if (scans) OutlinedButton(enabled = canEdit, onClick = ::scan) {
                    Icon(Icons.Rounded.DocumentScanner, null); Text(" " + t("examShare.upload.scan"))
                }
                OutlinedButton(enabled = canEdit, onClick = {
                    photos.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
                }) { Icon(Icons.Rounded.Image, null); Text(" " + t("examShare.upload.photoLibrary")) }
                OutlinedButton(enabled = canEdit, onClick = { documents.launch(acceptedDocuments) }) {
                    Icon(Icons.Rounded.Folder, null); Text(" " + t("examShare.upload.chooseFiles"))
                }
            }
            Text(
                t(if (mixed) "error.exam-share-upload-mixed" else "examShare.upload.fileSelection"),
                color = if (mixed) Theme.colors.danger else Theme.colors.muted,
                style = MaterialTheme.typography.bodySmall,
            )

            // Subjects: the course's own, and any others the material covers.
            Column(verticalArrangement = Arrangement.spacedBy(Theme.space2)) {
                ChoiceLabel(t("examShare.upload.subjects"))
                FlowRow(horizontalArrangement = Arrangement.spacedBy(Theme.space3)) {
                    current.subjects.forEach { entry ->
                        FilterChip(
                            selected = true,
                            enabled = canEdit && entry.removable,
                            onClick = { change(ExamUploadEdit.RemoveSubject(entry.subject.code)) },
                            label = { Text(entry.subject.name) },
                            trailingIcon = if (entry.removable) ({ Icon(Icons.Rounded.Close, t("common.remove"), Modifier.size(FilterChipDefaults.IconSize)) }) else null,
                        )
                    }
                }
                if (current.subjects.size.toUInt() < limits.subjects) {
                    OutlinedTextField(
                        query, { query = it.take(limits.query.toInt()) },
                        enabled = canEdit,
                        placeholder = { Text(t("examShare.upload.addSubject")) },
                        leadingIcon = { Icon(Icons.Rounded.Search, null) },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    hits?.let { found ->
                        if (found.isEmpty()) Text(t("examShare.noItems"), color = Theme.colors.muted, style = MaterialTheme.typography.bodySmall)
                        FlowRow(horizontalArrangement = Arrangement.spacedBy(Theme.space3)) {
                            found.forEach { hit ->
                                FilterChip(
                                    selected = false,
                                    enabled = canEdit,
                                    onClick = { change(ExamUploadEdit.AddSubject(hit.code)) { query = "" } },
                                    label = { Text("${hit.name} · ${hit.code}") },
                                    leadingIcon = { Icon(Icons.Rounded.Add, null, Modifier.size(FilterChipDefaults.IconSize)) },
                                )
                            }
                        }
                    }
                }
            }

            // Teachers: those the site lists, and any it does not yet.
            Column(verticalArrangement = Arrangement.spacedBy(Theme.space2)) {
                ChoiceLabel(t("examShare.teacher"))
                FlowRow(horizontalArrangement = Arrangement.spacedBy(Theme.space3)) {
                    current.teachers.forEach { teacher ->
                        val checked = teacher.id in draft.teacherIds
                        FilterChip(
                            selected = checked,
                            enabled = canEdit && (checked || draft.teacherIds.size.toUInt() < limits.teachers),
                            onClick = { edit { it.copy(teacherIds = if (checked) it.teacherIds - teacher.id else it.teacherIds + teacher.id) } },
                            label = { Text(teacher.name) },
                        )
                    }
                    draft.newTeachers.forEachIndexed { index, teacher ->
                        FilterChip(
                            selected = true,
                            enabled = canEdit,
                            onClick = { edit { it.copy(newTeachers = it.newTeachers.toMutableList().apply { removeAt(index) }) } },
                            label = { Text(listOf(teacher.familyName, teacher.givenName).filter { it.isNotEmpty() }.joinToString(" ")) },
                            trailingIcon = { Icon(Icons.Rounded.Close, t("common.remove"), Modifier.size(FilterChipDefaults.IconSize)) },
                        )
                    }
                    if (draft.newTeachers.size.toUInt() < limits.teachers) FilterChip(
                        selected = addingTeacher,
                        enabled = canEdit,
                        onClick = { addingTeacher = !addingTeacher },
                        label = { Text(t("examShare.upload.newTeacher")) },
                        leadingIcon = { Icon(Icons.Rounded.Add, null, Modifier.size(FilterChipDefaults.IconSize)) },
                    )
                }
                if (addingTeacher) Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Theme.space3)) {
                    OutlinedTextField(familyName, { familyName = it.take(limits.teacherName.toInt()) }, enabled = canEdit, singleLine = true,
                        label = { Text(t("examShare.upload.familyName")) }, modifier = Modifier.weight(1f))
                    OutlinedTextField(givenName, { givenName = it.take(limits.teacherName.toInt()) }, enabled = canEdit, singleLine = true,
                        label = { Text(t("examShare.upload.givenName")) }, modifier = Modifier.weight(1f))
                    TextButton(enabled = canEdit && familyName.isNotBlank(), onClick = {
                        edit { it.copy(newTeachers = it.newTeachers + ExamNewTeacher(familyName.trim(), givenName.trim())) }
                        familyName = ""
                        givenName = ""
                        addingTeacher = false
                    }) { Text(t("common.add")) }
                }
            }

            Choices(t("examShare.upload.role"), choices.roles.map { it.role to t("examShare.role.${it.role.wire}") }, draft.role, canEdit) {
                change(ExamUploadEdit.Role(it))
            }
            val (recent, older) = years(choices)
            Choices(
                t("examShare.year"),
                recent.map<UInt, Pair<UInt?, String>> { it to "$it" } + (null to t("examShare.upload.unknown")),
                draft.academicYear, canEdit,
                more = older.map { it to "$it" },
            ) { picked -> edit { it.copy(academicYear = picked) } }
            // A role with one occasion (notes, other) has nothing to pick.
            if (role.occasions.size != 1) {
                Choices(t("examShare.occasion"), role.occasions.map { it to t("examShare.occasion.${it.wire}") }, draft.occasion, canEdit) { picked ->
                    edit {
                        it.copy(
                            occasion = picked,
                            occasionNumber = when {
                                picked !in choices.numbered -> 0u
                                it.occasionNumber == 0u -> 1u
                                else -> it.occasionNumber
                            },
                        )
                    }
                }
            }
            if (numbered) {
                val numbers = (1u..limits.occasionNumber).toList()
                Choices(
                    t("examShare.upload.occasionNumber"),
                    numbers.take(NUMBERS_SHOWN).map { it to "$it" }, draft.occasionNumber, canEdit,
                    more = numbers.drop(NUMBERS_SHOWN).map { it to "$it" },
                ) { picked -> edit { it.copy(occasionNumber = picked) } }
            }
            Choices(t("examShare.upload.provenance"), choices.provenances.map { it to t("examShare.upload.${it.wire}") }, draft.provenance, canEdit) { picked ->
                edit { it.copy(provenance = picked) }
            }
            if (role.answerSources.isNotEmpty()) {
                Choices(t("examShare.upload.answerSource"), role.answerSources.map { it to t("examShare.upload.${it.wire}") }, draft.answerSource, canEdit) { picked ->
                    edit { it.copy(answerSource = picked) }
                }
            }
            OutlinedTextField(draft.variant, { value -> edit { it.copy(variant = value.take(limits.variant.toInt())) } }, enabled = canEdit,
                label = { Text(t("examShare.upload.variant")) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(draft.note, { value -> edit { it.copy(note = value.take(limits.note.toInt())) } }, enabled = canEdit,
                label = { Text(t("examShare.upload.note")) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            Text(t("examShare.uploadPublic"), color = Theme.colors.muted, style = MaterialTheme.typography.bodySmall)
            error?.let { Text(it, color = Theme.colors.danger) }
            when {
                sending -> ExamUploadProgress(progress, true)
                picking -> ExamUploadProgress(null, true)
                pending -> ExamUploadProgress(progress, false)
            }

            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(Theme.space4, Alignment.End)) {
                if (sending && progress?.cancellable == true) TextButton(onClick = {
                    scope.launch {
                        try {
                            model.examShare(ExamAction.CancelUpload)
                        } catch (cancelled: CancellationException) {
                            throw cancelled
                        } catch (failure: Exception) {
                            error = AppModel.describe(failure)
                        }
                    }
                }) { Text(t("common.cancel")) }
                else TextButton(onClick = dismiss, enabled = !busy) { Text(t("common.cancel")) }
                if (pending && !sending) {
                    Button(enabled = !busy, onClick = { send(ExamAction.RecoverUpload) }) { Text(t("examShare.upload.recover")) }
                } else {
                    Button(enabled = ready, onClick = { send(ExamAction.Upload(files, draft)) }) {
                        if (sending) CircularProgressIndicator(Modifier.padding(end = Theme.space4).size(16.dp), strokeWidth = 2.dp, color = Theme.colors.accentInk)
                        Text(t("examShare.upload.title"))
                    }
                }
            }
        }
    }
}

@Composable
private fun ChoiceLabel(label: String) {
    Text(label, style = MaterialTheme.typography.labelLarge, color = Theme.colors.muted)
}

/** A labelled single choice as wrapping chips; [more] folds the rest into
 * one chip's menu. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun <T> Choices(
    label: String?,
    options: List<Pair<T, String>>,
    selected: T,
    enabled: Boolean = true,
    more: List<Pair<T, String>> = emptyList(),
    pick: (T) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(Theme.space2)) {
        label?.let { ChoiceLabel(it) }
        FlowRow(horizontalArrangement = Arrangement.spacedBy(Theme.space3)) {
            options.forEach { (value, text) ->
                FilterChip(selected = selected == value, enabled = enabled, onClick = { pick(value) }, label = { Text(text) })
            }
            if (more.isNotEmpty()) Box {
                var open by remember { mutableStateOf(false) }
                val chosen = more.firstOrNull { it.first == selected }
                FilterChip(
                    selected = chosen != null,
                    enabled = enabled,
                    onClick = { open = true },
                    label = { Text(chosen?.second ?: t("examShare.more")) },
                    trailingIcon = { Icon(Icons.Rounded.ArrowDropDown, null, Modifier.size(FilterChipDefaults.IconSize)) },
                )
                DropdownMenu(open, { open = false }) {
                    more.forEach { (value, text) ->
                        DropdownMenuItem({ Text(text) }, { open = false; pick(value) })
                    }
                }
            }
        }
    }
}

/** Asks for documents of one subject; every field but the subject is optional. */
@Composable
internal fun RequestDialog(
    subject: ExamSubjectPage,
    choices: ExamChoices,
    busy: Boolean,
    error: String?,
    dismiss: () -> Unit,
    submit: (ExamRequestDraft) -> Unit,
) {
    var classId by remember { mutableStateOf<UInt?>(null) }
    var year by remember { mutableStateOf<UInt?>(null) }
    var occasion by remember { mutableStateOf<ExamOccasion?>(null) }
    var role by remember { mutableStateOf<ExamRole?>(null) }
    var description by remember { mutableStateOf("") }
    val (recent, older) = years(choices)
    AlertDialog(
        onDismissRequest = dismiss,
        title = { Text(t("examShare.newRequest")) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(Theme.space5)) {
                Text(subject.subject.name, fontWeight = FontWeight.SemiBold)
                if (subject.rows.size > 1) Choices(
                    t("examShare.class"),
                    listOf<Pair<UInt?, String>>(null to t("examShare.allClasses")) + subject.rows.mapNotNull { row ->
                        row.classIds.firstOrNull()?.let { id ->
                            id to (row.labels + row.teachers.map { it.name } + row.taughtIn).filter { it.isNotBlank() }.joinToString(" · ")
                        }
                    },
                    classId,
                ) { classId = it }
                Choices(
                    t("examShare.year"),
                    listOf<Pair<UInt?, String>>(null to t("examShare.allYears")) + recent.map { it to "$it" },
                    year,
                    more = older.map { it to "$it" },
                ) { year = it }
                Choices(
                    t("examShare.occasion"),
                    listOf<Pair<ExamOccasion?, String>>(null to t("examShare.allOccasions")) + choices.requestOccasions.map { it to t("examShare.occasion.${it.wire}") },
                    occasion,
                ) { occasion = it }
                Choices(
                    t("examShare.upload.role"),
                    listOf<Pair<ExamRole?, String>>(null to t("examShare.wantedRole")) + choices.requestRoles.map { it to t("examShare.role.${it.wire}") },
                    role,
                ) { role = it }
                OutlinedTextField(
                    description, { description = it.take(choices.limits.requestDescription.toInt()) },
                    label = { Text(t("examShare.description")) }, minLines = 3, modifier = Modifier.fillMaxWidth(),
                )
                error?.let { Text(it, color = Theme.colors.danger) }
            }
        },
        confirmButton = {
            TextButton(enabled = !busy, onClick = {
                submit(ExamRequestDraft(subject.subject.code, classId, year, occasion, role, description.trim()))
            }) { Text(t("examShare.newRequest")) }
        },
        dismissButton = { TextButton(onClick = dismiss) { Text(t("common.cancel")) } },
    )
}

@Composable
internal fun ReportDialog(
    reasons: List<ExamReportReason>,
    detailLimit: Int,
    busy: Boolean,
    error: String?,
    dismiss: () -> Unit,
    submit: (ExamReportReason, String) -> Unit,
) {
    var reason by remember { mutableStateOf(reasons.first()) }
    var detail by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = dismiss,
        title = { Text(t("examShare.report")) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(Theme.space4)) {
                Choices(null, reasons.map { it to t("examShare.reason.${it.wire}") }, reason) { reason = it }
                OutlinedTextField(
                    detail, { detail = it.take(detailLimit) },
                    label = { Text(t("examShare.description")) }, minLines = 3, modifier = Modifier.fillMaxWidth(),
                )
                error?.let { Text(it, color = Theme.colors.danger) }
            }
        },
        confirmButton = {
            TextButton(enabled = !busy, onClick = { submit(reason, detail.trim()) }) {
                Text(t("examShare.report"), color = Theme.colors.danger)
            }
        },
        dismissButton = { TextButton(onClick = dismiss) { Text(t("common.cancel")) } },
    )
}

/** Copy while the document picker URI grants are live. Android's decoder
 * supplies HEIF support; Core handles the common image/PDF preparation. */
private suspend fun stageFiles(context: Context, folder: File, uris: List<Uri>): List<ExamUploadFile> {
    if (!folder.mkdirs()) throw IOException()
    val resolver = context.contentResolver
    return uris.map { uri ->
        currentCoroutineContext().ensureActive()
        val type = resolver.getType(uri).orEmpty()
        val name = SharedInbox.displayName(context, uri)
        val extension = name.substringAfterLast('.', "").lowercase()
        val heif = type in setOf("image/heic", "image/heif") || extension in setOf("heic", "heif")
        val suffix = when {
            heif -> ".jpg"
            extension in setOf("pdf", "txt", "jpg", "jpeg", "png", "webp") -> ".$extension"
            else -> ".bin"
        }
        val file = File.createTempFile("source-", suffix, folder)
        if (heif) {
            val bitmap = ImageDecoder.decodeBitmap(ImageDecoder.createSource(resolver, uri)) { decoder, info, _ ->
                decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
                val size = info.size
                val ratio = 4000.0 / maxOf(size.width, size.height)
                if (ratio < 1) decoder.setTargetSize((size.width * ratio).roundToInt().coerceAtLeast(1), (size.height * ratio).roundToInt().coerceAtLeast(1))
            }
            try {
                file.outputStream().use { if (!bitmap.compress(Bitmap.CompressFormat.JPEG, 90, it)) throw IOException() }
            } finally {
                bitmap.recycle()
            }
        } else {
            val input = resolver.openInputStream(uri) ?: throw IOException()
            input.use {
                file.outputStream().use { output ->
                    val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
                    var size = 0
                    while (true) {
                        currentCoroutineContext().ensureActive()
                        val count = input.read(buffer)
                        if (count < 0) break
                        size += count
                        if (size > maxSourceBytes) throw TooLarge()
                        output.write(buffer, 0, count)
                    }
                }
            }
        }
        val uploadName = when {
            heif -> name.substringBeforeLast('.') + ".jpg"
            type == "text/plain" && extension !in setOf("txt", "md", "markdown") -> "$name.txt"
            else -> name
        }
        ExamUploadFile(file.path, uploadName)
    }
}
