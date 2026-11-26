package app.isctcompanion.design

import android.os.SystemClock
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.currentStateAsState
import app.isctcompanion.LocalModel
import app.isctcompanion.features.settings.WelcomeSheet
import java.util.UUID
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

class AppNotices {
    data class Feedback(val text: String, val id: UUID = UUID.randomUUID())
    data class Action(val label: String, val run: () -> Unit)
    data class Message(
        val id: String,
        val title: String,
        val body: String? = null,
        val primary: Action,
        val secondary: List<Action> = emptyList(),
        val announcementId: String? = null,
        val cancelled: () -> Unit = {},
        val destructive: Boolean = false,
    )

    var feedback by mutableStateOf<Feedback?>(null)
        private set
    private val requests = mutableStateListOf<Message>()
    val nextRequest get() = requests.firstOrNull()

    fun show(text: String) { feedback = Feedback(text) }
    fun expire(id: UUID) { if (feedback?.id == id) feedback = null }

    fun confirm(title: String, action: String, body: String? = null, destructive: Boolean = false,
                cancelLabel: String = t("common.cancel"), run: () -> Unit) {
        requests += Message(UUID.randomUUID().toString(), title, body,
            primary = Action(action, run), secondary = listOf(Action(cancelLabel) {}), destructive = destructive)
    }

    fun takeRequest(): Message? = requests.removeFirstOrNull()
}

/** Automatic messages share the same presentation as user confirmations.
 * Focus comes from the activity window, including when a native page,
 * permission prompt, sheet or file picker takes it away. */
@Composable
fun AppNoticeLayer(snackbar: SnackbarHostState, userFlow: Boolean, connect: () -> Unit) {
    val model = LocalModel.current
    val notices = model.notices
    val scope = rememberCoroutineScope()
    val uriHandler = LocalUriHandler.current
    val resumed = LocalLifecycleOwner.current.lifecycle.currentStateAsState().value.isAtLeast(Lifecycle.State.RESUMED)
    val interactive = resumed && LocalWindowInfo.current.isWindowFocused
    var welcome by remember { mutableStateOf(false) }
    var message by remember { mutableStateOf<AppNotices.Message?>(null) }
    val offered = remember { mutableSetOf<String>() }
    val release = model.updater.offer.takeIf { model.records.settings.optBoolean("appUpdateChecks", true) }
    val feed = model.announcements
    val announcementsReady = model.announcementsReady
    val undo = model.undo
    val request = notices.nextRequest

    LaunchedEffect(resumed, interactive, userFlow, welcome, message, request, undo, Guide.welcome, release, feed, announcementsReady) {
        val id = message?.announcementId
        if (id != null && feed != null && feed.active.none { it.id == id }) {
            message = null
            return@LaunchedEffect
        }
        if (!resumed || message != null || welcome) return@LaunchedEffect
        if (request != null) {
            message = notices.takeRequest()
            return@LaunchedEffect
        }
        if (!interactive || userFlow || undo != null) return@LaunchedEffect
        if (!Guide.welcome) {
            welcome = true
            return@LaunchedEffect
        }
        if (release != null && offered.add("update:${release.version}")) {
            val close = { model.updater.answered(release.version) }
            message = AppNotices.Message(
                id = "update:${release.version}",
                title = t("appUpdate.prompt.available", "version" to release.version),
                primary = AppNotices.Action(t("appUpdate.prompt.update")) { close(); model.updater.install(release) },
                secondary = listOf(
                    AppNotices.Action(t("common.later"), close),
                    AppNotices.Action(t("appUpdate.prompt.skip")) {
                        close()
                        scope.launch { model.updateSettings { it.put("skippedAppVersion", release.version) } }
                    },
                ),
                cancelled = close,
            )
            return@LaunchedEffect
        }
        if (!announcementsReady) return@LaunchedEffect
        val item = feed?.pending?.firstOrNull { "announcement:${it.id}" !in offered } ?: return@LaunchedEffect
        val key = "announcement:${item.id}"
        offered.add(key)
        val answer = { scope.launch { model.answerAnnouncement(item.id) } }
        message = AppNotices.Message(
            id = key, title = item.title, body = item.body, announcementId = item.id,
            primary = AppNotices.Action(t("announcements.acknowledge")) { answer() },
            secondary = listOfNotNull(item.link?.let { link ->
                AppNotices.Action(link.label) { answer(); uriHandler.openUri(link.url) }
            }),
        )
    }

    val feedback = notices.feedback
    val showFeedback = interactive && !userFlow && !welcome && message == null
    if (undo != null) {
        var remaining by remember(undo.id) { mutableStateOf(4_000L) }
        LaunchedEffect(undo.id, showFeedback) {
            if (!showFeedback) return@LaunchedEffect
            val start = SystemClock.elapsedRealtime()
            val result = try {
                withTimeoutOrNull(remaining) {
                    snackbar.showSnackbar(
                        t("app.hidTitle", "title" to undo.title),
                        actionLabel = t("common.undo"),
                        duration = SnackbarDuration.Indefinite,
                    )
                }
            } finally {
                remaining = (remaining - (SystemClock.elapsedRealtime() - start)).coerceAtLeast(0)
            }
            if (result == SnackbarResult.ActionPerformed) scope.launch { model.undoHide(undo) }
            else model.dismissUndo(undo)
        }
    } else if (feedback != null) {
        LaunchedEffect(feedback.id, showFeedback) {
            if (showFeedback) {
                snackbar.showSnackbar(feedback.text, duration = SnackbarDuration.Short)
                notices.expire(feedback.id)
            }
        }
    }

    if (welcome) WelcomeSheet(onClosed = { welcome = false }, connect = connect)
    message?.let { shown ->
        fun close(action: () -> Unit) {
            if (message?.id != shown.id) return
            message = null
            action()
        }
        AlertDialog(
            onDismissRequest = { close(shown.cancelled) },
            title = { Text(shown.title) },
            text = shown.body?.let { body -> { Text(body, Modifier.verticalScroll(rememberScrollState())) } },
            confirmButton = {
                Button(
                    onClick = { close(shown.primary.run) },
                    colors = if (shown.destructive) ButtonDefaults.buttonColors(containerColor = Theme.colors.danger)
                    else ButtonDefaults.buttonColors(),
                ) { Text(shown.primary.label) }
            },
            dismissButton = {
                Row {
                    shown.secondary.forEach { action ->
                        TextButton(onClick = { close(action.run) }) { Text(action.label) }
                    }
                }
            },
        )
    }
}
