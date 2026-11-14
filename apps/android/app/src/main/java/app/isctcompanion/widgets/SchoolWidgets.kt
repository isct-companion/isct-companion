package app.isctcompanion.widgets

import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.longPreferencesKey
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.glance.GlanceId
import androidx.glance.GlanceModifier
import androidx.glance.GlanceTheme
import androidx.glance.LocalSize
import androidx.glance.currentState
import androidx.glance.action.Action
import androidx.glance.action.clickable
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.appwidget.GlanceAppWidgetManager
import androidx.glance.appwidget.SizeMode
import androidx.glance.appwidget.action.actionStartActivity
import androidx.glance.appwidget.components.Scaffold
import androidx.glance.appwidget.provideContent
import androidx.glance.appwidget.state.updateAppWidgetState
import androidx.glance.state.PreferencesGlanceStateDefinition
import androidx.glance.layout.Column
import androidx.glance.layout.Spacer
import androidx.glance.layout.fillMaxWidth
import androidx.glance.layout.height
import androidx.glance.layout.padding
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextStyle
import androidx.glance.unit.ColorProvider
import app.isctcompanion.MainActivity
import app.isctcompanion.design.MessageCatalog
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

class ClassesWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = ClassesWidget
}

class AssignmentsWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = AssignmentsWidget
}

private object ClassesWidget : SchoolWidget(Purpose.Courses)
private object AssignmentsWidget : SchoolWidget(Purpose.Assignments)

internal suspend fun refreshWidgets(context: Context, courses: Boolean, assignments: Boolean) {
    if (courses) ClassesWidget.refresh(context)
    if (assignments) AssignmentsWidget.refresh(context)
}

private val refreshKey = longPreferencesKey("snapshot-refresh")

private abstract class SchoolWidget(private val purpose: Purpose) : GlanceAppWidget() {
    override val sizeMode = SizeMode.Exact
    override val stateDefinition = PreferencesGlanceStateDefinition

    suspend fun refresh(context: Context) {
        for (id in GlanceAppWidgetManager(context).getGlanceIds(javaClass)) {
            updateAppWidgetState(context, id) { state ->
                state[refreshKey] = (state[refreshKey] ?: 0L) + 1L
            }
            update(context, id)
        }
    }

    override suspend fun provideGlance(context: Context, id: GlanceId) {
        // This path can run in a process that has never opened Rust or Core.
        provideContent {
            val version = currentState<Preferences>()[refreshKey]
            val display = remember(version) { WidgetFile.read(context, purpose, Instant.now()) }
            val language = display.locale ?: context.resources.configuration.locales[0].language
            val catalog = remember(language) { MessageCatalog(context, language) }
            GlanceTheme {
                WidgetContent(context, display, catalog, purpose)
            }
        }
    }
}

@Composable
private fun WidgetContent(context: Context, display: WidgetDisplay, catalog: MessageCatalog, purpose: Purpose) {
    val size = LocalSize.current
    val rows = when {
        size.height >= 240.dp -> 5
        size.height >= 160.dp -> 3
        else -> 1
    }
    Scaffold(
        modifier = GlanceModifier.clickable(open(context, display.destination)),
        backgroundColor = GlanceTheme.colors.widgetBackground,
    ) {
        Column(modifier = GlanceModifier.fillMaxWidth().padding(vertical = 10.dp)) {
            Text(
                catalog.string(purpose.label),
                style = TextStyle(color = GlanceTheme.colors.onSurfaceVariant, fontSize = 12.sp, fontWeight = FontWeight.Medium),
                maxLines = 1,
            )
            Spacer(GlanceModifier.height(4.dp))
            val primary = display.primary
            if (primary == null) {
                Text(catalog.string(display.message ?: "widget.openToUpdate"), style = TextStyle(color = GlanceTheme.colors.onSurface))
            } else {
                RowView(context, primary, display.zone, catalog, true, rows > 1)
                for (row in display.additional.take(rows - 1)) {
                    Spacer(GlanceModifier.height(5.dp))
                    RowView(context, row, display.zone, catalog, false, true)
                }
            }
            if (size.height >= 110.dp) display.updated?.let { at ->
                Spacer(GlanceModifier.height(4.dp))
                Text(
                    catalog.string("widget.updated", "time" to formatted(at, display.zone, catalog, true)),
                    style = TextStyle(color = GlanceTheme.colors.onSurfaceVariant, fontSize = 11.sp),
                    maxLines = 1,
                )
            }
        }
    }
}

@Composable
private fun RowView(context: Context, row: WidgetRow, zone: ZoneId, catalog: MessageCatalog, hero: Boolean, link: Boolean) {
    val modifier = GlanceModifier.fillMaxWidth()
    Column(modifier = if (link) modifier.clickable(open(context, row.destination)) else modifier) {
        Text(
            row.title,
            style = TextStyle(color = GlanceTheme.colors.onSurface, fontSize = if (hero) 16.sp else 13.sp, fontWeight = FontWeight.Bold),
            maxLines = if (hero) 2 else 1,
        )
        val start = row.starts?.let { formatted(it, zone, catalog, row.dated) }
        val end = row.ends?.let { formatted(it, zone, catalog, false) }
        val interval = when {
            start == null -> null
            end != null -> "$start–$end"
            row.overdue -> catalog.string("widget.overdue", "time" to start)
            else -> start
        }
        val accent = row.color?.removePrefix("#")?.toLongOrNull(16)?.let {
            ColorProvider(Color((0xFF000000L or it).toInt()))
        }
        row.detail?.let {
            Text(it, style = TextStyle(color = accent ?: GlanceTheme.colors.onSurface, fontSize = 12.sp), maxLines = 1)
        }
        interval?.let {
            Text(it, style = TextStyle(color = GlanceTheme.colors.onSurfaceVariant, fontSize = 12.sp), maxLines = 1)
        }
    }
}

private fun formatted(at: Instant, zone: ZoneId, catalog: MessageCatalog, date: Boolean): String =
    DateTimeFormatter.ofPattern(if (date) "MMM d, HH:mm" else "HH:mm", catalog.locale).withZone(zone).format(at)

private fun open(context: Context, uri: Uri): Action =
    actionStartActivity(Intent(Intent.ACTION_VIEW, uri, context, MainActivity::class.java))
