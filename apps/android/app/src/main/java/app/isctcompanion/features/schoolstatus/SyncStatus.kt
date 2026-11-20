package app.isctcompanion.features.schoolstatus

import app.isctcompanion.design.Card
import app.isctcompanion.design.Format
import app.isctcompanion.design.LocalReducedMotion
import app.isctcompanion.design.Motion
import app.isctcompanion.design.RowDivider
import app.isctcompanion.design.SectionLabel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import app.isctcompanion.design.lookup
import androidx.compose.animation.fadeOut
import androidx.compose.animation.fadeIn
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.PauseCircle
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.AccountWait
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.Held
import app.isctcompanion.core.RunView
import app.isctcompanion.core.SchoolAuthProfile
import app.isctcompanion.core.SchoolSessionState
import app.isctcompanion.core.SourceView
import app.isctcompanion.core.SyncFailure
import app.isctcompanion.core.SyncLabel
import app.isctcompanion.core.SyncPriority
import app.isctcompanion.core.RetryPlan
import app.isctcompanion.core.SyncSnapshot
import app.isctcompanion.core.SyncAttentionProblem
import app.isctcompanion.core.SyncWork
import kotlinx.coroutines.delay
import org.json.JSONObject

/* Words for Core's sync snapshot (`plan`). Core decides what runs, what
 * waits, what failed and when it retries; these only name it, with the iOS
 * shell's strings (`Features/SchoolStatus/SchoolStatusViews.swift`). */

/** A source as Core names it in `SourceView.source`. */
fun sourceName(source: String): String = when (source) {
    "lms" -> "LMS"
    "kyomu" -> "教務Web"
    "references" -> t("syncStatus.dataset.referenceData")
    "catalog" -> t("syncStatus.dataset.courseCatalog")
    else -> source
}

/** Dataset names, keyed by the dataset id Core puts in `RunView.dataset` and
 * in its labels' keys. */
private fun datasetName(key: String): String? = when (key) {
    "lms.enrolment" -> t("common.courses")
    "lms.deadlines" -> t("syncStatus.dataset.deadlines")
    "lms.course" -> t("syncStatus.dataset.courseContent")
    "lms.assign" -> t("common.assignments")
    "lms.history" -> t("syncStatus.dataset.pastCourses")
    "kyomu.page" -> t("syncStatus.dataset.kyomuPages")
    "catalog.official" -> t("syncStatus.dataset.courseCatalog")
    "reference" -> t("syncStatus.dataset.referenceData")
    else -> null
}

fun dataset(dataset: String): String = datasetName(dataset) ?: dataset

/** A Core label: a dataset key reads as the dataset's name; any other key is
 * a catalog key filled with Core's arguments. */
fun label(label: SyncLabel): String {
    datasetName(label.key.removePrefix("sync."))?.let { return it }
    val args = runCatching { JSONObject(label.args) }.getOrNull()
    val values = args?.keys()?.asSequence()?.map { it to args.get(it) }?.toList().orEmpty()
    return lookup(label.key, *values.toTypedArray()) ?: label.key
}

fun heldText(held: Held): String = when (held) {
    is Held.Offline -> t("syncStatus.waitingForAConnection")
    is Held.Cellular -> t("syncStatus.waitingForWifi")
    // Android's Battery Saver, not iOS's Low Power Mode.
    is Held.PowerSaver -> t("syncStatus.pausedPowerSaver")
    is Held.Background -> t("syncStatus.resumesWhenTheAppIs")
    is Held.Backoff -> retryAt(held.until)
    is Held.HostPaused -> t("syncStatus.hostIsPausedUntilTime", "host" to held.host, "time" to Format.due(held.until))
    // No credential on the key's path can serve it now; not a failure.
    is Held.Account -> when (val wait = held.wait) {
        is AccountWait.Person -> t("syncStatus.held.person")
        // A pause after a sign-in without a verdict, said as the account row says it.
        is AccountWait.Paused -> t("schoolSession.state.unavailableRetry", "time" to Format.due(wait.until))
    }
    is Held.Resetting -> t("syncStatus.resetting")
    is Held.Prerequisite -> t("syncStatus.waitingForTheDataIt")
}

