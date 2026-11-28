package app.isctcompanion.features.settings

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import app.isctcompanion.LocalModel
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.FinderDepartment
import app.isctcompanion.core.StudyEnrollment
import app.isctcompanion.core.StudyEntryKind
import app.isctcompanion.core.StudyStage
import app.isctcompanion.design.Card
import app.isctcompanion.design.Format
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import kotlinx.coroutines.launch

/** 学習プロフィール: department, admission year and degree programme, the
 * desktop's Study settings. The department is `settings.departmentId`; the
 * year and programme go through Core's `setStudyProfile`, as on desktop. Each
 * choice saves at once; a programme picked before the year waits for it. */
@Composable
internal fun StudyProfilePage() {
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    var departments by remember { mutableStateOf<List<FinderDepartment>>(emptyList()) }
    var stored by remember { mutableStateOf<AppModel.StudyProfile?>(null) }
    var year by remember { mutableStateOf<Int?>(null) }
    var stage by remember { mutableStateOf(StudyStage.UNKNOWN) }
    var currentYear by remember { mutableStateOf<Int?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    suspend fun load() {
        val profile = model.studyProfile() ?: return
        stored = profile
        year = profile.admissionYear
        stage = profile.enrollments.lastOrNull()?.stage ?: StudyStage.UNKNOWN
    }
    LaunchedEffect(Unit) {
        departments = model.departments()
        currentYear = model.academicYear(Format.today())
    }
    LaunchedEffect(model.contentRevision) { load() }

    // Core stores only ids of its department table, so the stored id is a
    // picker value as it is.
    val departmentId = model.records.settings.optString("departmentId")
    val departmentOptions = listOf("" to t("settings.studyProfile.notSet")) +
        departments.map { it.id to it.name }
    // The desktop's choices: the ten years up to next year, plus a stored one.
    val now = currentYear ?: Format.today().year
    val years = ((0 until 10).map { now + 1 - it } + listOfNotNull(year)).distinct().sortedDescending()
    val stages = listOf(
        StudyStage.UNKNOWN to t("settings.studyProfile.notSet"), StudyStage.UNDERGRADUATE to t("settings.studyProfile.undergraduate"), StudyStage.MASTER to t("settings.studyProfile.master"),
        StudyStage.PROFESSIONAL to t("settings.studyProfile.professionalDegree"), StudyStage.DOCTORAL to t("settings.studyProfile.doctoral"),
    )

    /** Builds the profile the desktop's save would: the programme replaces the
     * latest enrollment's stage, and enrollments without a term start in the
     * admission year's 1Q. */
    fun choose(chosenYear: Int?, chosenStage: StudyStage) {
        year = chosenYear
        stage = chosenStage
        error = null
        val enrollments = stored?.enrollments.orEmpty().toMutableList()
        if (enrollments.isEmpty()) {
            if (chosenStage != StudyStage.UNKNOWN) enrollments.add(StudyEnrollment(chosenStage, StudyEntryKind.UNKNOWN, null, ""))
        } else {
            enrollments[enrollments.lastIndex] = enrollments.last().copy(stage = chosenStage)
        }
        if (enrollments.any { it.admittedTerm.isEmpty() }) {
            if (chosenYear == null) {
                error = t("settings.studyProfile.chooseAnAdmissionYear")
                return
            }
            enrollments.replaceAll { if (it.admittedTerm.isEmpty()) it.copy(admittedTerm = "$chosenYear-1Q") else it }
        }
        enrollments.sortBy { it.admittedTerm }
        val profile = AppModel.StudyProfile(chosenYear, enrollments)
        scope.launch {
            try {
                model.setStudyProfile(profile)
                stored = profile
            } catch (failure: Exception) {
                error = AppModel.describe(failure)
                load()
            }
        }
    }

    Page {
        Card {
            PickerRow(t("settings.main.department"), departmentId, departmentOptions) { id ->
                scope.launch { model.updateSettings { if (id.isEmpty()) it.remove("departmentId") else it.put("departmentId", id) } }
            }
            RowDivider(inset = Theme.rowX)
            PickerRow(
                t("settings.main.admissionYear"), year,
                listOf<Pair<Int?, String>>(null to t("settings.studyProfile.notSet")) + years.map { it to t("settings.main.enteredYear", "year" to it) },
            ) { choose(it, stage) }
            RowDivider(inset = Theme.rowX)
            PickerRow(t("settings.main.degreeStage"), stage, stages) { choose(year, it) }
        }
        error?.let { Footer(it, Theme.colors.danger) }
            ?: Footer(t("settings.main.usedForCourseSuggestionsAnd"))
    }
}
