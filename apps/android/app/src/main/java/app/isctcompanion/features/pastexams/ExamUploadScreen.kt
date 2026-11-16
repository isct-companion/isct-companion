package app.isctcompanion.features.pastexams

import android.content.Context
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.Uri
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.KeyboardArrowUp
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.isctcompanion.IsctApplication
import app.isctcompanion.LocalModel
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.ExamAction
import app.isctcompanion.core.ExamAnswer
import app.isctcompanion.core.ExamChoices
import app.isctcompanion.core.ExamNewTeacher
import app.isctcompanion.core.ExamSubject
import app.isctcompanion.core.ExamUploadDraft
import app.isctcompanion.core.ExamUploadEdit
import app.isctcompanion.core.ExamUploadFile
import app.isctcompanion.core.ExamUploadForm
import app.isctcompanion.core.ExamUploadPhase
import app.isctcompanion.core.ExamUploadStatus
import app.isctcompanion.core.SdkException
import app.isctcompanion.core.SharedInbox
import app.isctcompanion.design.ActionButton
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.TabPage
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import java.io.File
import java.io.IOException
import java.util.UUID
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlin.math.roundToInt

private val acceptedDocuments = arrayOf("application/pdf", "text/plain", "image/jpeg", "image/png", "image/webp", "image/heic", "image/heif")
private const val maxSourceBytes = 25 * 1024 * 1024

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
        Text(label, color = Theme.colors.muted)
        if (sending) {
            if (phase == ExamUploadPhase.TRANSFERRING && status != null && status.total > 0u) {
                val progress = (status.sent.toFloat() / status.total.toFloat()).coerceIn(0f, 1f)
                LinearProgressIndicator(progress = { progress }, modifier = Modifier.fillMaxWidth())
            } else {
                LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
            }
        }
    }
}

/** Core's upload form: edits with rules (subjects, role) go back to Core,
 * plain fields change [form]'s draft here, and the upload sends that draft. */
