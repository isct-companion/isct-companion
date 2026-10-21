package app.isctcompanion.design

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.isctcompanion.core.Held
import app.isctcompanion.core.RunView
import app.isctcompanion.core.SchoolSite
import app.isctcompanion.core.SourceView
import app.isctcompanion.core.SyncPriority
import app.isctcompanion.core.SyncScope
import app.isctcompanion.core.SyncSnapshot

/* Core's sync snapshot, rendered as it is: what runs, why it waits and when
 * it retries all come from Core. The shell only puts datasets, sources and
 * holds into words, and turns a Retry tap into a `Refresh` hint. */

/** The words for a dataset Core reads. */
fun datasetName(dataset: String): String = when (dataset) {
    "lms.enrolment" -> t("Courses")
    "lms.deadlines" -> t("Deadlines")
    "lms.course" -> t("Course content")
    "lms.assign" -> t("Assignments")
    "lms.history" -> t("Past courses")
    "kyomu.page" -> t("教務Web pages")
    "catalog.official" -> t("Course catalog")
    "reference" -> t("Reference data")
    else -> dataset
}

fun sourceName(source: String): String = when (source) {
    "lms" -> "LMS"
    "kyomu" -> t("Kyomu Web")
    "references" -> t("Official data")
    "catalog" -> t("Course catalog")
    else -> source
}

fun heldText(held: Held): String = when (held) {
    is Held.Offline -> t("Waiting for a connection")
    is Held.Cellular -> t("Waiting for Wi-Fi")
    is Held.PowerSaver -> t("Paused: power saver")
    is Held.Background -> t("Resumes when the app is in front")
    is Held.Backoff -> retryAt(held.until)
    is Held.HostPaused -> t("{{host}} is paused until {{time}}", "host" to held.host, "time" to moment(held.until))
    is Held.SignInRequired -> t("Sign in to your school account again.")
    is Held.Resetting -> t("Resetting")
}

private fun retryAt(raw: String): String = t("Retry at {{time}}", "time" to moment(raw))

/** `14:05` today, otherwise the day as deadlines write it. */
private fun moment(raw: String): String {
    val instant = Format.date(raw) ?: return raw
    return if (Format.tokyo(instant).toLocalDate() == Format.today()) Format.time(instant) else Format.due(raw)
}

/** What a Retry on a failed run asks Core to read again. */
fun retryScope(run: RunView): SyncScope = when {
    run.dataset.startsWith("lms.") ->
        run.scope?.toULongOrNull()?.let { SyncScope.Course(it) } ?: SyncScope.Site(SchoolSite.SCIENCE_LMS)
    run.dataset == "kyomu.page" -> SyncScope.Site(SchoolSite.KYOMU)
    run.dataset == "reference" -> SyncScope.References
    run.dataset.startsWith("catalog.") -> run.scope?.toUShortOrNull()?.let { SyncScope.Catalog(it) } ?: SyncScope.Everything
    else -> SyncScope.Everything
}

private fun sourceScope(source: String): SyncScope = when (source) {
    "lms" -> SyncScope.Site(SchoolSite.SCIENCE_LMS)
    "kyomu" -> SyncScope.Site(SchoolSite.KYOMU)
    "references" -> SyncScope.References
    else -> SyncScope.Everything
}

private val RunView.active: Boolean
    get() = held == null && failure == null

/** A run with a count on screen: Core knows its total and the person waits on it. */
private val RunView.counted: Boolean
    get() = active && total != null && priority != SyncPriority.BACKGROUND

/** Header ring while Core reads something no counted row shows. */
@Composable
fun SyncSpinner(snapshot: SyncSnapshot?) {
    if (snapshot?.runs.orEmpty().any { it.active && !it.counted }) {
        CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
        Spacer(Modifier.width(16.dp))
    }
}

/** Counted runs, holds and failures; nothing when Core is idle or only busy
 * in the background. */
@Composable
fun SyncNotices(snapshot: SyncSnapshot?, onRetry: (SyncScope) -> Unit, onSignIn: () -> Unit) {
    snapshot ?: return
    val counted = snapshot.runs.filter { it.counted }
    val failed = snapshot.runs.filter { it.failure != null }
    val held = snapshot.runs
        .filter { it.failure == null && it.held != null }
        .filterNot { snapshot.resetting && it.held is Held.Resetting }
        .groupBy { heldText(it.held!!) }
    if (counted.isEmpty() && failed.isEmpty() && held.isEmpty() && !snapshot.resetting) return
    Card {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            if (snapshot.resetting) Line(t("Resetting"))
            for (run in counted) {
                val total = run.total ?: continue
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Row {
                        Line(datasetName(run.dataset), Modifier.weight(1f))
                        Line("${run.done}/$total")
                    }
                    val fraction = if (total == 0u) 0f else run.done.toLong().toFloat() / total.toLong().toFloat()
                    LinearProgressIndicator(progress = { fraction }, modifier = Modifier.fillMaxWidth())
                }
            }
            for ((reason, runs) in held) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    val names = runs.map { datasetName(it.dataset) }.distinct().joinToString(", ")
                    Line("$names · $reason", Modifier.weight(1f))
                    if (runs.any { it.held is Held.SignInRequired }) TextButton(onClick = onSignIn) { Text(t("Sign in")) }
                }
            }
            for (run in failed) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Line("${datasetName(run.dataset)} · ${t("Update failed")}")
                        (run.failure?.retryAt ?: (run.held as? Held.Backoff)?.until)?.let { Line(retryAt(it), muted = true) }
                    }
                    TextButton(onClick = { onRetry(retryScope(run)) }) { Text(t("Retry")) }
                }
            }
        }
    }
}

/** One row per source: when Core last read it, or why it is waiting. */
@Composable
fun SyncSources(sources: List<SourceView>, onRetry: (SyncScope) -> Unit) {
    Card {
        sources.forEachIndexed { index, source ->
            if (index > 0) RowDivider(inset = 16.dp)
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
            ) {
                Column(Modifier.weight(1f)) {
                    Text(sourceName(source.source), color = Theme.colors.ink)
                    Line(sourceStatus(source), muted = true)
                }
                if (source.running) CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                if (source.failure != null) {
                    TextButton(onClick = { onRetry(sourceScope(source.source)) }) { Text(t("Retry")) }
                }
            }
        }
    }
}

private fun sourceStatus(source: SourceView): String {
    source.maintenance?.let { return t("Under maintenance") }
    source.failure?.let { failure ->
        return listOfNotNull(t("Update failed"), failure.retryAt?.let(::retryAt)).joinToString(" · ")
    }
    source.held?.let { return heldText(it) }
    return source.lastUpdated?.let { t("Updated {{date}}", "date" to moment(it)) } ?: t("Never updated")
}

@Composable
private fun Line(text: String, modifier: Modifier = Modifier, muted: Boolean = false) {
    Text(
        text,
        style = MaterialTheme.typography.bodyMedium,
        color = if (muted) Theme.colors.muted else Theme.colors.ink,
        modifier = modifier,
    )
}