private fun retryAt(raw: String): String = t("syncStatus.retryAtTime", "time" to Format.due(raw))

/** "Could not update Deadlines · Retry at Today 14:05". */
private fun failureText(failure: SyncFailure, name: String): String =
    listOfNotNull(t("syncStatus.couldNotUpdateSource", "source" to name), label(failure.reason), failure.retryAt?.let(::retryAt)).joinToString(" · ")

/** What the header shows for Core's runs: a ring only when a run has a
 * total, a spinner for other work, an icon when everything waits or failed. */
sealed interface SyncIndicator {
    data class Progress(val fraction: Float, val emphasized: Boolean) : SyncIndicator
    data class Spinner(val emphasized: Boolean) : SyncIndicator
    data object Failed : SyncIndicator
    data object Held : SyncIndicator
}

/** The one thing that needs the person, for 今日's banner. */
data class SyncProblem(val text: String, val retry: RetryPlan?, val signIn: SchoolAuthProfile?)

val SyncSnapshot.showsLoading: Boolean
    get() = attention.primary is SyncWork.Active || attention.primary is SyncWork.Resetting

val SyncSnapshot.indicator: SyncIndicator?
    get() = when (val work = attention.primary) {
        is SyncWork.Resetting -> SyncIndicator.Spinner(true)
        is SyncWork.Held -> SyncIndicator.Held
        is SyncWork.Problem -> SyncIndicator.Failed
        is SyncWork.Active -> runs.firstOrNull { it.run == work.run }?.let { run ->
            val emphasized = run.priority != SyncPriority.BACKGROUND
            val total = run.total?.toLong()
            if (total == null) SyncIndicator.Spinner(emphasized)
            else SyncIndicator.Progress(minOf(run.done.toLong(), total).toFloat() / maxOf(total, 1L), emphasized)
        }
        null -> null
    }

val SyncSnapshot.problem: SyncProblem?
    get() = when (val problem = attention.problem) {
        is SyncAttentionProblem.Account -> SyncProblem(
            problem.presentation?.let { AppModel.signInText(it, problem.retryAt) } ?: t("syncStatus.held.person"),
            null, problem.profile,
        )
        is SyncAttentionProblem.RunFailed -> SyncProblem(failureText(problem.failure, dataset(problem.dataset)), problem.retry, null)
        is SyncAttentionProblem.SourceFailed -> SyncProblem(failureText(problem.failure, sourceName(problem.source)), problem.retry, null)
        null -> null
    }

val SyncSnapshot.line: String
    get() = when (val work = attention.primary) {
        is SyncWork.Resetting -> t("syncStatus.resetting")
        is SyncWork.Problem -> problem?.text
        is SyncWork.Active -> runs.firstOrNull { it.run == work.run }?.let { run ->
            listOfNotNull(label(run.label), run.current?.let(::label)).joinToString(" · ")
        }
        is SyncWork.Held -> runs.firstOrNull { it.run == work.run }?.let { run ->
            run.held?.let { "${label(run.label)} · ${heldText(it)}" }
        }
        null -> null
    } ?: t("syncStatus.upToDate")

@Composable
fun SyncIndicatorView(indicator: SyncIndicator) {
    when (indicator) {
        is SyncIndicator.Progress -> CircularProgressIndicator(
            progress = { maxOf(0.03f, indicator.fraction) }, modifier = Modifier.size(20.dp), strokeWidth = 3.dp,
            color = if (indicator.emphasized) Theme.colors.accent else Theme.colors.muted, trackColor = Theme.colors.line,
        )
        is SyncIndicator.Spinner -> CircularProgressIndicator(
            Modifier.size(18.dp), strokeWidth = 2.dp, color = if (indicator.emphasized) Theme.colors.accent else Theme.colors.muted,
        )
        SyncIndicator.Failed -> Icon(Icons.Rounded.ErrorOutline, null, tint = Theme.colors.danger)
        SyncIndicator.Held -> Icon(Icons.Rounded.PauseCircle, null, tint = Theme.colors.muted)
    }
}