@Composable
internal fun ExamUploadScreen(
    form: ExamUploadForm,
    choices: ExamChoices,
    onBack: () -> Unit,
    onUploaded: () -> Unit,
) {
    val model = LocalModel.current
    val context = LocalContext.current.applicationContext as IsctApplication
    val scope = rememberCoroutineScope()
    val staging = remember { File(context.cacheDir, "exam-upload/${UUID.randomUUID()}") }
    var files by remember { mutableStateOf(emptyList<ExamUploadFile>()) }
    var current by remember(form) { mutableStateOf(form) }
    val draft = current.draft
    var year by remember(form) { mutableStateOf(draft.academicYear?.toString().orEmpty()) }
    var number by remember(form) { mutableStateOf(draft.occasionNumber.toString()) }
    var familyName by remember { mutableStateOf("") }
    var givenName by remember { mutableStateOf("") }
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

    /** A plain field: the draft changes here. */
    fun edit(update: (ExamUploadDraft) -> ExamUploadDraft) {
        current = current.copy(draft = update(current.draft))
    }

    DisposableEffect(staging) {
        onDispose {
            disposed = true
            // The process-owned job still owns the files if the screen leaves
            // before Core finishes reading them.
            if (!sending) context.scope.launch(Dispatchers.IO) { staging.deleteRecursively() }
        }
    }
    BackHandler { if (!busy) onBack() }

    val documents = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isEmpty()) return@rememberLauncherForActivityResult
        scope.launch {
            picking = true
            error = null
            val selection = File(staging, UUID.randomUUID().toString())
            try {
                val next = withContext(Dispatchers.IO) { stageFiles(context, selection, uris) }
                files.firstOrNull()?.let { File(it.path).parentFile?.deleteRecursively() }
                files = next
            } catch (cancelled: CancellationException) {
                selection.deleteRecursively()
                throw cancelled
            } catch (_: Exception) {
                selection.deleteRecursively()
                error = t("examShare.upload.failedFiles")
            } finally {
                picking = false
            }
        }
    }

    fun move(index: Int, offset: Int) {
        files = files.toMutableList().apply { add(index + offset, removeAt(index)) }
    }

    /** Runs a form request; a failure shows on the form. */
    fun perform(work: suspend () -> Unit) {
        scope.launch {
            loading = true
            error = null
            try {
                work()
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (failure: Exception) {
                error = AppModel.describe(failure)
            } finally {
                loading = false
            }
        }
    }

    /** An edit with rules: Core answers the form it leads to. */
    fun change(step: ExamUploadEdit, then: () -> Unit = {}) = perform {
        current = (model.examShare(ExamAction.EditUpload(current, step)) as ExamAnswer.UploadForm).v1
        if (current.draft.occasion !in choices.numbered) number = "0"
        then()
    }

    fun send(action: ExamAction) {
        sending = true
        error = null
        context.scope.launch {
            try {
                if (model.examShare(action) == ExamAnswer.Uploaded) {
                    withContext(Dispatchers.IO) { staging.deleteRecursively() }
                    if (!disposed) onUploaded()
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

    TabPage(
        title = t(if (current.request == null) "examShare.upload.title" else "examShare.upload.answerRequest"),
        navigationIcon = {
            IconButton(enabled = !busy, onClick = onBack) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, t("common.back")) }
        },
    ) {
        Column(Modifier.fillMaxSize().imePadding()) {
            LazyColumn(
                Modifier.weight(1f),
                contentPadding = PaddingValues(Theme.gutter),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                item {
                    TextButton(enabled = canEdit, onClick = { documents.launch(acceptedDocuments) }) { Text(t("examShare.upload.chooseFiles")) }
                    if (files.isEmpty()) Text(t("examShare.upload.fileSelection"), color = Theme.colors.muted)
                    if (files.size > 1) SectionLabel(t("examShare.upload.fileOrder"))
                }
                itemsIndexed(files, key = { _, file -> file.path }) { index, file ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(file.name, modifier = Modifier.weight(1f), maxLines = 2, overflow = TextOverflow.Ellipsis)
                        if (files.size > 1) {
                            IconButton(enabled = canEdit && index > 0, onClick = { move(index, -1) }) {
                                Icon(Icons.Rounded.KeyboardArrowUp, t("examShare.upload.moveUp"))
                            }
                            IconButton(enabled = canEdit && index < files.lastIndex, onClick = { move(index, 1) }) {
                                Icon(Icons.Rounded.KeyboardArrowDown, t("examShare.upload.moveDown"))
                            }
                        }
                        IconButton(enabled = canEdit, onClick = {
                            File(file.path).delete()
                            files = files - file
                        }) { Icon(Icons.Rounded.Close, t("common.remove")) }
                    }
                }
                item { SectionLabel(t("examShare.upload.subjects")) }
                items(current.subjects, key = { "subject:${it.subject.code}" }) { entry ->
                    val code = entry.subject.code
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text("${entry.subject.name} · $code", modifier = Modifier.weight(1f))
                        if (entry.removable) {
                            IconButton(enabled = canEdit, onClick = { change(ExamUploadEdit.RemoveSubject(code)) }) {
                                Icon(Icons.Rounded.Close, t("common.remove"))
                            }
                        }
                    }
                }
                if (current.subjects.size.toUInt() < limits.subjects) {
                    item {
                        OutlinedTextField(query, { query = it.take(limits.query.toInt()); hits = null }, enabled = canEdit, singleLine = true,
                            label = { Text(t("examShare.upload.addSubject")) }, modifier = Modifier.fillMaxWidth())
                        TextButton(enabled = canEdit && query.isNotBlank(), onClick = {
                            perform {
                                hits = (model.examShare(ExamAction.Search(query)) as ExamAnswer.Subjects).v1
                                    .filterNot { it.code in draft.subjectCodes }
                            }
                        }) { Text(t("examShare.search")) }
                        if (hits?.isEmpty() == true) Text(t("examShare.noItems"), color = Theme.colors.muted)
                    }
                    items(hits.orEmpty(), key = { "hit:${it.code}" }) { hit ->
                        TextButton(enabled = canEdit, onClick = {
                            change(ExamUploadEdit.AddSubject(hit.code)) {
                                query = ""
                                hits = null
                            }
                        }) {
                            Text("${hit.name} · ${hit.code}")
                        }
                    }
                }
                item { SectionLabel(t("examShare.teacher")) }
                items(current.teachers, key = { "teacher:${it.id}" }) { teacher ->
                    val checked = teacher.id in draft.teacherIds
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Checkbox(checked = checked,
                            enabled = canEdit && (checked || draft.teacherIds.size.toUInt() < limits.teachers),
                            onCheckedChange = { selected ->
                                edit { it.copy(teacherIds = if (selected) it.teacherIds + teacher.id else it.teacherIds - teacher.id) }
                            })
                        Text(teacher.name)
                    }
                }
                items(draft.newTeachers.size) { index ->
                    val teacher = draft.newTeachers[index]
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(listOf(teacher.familyName, teacher.givenName).filter { it.isNotEmpty() }.joinToString(" "), modifier = Modifier.weight(1f))
                        IconButton(enabled = canEdit, onClick = {
                            edit { it.copy(newTeachers = it.newTeachers.toMutableList().apply { removeAt(index) }) }
                        }) { Icon(Icons.Rounded.Close, t("common.remove")) }
                    }
                }
                if (draft.newTeachers.size.toUInt() < limits.teachers) item {
                    SectionLabel(t("examShare.upload.newTeacher"))
                    UploadField("examShare.upload.familyName", familyName, canEdit, limits.teacherName.toInt()) { familyName = it }
                    UploadField("examShare.upload.givenName", givenName, canEdit, limits.teacherName.toInt()) { givenName = it }
                    TextButton(enabled = canEdit && familyName.isNotBlank(), onClick = {
                        edit { it.copy(newTeachers = it.newTeachers + ExamNewTeacher(familyName.trim(), givenName.trim())) }
                        familyName = ""
                        givenName = ""
                    }) { Text(t("common.add")) }
                }
                item {
                    UploadChoice("examShare.upload.role", draft.role, choices.roles.map { it.role to t("examShare.role.${it.role.wire}") }, canEdit) {
                        change(ExamUploadEdit.Role(it))
                    }
                    OutlinedTextField(year, { year = it.filter { digit -> digit in '0'..'9' }.take(4) },
                        enabled = canEdit, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number), singleLine = true,
                        label = { Text(t("examShare.year")) }, modifier = Modifier.fillMaxWidth())
                    // A role with one occasion (notes, other) has nothing to pick.
                    if (role.occasions.size != 1) {
                        UploadChoice("examShare.occasion", draft.occasion, role.occasions.map { it to t("examShare.occasion.${it.wire}") }, canEdit) { picked ->
                            edit { it.copy(occasion = picked) }
                            if (picked !in choices.numbered) number = "0"
                        }
                    }
                    if (numbered) OutlinedTextField(number, { number = it.filter { digit -> digit in '0'..'9' }.take(limits.occasionNumber.toString().length) },
                        enabled = canEdit, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number), singleLine = true,
                        label = { Text(t("examShare.upload.occasionNumber")) }, modifier = Modifier.fillMaxWidth())
                    UploadChoice("examShare.upload.provenance", draft.provenance, choices.provenances.map { it to t("examShare.upload.${it.wire}") }, canEdit) { picked ->
                        edit { it.copy(provenance = picked) }
                    }
                    if (role.answerSources.isNotEmpty()) {
                        UploadChoice("examShare.upload.answerSource", draft.answerSource, role.answerSources.map { it to t("examShare.upload.${it.wire}") }, canEdit) { picked ->
                            edit { it.copy(answerSource = picked) }
                        }
                    }
                    UploadField("examShare.upload.variant", draft.variant, canEdit, limits.variant.toInt()) { value -> edit { it.copy(variant = value) } }
                    UploadField("examShare.upload.note", draft.note, canEdit, limits.note.toInt()) { value -> edit { it.copy(note = value) } }
                }
            }
            Column(
                Modifier.fillMaxWidth().padding(horizontal = Theme.gutter, vertical = 8.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                error?.let { Text(it, color = Theme.colors.danger) }
                when {
                    sending -> {
                        ExamUploadProgress(progress, true)
                        if (progress?.cancellable == true) {
                            TextButton(onClick = {
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
                        }
                    }
                    picking -> ExamUploadProgress(null, true)
                    pending -> {
                        ExamUploadProgress(progress, false)
                        ActionButton(title = t("examShare.upload.recover"), onClick = { send(ExamAction.RecoverUpload) })
                    }
                    else -> ActionButton(
                        title = t("examShare.upload.title"),
                        enabled = canEdit && files.isNotEmpty() && (year.isEmpty() || year.length == 4) &&
                            (!numbered || (number.toUIntOrNull() ?: 0u) <= limits.occasionNumber) && familyName.isBlank() && givenName.isBlank(),
                        onClick = {
                            send(ExamAction.Upload(files, draft.copy(academicYear = year.toUIntOrNull(),
                                occasionNumber = if (numbered) number.toUIntOrNull() ?: 0u else 0u)))
                        },
                    )
                }
            }
        }
    }
}

@Composable
private fun UploadField(field: String, value: String, enabled: Boolean, limit: Int, update: (String) -> Unit) {
    OutlinedTextField(value, { update(it.take(limit)) }, enabled = enabled, singleLine = true,
        label = { Text(t(field)) }, modifier = Modifier.fillMaxWidth())
}

@Composable
private fun <T> UploadChoice(label: String, value: T, choices: List<Pair<T, String>>, enabled: Boolean, pick: (T) -> Unit) {
    var menu by remember { mutableStateOf(false) }
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.SpaceBetween, modifier = Modifier.fillMaxWidth()) {
        Text(t(label))
        Box {
            TextButton(enabled = enabled, onClick = { menu = true }) { Text(choices.first { it.first == value }.second) }
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                choices.forEach { (choice, text) ->
                    DropdownMenuItem(text = { Text(text) }, onClick = { menu = false; pick(choice) })
                }
            }
        }
    }
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
                        if (size > maxSourceBytes) throw IOException()
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
