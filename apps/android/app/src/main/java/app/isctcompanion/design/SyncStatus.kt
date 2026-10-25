package app.isctcompanion.design

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
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.PauseCircle
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
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.Held
import app.isctcompanion.core.RunView
import app.isctcompanion.core.SchoolAuthProfile
import app.isctcompanion.core.SchoolSite
import app.isctcompanion.core.SourceView
import app.isctcompanion.core.SyncFailure
import app.isctcompanion.core.SyncLabel
import app.isctcompanion.core.SyncPriority
import app.isctcompanion.core.SyncScope
import app.isctcompanion.core.SyncSnapshot
import kotlinx.coroutines.delay
import org.json.JSONObject

/* Words for Core's sync snapshot (`sync_engine`). Core decides what runs, what
 * waits, what failed and when it retries; these only name it, with the iOS
 * shell's strings (`Features/Status/SchoolStatusViews.swift`). */

/** A source as Core names it in `SourceView.source`. */
fun sourceName(source: String): String = when (source) {
    "lms" -> "LMS"
    "kyomu" -> "教務Web"
    "references" -> t("Reference data")
    "catalog" -> t("Course catalog")
    else -> source
}

/** Dataset names, keyed by the dataset id Core puts in `RunView.dataset` and
 * in its labels' keys. */
private fun datasetName(key: String): String? = when (key) {
    "lms.enrolment" -> t("Courses")
    "lms.deadlines" -> t("Deadlines")
    "lms.course" -> t("Course content")
    "lms.assign" -> t("Assignments")
    "lms.history" -> t("Past courses")
    "kyomu.page" -> t("教務Web pages")
    "catalog.official" -> t("Course catalog")
    "reference" -> t("Reference data")
    else -> null
}

fun dataset(dataset: String): String = datasetName(dataset) ?: dataset

/** A Core label: a dataset key reads as the dataset's name; any other key is
 * a catalog key filled with Core's arguments. */
fun label(label: SyncLabel): String {
    datasetName(label.key.removePrefix("sync."))?.let { return it }
    val args = runCatching { JSONObject(label.args) }.getOrNull()
    val values = args?.keys()?.asSequence()?.map { it to args.get(it) }?.toList().orEmpty()
    return t(label.key, *values.toTypedArray())
}

fun heldText(held: Held): String = when (held) {
    is Held.Offline -> t("Waiting for a connection")
    is Held.Cellular -> t("Waiting for Wi-Fi")
    // Android's Battery Saver, not iOS's Low Power Mode.
    is Held.PowerSaver -> t("Paused: power saver")
    is Held.Background -> t("Resumes when the app is in front")
    is Held.Backoff -> retryAt(held.until)
    is Held.HostPaused -> t("{{host}} is paused until {{time}}", "host" to held.host, "time" to Format.due(held.until))
    is Held.SignInRequired -> t("Sign in required")
    is Held.Resetting -> t("Resetting")
    is Held.Prerequisite -> t("Waiting for the data it depends on")
}

private fun retryAt(raw: String): String = t("Retry at {{time}}", "time" to Format.due(raw))

/** "Could not update Deadlines · Retry at Today 14:05". */
private fun failureText(failure: SyncFailure, name: String): String =
    listOfNotNull(t("Could not update {{source}}", "source" to name), failure.retryAt?.let(::retryAt)).joinToString(" · ")

/** What a Retry asks Core to read again for a failed run. */
private fun retryScope(run: RunView): SyncScope = when {
    run.dataset.startsWith("lms.") ->
        run.scope?.toULongOrNull()?.let { SyncScope.Course(it) } ?: SyncScope.Site(SchoolSite.SCIENCE_LMS)
    run.dataset.startsWith("kyomu.") -> SyncScope.Site(SchoolSite.KYOMU)
    run.dataset == "reference" -> SyncScope.References
    run.dataset.startsWith("catalog.") -> run.scope?.toUShortOrNull()?.let { SyncScope.Catalog(it) } ?: SyncScope.Everything
    else -> SyncScope.Everything
}