/** The indicator beside the page title, as a button that opens the
 * connection sheet. When work the person started finishes cleanly, a check
 * shows in its place for a moment. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SyncRingButton() {
    val model = LocalModel.current
    var sheet by remember { mutableStateOf(false) }
    var done by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    val snapshot = model.syncSnapshot
    val nowBusy = snapshot?.attention?.busy == true
    LaunchedEffect(nowBusy) {
        val finished = busy && !nowBusy && snapshot?.indicator == null
        busy = nowBusy
        if (!finished) return@LaunchedEffect
        done = true
        delay(1_600)
        done = false
    }
    val indicator = snapshot?.indicator
    if (snapshot != null && indicator != null) {
        IconButton(onClick = { sheet = true }, modifier = Modifier.semantics { contentDescription = snapshot.line }) {
            SyncIndicatorView(indicator)
        }
    } else {
        val reduced = LocalReducedMotion.current
        AnimatedVisibility(
            done,
            enter = if (reduced) fadeIn(Motion.fast()) else scaleIn(Motion.baseSpring()) + fadeIn(Motion.base()),
            exit = if (reduced) fadeOut(Motion.fast()) else scaleOut(Motion.baseSpring()) + fadeOut(Motion.base()),
        ) {
            Box(Modifier.size(48.dp).semantics { contentDescription = t("syncStatus.updated") }, Alignment.Center) {
                Icon(Icons.Rounded.CheckCircle, null, tint = Theme.colors.ok)
            }
        }
    }
    if (sheet) {
        ModalBottomSheet(onDismissRequest = { sheet = false }, containerColor = Theme.colors.bg) {
            Column(
                Modifier.verticalScroll(rememberScrollState()).padding(horizontal = Theme.gutter).padding(bottom = Theme.space10),
                verticalArrangement = Arrangement.spacedBy(Theme.space4),
            ) {
                Text(
                    t("common.schoolConnection"), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold,
                    color = Theme.colors.ink, modifier = Modifier.padding(horizontal = Theme.space8),
                )
                SyncSections()
            }
        }
    }
}

/** Sources with their last update, then the runs underway, for the sheet and
 * データ更新. */
@Composable
fun ColumnScope.SyncSections() {
    val snapshot = LocalModel.current.syncSnapshot ?: return
    if (snapshot.sources.isNotEmpty()) {
        SectionLabel(t("syncStatus.lastUpdated"))
        Card {
            snapshot.sources.forEachIndexed { index, source ->
                if (index > 0) RowDivider(inset = Theme.rowX)
                SourceRow(source)
            }
        }
    }
    if (snapshot.resetting || snapshot.runs.isNotEmpty()) {
        Card(Modifier.padding(top = Theme.space4)) {
            if (snapshot.resetting) {
                Row(Modifier.fillMaxWidth().padding(horizontal = Theme.rowX, vertical = Theme.rowY), verticalAlignment = Alignment.CenterVertically) {
                    Text(t("syncStatus.resetting"), color = Theme.colors.ink, modifier = Modifier.weight(1f))
                    CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                }
            }
            snapshot.runs.forEachIndexed { index, run ->
                if (index > 0 || snapshot.resetting) RowDivider(inset = Theme.rowX)
                RunRow(run)
            }
        }
    }
}

@Composable
private fun SourceRow(source: SourceView) {
    Column(Modifier.fillMaxWidth().padding(horizontal = Theme.rowX, vertical = Theme.rowY), verticalArrangement = Arrangement.spacedBy(Theme.space2)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(sourceName(source.source), fontWeight = FontWeight.SemiBold, color = Theme.colors.ink, modifier = Modifier.weight(1f))
            if (source.running) CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
        }
        Small(source.lastUpdated?.let { t("syncStatus.updatedDate", "date" to Format.due(it)) } ?: t("common.neverUpdated"))
        val maintenance = source.maintenance
        val failure = source.failure
        val held = source.held
        when {
            maintenance != null -> Small(
                listOf(t("syncStatus.sourceIsUnderMaintenance", "source" to sourceName(source.source)), maintenance)
                    .filter { it.isNotEmpty() }.joinToString(" · "),
                Theme.colors.warn,
            )
            failure != null -> FailureLine(failureText(failure, sourceName(source.source)), source.retry)
            held != null -> Small(heldText(held))
            source.session == SchoolSessionState.NOT_CONFIGURED -> Small(t("common.notConnected"))
        }
    }
}

