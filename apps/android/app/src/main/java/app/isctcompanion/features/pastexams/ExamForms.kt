package app.isctcompanion.features.pastexams

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ArrowDropDown
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import app.isctcompanion.core.ExamChoices
import app.isctcompanion.core.ExamOccasion
import app.isctcompanion.core.ExamReportReason
import app.isctcompanion.core.ExamRequestDraft
import app.isctcompanion.core.ExamRole
import app.isctcompanion.core.ExamSubjectPage
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t

/** Year and number chips shown before the rest fold into a menu. */
private const val YEARS_SHOWN = 8

/** Recent years as chips, older ones (down to Core's earliest) in the menu. */
internal fun years(choices: ExamChoices): Pair<List<UInt>, List<UInt>> {
    val all = (choices.limits.latestYear.toInt() downTo choices.limits.earliestYear.toInt()).map { it.toUInt() }
    return all.take(YEARS_SHOWN) to all.drop(YEARS_SHOWN)
}

@Composable
internal fun ChoiceLabel(label: String) {
    Text(label, style = MaterialTheme.typography.labelLarge, color = Theme.colors.muted)
}

/** A labelled single choice as wrapping chips; [more] folds the rest into
 * one chip's menu. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun <T> Choices(
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