/** What a Retry asks Core to read again for a failed source. */
private fun retryScope(source: SourceView): SyncScope = when (source.source) {
    "lms" -> SyncScope.Site(SchoolSite.SCIENCE_LMS)
    "kyomu" -> SyncScope.Site(SchoolSite.KYOMU)
    "references" -> SyncScope.References
    else -> SyncScope.Everything
}

/** What the header shows for Core's runs: a ring only when a run has a
 * total, a spinner for other work, an icon when everything waits or failed. */
sealed interface SyncIndicator {
    data class Progress(val fraction: Float, val emphasized: Boolean) : SyncIndicator
    data class Spinner(val emphasized: Boolean) : SyncIndicator
    data object Failed : SyncIndicator
    data object Held : SyncIndicator
}

/** The one thing that needs the person, for 今日's banner. */
data class SyncProblem(val text: String, val retry: SyncScope?, val signIn: SchoolAuthProfile?)

/** Runs reading now: neither held nor failed. */
val SyncSnapshot.activeRuns: List<RunView>
    get() = runs.filter { it.held == null && it.failure == null }

/** Work the person asked for or is looking at, as opposed to background reads. */
val SyncSnapshot.isBusy: Boolean
    get() = resetting || activeRuns.any { it.priority != SyncPriority.BACKGROUND }

val SyncSnapshot.indicator: SyncIndicator?
    get() {
        val active = activeRuns
        active.firstOrNull { it.total != null }?.let { run ->
            val total = run.total!!.toLong()
            return SyncIndicator.Progress(
                minOf(run.done.toLong(), total).toFloat() / maxOf(total, 1L), run.priority != SyncPriority.BACKGROUND,
            )
        }
        if (resetting) return SyncIndicator.Spinner(true)
        active.firstOrNull()?.let { return SyncIndicator.Spinner(it.priority != SyncPriority.BACKGROUND) }
        if (problem != null) return SyncIndicator.Failed
        if (runs.any { it.held != null }) return SyncIndicator.Held
        return null
    }

val SyncSnapshot.problem: SyncProblem?
    get() {
        val held = runs.mapNotNull { it.held } + sources.mapNotNull { it.held }
        held.firstNotNullOfOrNull { it as? Held.SignInRequired }?.let {
            return SyncProblem(t("Sign in required"), null, it.profile)
        }
        runs.firstOrNull { it.failure != null }?.let { run ->
            return SyncProblem(failureText(run.failure!!, label(run.label)), retryScope(run), null)
        }
        sources.firstOrNull { it.failure != null }?.let { source ->
            return SyncProblem(failureText(source.failure!!, sourceName(source.source)), retryScope(source), null)
        }
        return null
    }

/** One line for the first-load card and the indicator's accessibility label. */
val SyncSnapshot.line: String
    get() {
        if (resetting) return t("Resetting")
        activeRuns.firstOrNull()?.let { run -> return listOfNotNull(label(run.label), run.current?.let(::label)).joinToString(" · ") }
        problem?.let { return it.text }
        runs.firstOrNull { it.held != null }?.let { return "${label(it.label)} · ${heldText(it.held!!)}" }
        return t("Up to date")
    }

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
        SyncIndicator.Failed -> Icon(Icons.Outlined.ErrorOutline, null, tint = Theme.colors.danger)
        SyncIndicator.Held -> Icon(Icons.Outlined.PauseCircle, null, tint = Theme.colors.muted)
    }
}