/** One run: a bar only when Core knows its total, a spinner otherwise. */
@Composable
private fun RunRow(run: RunView) {
    Column(Modifier.fillMaxWidth().padding(horizontal = Theme.rowX, vertical = Theme.rowY), verticalArrangement = Arrangement.spacedBy(Theme.space2)) {
        val total = run.total
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(label(run.label), color = Theme.colors.ink, modifier = Modifier.weight(1f))
            if (total != null) {
                Small("${run.done}/$total")
            } else if (run.held == null && run.failure == null) {
                CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
            }
        }
        if (total != null && run.failure == null) {
            val fraction = minOf(run.done.toLong(), total.toLong()).toFloat() / maxOf(total.toLong(), 1L)
            LinearProgressIndicator(
                progress = { fraction }, modifier = Modifier.fillMaxWidth(),
                color = if (run.priority == SyncPriority.BACKGROUND) Theme.colors.muted else Theme.colors.accent,
            )
        }
        run.current?.let { Small(label(it)) }
        val failure = run.failure
        val held = run.held
        if (failure != null) {
            FailureLine(failureText(failure, dataset(run.dataset)), run.retry)
            // Core's diagnostic text, selectable to quote in a report.
            failure.detail?.let { SelectionContainer { Small(it) } }
        } else if (held != null) {
            Small(heldText(held))
        }
    }
}

@Composable
private fun FailureLine(text: String, retry: RetryPlan?) {
    val model = LocalModel.current
    Row(verticalAlignment = Alignment.CenterVertically) {
        Small(text, Theme.colors.danger, Modifier.weight(1f))
        if (retry != null) {
            TextButton(onClick = { model.retrySync(retry) }) { Text(t("common.retry"), fontWeight = FontWeight.SemiBold) }
        }
    }
}

@Composable
private fun Small(text: String, color: androidx.compose.ui.graphics.Color = Theme.colors.muted, modifier: Modifier = Modifier) {
    Text(text, style = MaterialTheme.typography.bodySmall, color = color, modifier = modifier)
}

/** "Needs you" (Sync feedback level 3): a failed read with Retry, or an
 * account to sign in to again. */
@Composable
fun SyncProblemBanner(problem: SyncProblem) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Theme.space5),
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Theme.radiusSurface))
            .background(Theme.colors.dangerSoft)
            .padding(start = Theme.space7, end = Theme.space2, top = Theme.space2, bottom = Theme.space2),
    ) {
        Icon(Icons.Rounded.ErrorOutline, null, tint = Theme.colors.danger)
        Text(problem.text, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.ink, modifier = Modifier.weight(1f).padding(vertical = Theme.space5))
        val profile = problem.signIn
        val retry = problem.retry
        if (profile != null) {
            TextButton(onClick = { navigator.openSettings(profile) }) { Text(t("syncStatus.signIn"), fontWeight = FontWeight.SemiBold) }
        } else if (retry != null) {
            TextButton(onClick = { model.retrySync(retry) }) { Text(t("common.retry"), fontWeight = FontWeight.SemiBold) }
        }
    }
}

/** First launch after signing in: 今日 has nothing yet, so say what is being
 * read instead of showing a blank page. */
@Composable
fun FirstLoadCard(snapshot: SyncSnapshot) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Theme.space7),
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Theme.radiusPanel))
            .background(Theme.colors.surface)
            .padding(Theme.cardPadding),
    ) {
        SyncIndicatorView(snapshot.indicator ?: SyncIndicator.Spinner(true))
        Column(verticalArrangement = Arrangement.spacedBy(Theme.space2)) {
            Text(t("syncStatus.loadingSchoolData"), fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
            Text(snapshot.line, style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted, maxLines = 2)
        }
    }
}