/** The indicator beside the page title, as a button that opens the
 * connection sheet. When work the person started finishes cleanly, a check
 * shows in its place for a moment. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SyncRingButton() {
    val model = LocalModel.current
    val haptics = LocalHapticFeedback.current
    var sheet by remember { mutableStateOf(false) }
    var done by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    val snapshot = model.syncSnapshot
    val nowBusy = snapshot?.isBusy == true
    LaunchedEffect(nowBusy) {
        val finished = busy && !nowBusy && snapshot?.indicator == null
        busy = nowBusy
        if (!finished) return@LaunchedEffect
        haptics.performHapticFeedback(HapticFeedbackType.Confirm)
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
        AnimatedVisibility(done, enter = scaleIn(), exit = scaleOut()) {
            Box(Modifier.size(48.dp).semantics { contentDescription = t("Updated") }, Alignment.Center) {
                Icon(Icons.Outlined.CheckCircle, null, tint = Theme.colors.ok)
            }
        }
    }
    if (sheet) {
        ModalBottomSheet(onDismissRequest = { sheet = false }, containerColor = Theme.colors.bg) {
            Column(
                Modifier.verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(bottom = 24.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(
                    t("School connection"), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold,
                    color = Theme.colors.ink, modifier = Modifier.padding(horizontal = 16.dp),
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
        SectionLabel(t("Last updated"))
        Card {
            snapshot.sources.forEachIndexed { index, source ->
                if (index > 0) RowDivider(inset = 16.dp)
                SourceRow(source)
            }
        }
    }
    if (snapshot.resetting || snapshot.runs.isNotEmpty()) {
        Card(Modifier.padding(top = 8.dp)) {
            if (snapshot.resetting) {
                Row(Modifier.fillMaxWidth().padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(t("Resetting"), color = Theme.colors.ink, modifier = Modifier.weight(1f))
                    CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                }
            }
            snapshot.runs.forEachIndexed { index, run ->
                if (index > 0 || snapshot.resetting) RowDivider(inset = 16.dp)
                RunRow(run)
            }
        }
    }
}

@Composable
private fun SourceRow(source: SourceView) {
    val model = LocalModel.current
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(sourceName(source.source), fontWeight = FontWeight.SemiBold, color = Theme.colors.ink, modifier = Modifier.weight(1f))
            if (source.running) CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
        }
        Small(source.lastUpdated?.let { t("Updated {{date}}", "date" to Format.due(it)) } ?: t("Never updated"))
        val maintenance = source.maintenance
        val failure = source.failure
        val held = source.held
        when {
            maintenance != null -> Small(
                listOf(t("{{source}} is under maintenance", "source" to sourceName(source.source)), maintenance)
                    .filter { it.isNotEmpty() }.joinToString(" · "),
                Theme.colors.warn,
            )
            failure != null -> FailureLine(failureText(failure, sourceName(source.source))) { model.refresh(retryScope(source)) }
            held != null -> Small(heldText(held))
            source.session == "signed-out" -> Small(t("Not connected"))
        }
    }
}

/** One run: a bar only when Core knows its total, a spinner otherwise. */
@Composable
private fun RunRow(run: RunView) {
    val model = LocalModel.current
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
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
            FailureLine(failureText(failure, dataset(run.dataset))) { model.refresh(retryScope(run)) }
        } else if (held != null) {
            Small(heldText(held))
        }
    }
}

@Composable
private fun FailureLine(text: String, retry: () -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Small(text, Theme.colors.danger, Modifier.weight(1f))
        TextButton(onClick = retry) { Text(t("Retry"), fontWeight = FontWeight.SemiBold) }
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
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Theme.radiusSurface))
            .background(Theme.colors.dangerSoft)
            .padding(start = 14.dp, end = 4.dp, top = 4.dp, bottom = 4.dp),
    ) {
        Icon(Icons.Outlined.ErrorOutline, null, tint = Theme.colors.danger)
        Text(problem.text, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.ink, modifier = Modifier.weight(1f).padding(vertical = 10.dp))
        val profile = problem.signIn
        val retry = problem.retry
        if (profile != null) {
            TextButton(onClick = { navigator.openSettings(profile) }) { Text(t("Sign in"), fontWeight = FontWeight.SemiBold) }
        } else if (retry != null) {
            TextButton(onClick = { model.refresh(retry) }) { Text(t("Retry"), fontWeight = FontWeight.SemiBold) }
        }
    }
}

/** First launch after signing in: 今日 has nothing yet, so say what is being
 * read instead of showing a blank page. */
@Composable
fun FirstLoadCard(snapshot: SyncSnapshot) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Theme.radiusPanel))
            .background(Theme.colors.surface)
            .padding(18.dp),
    ) {
        SyncIndicatorView(snapshot.indicator ?: SyncIndicator.Spinner(true))
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(t("Loading school data"), fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
            Text(snapshot.line, style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted, maxLines = 2)
        }
    }
}
