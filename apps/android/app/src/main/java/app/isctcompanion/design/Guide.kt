package app.isctcompanion.design

import android.content.Context
import android.content.SharedPreferences
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp

/**
 * First-run guidance, as desktop's `components/guide.tsx`: one welcome sheet,
 * then one-time tips beside the control they explain. Android has no TipKit,
 * and Material's tooltips neither remember being seen nor wait for the sheet,
 * so the two facts live here. What has been seen is a per-device convenience,
 * kept in this device's preferences and never synced.
 */
object Guide {
    /** Every tip, as desktop's `TIPS`; only the hidden gestures this shell has. */
    val TIPS = listOf("timetable-marks", "assignment-swipe")

    private var preferences: SharedPreferences? = null

    /** Whether the welcome sheet was closed on this device. Tips wait for it. */
    var welcome by mutableStateOf(true)
        private set
    private var tips by mutableStateOf(emptySet<String>())

    fun attach(context: Context) {
        val store = context.getSharedPreferences("isct-companion-guide", Context.MODE_PRIVATE)
        preferences = store
        welcome = store.getBoolean("welcome", false)
        tips = store.getStringSet("tips", null).orEmpty()
    }

    fun closeWelcome() {
        welcome = true
        preferences?.edit()?.putBoolean("welcome", true)?.apply()
    }

    internal fun shows(id: String) = welcome && id !in tips

    internal fun dismiss(id: String) {
        tips = tips + id
        preferences?.edit()?.putStringSet("tips", tips)?.apply()
    }

    /** Clears what this device has seen; the welcome sheet and tips come back. */
    fun reset() {
        welcome = false
        tips = emptySet()
        preferences?.edit()?.clear()?.apply()
    }
}

/** Shows `title` (and `text`, if any) once, after the welcome sheet is done, until
 * "Got it". Strings are iOS `Design/Guide.swift`'s, so all shells share them. */
@Composable
fun GuideTip(id: String, title: String, text: String? = null, modifier: Modifier = Modifier) {
    if (!Guide.shows(id)) return
    Column(
        modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Theme.radiusSurface))
            .background(Theme.colors.surface)
            .padding(start = 16.dp, end = 8.dp, top = 12.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Text(t(title), fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
        text?.let { Text(t(it), style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted) }
        TextButton(onClick = { Guide.dismiss(id) }, modifier = Modifier.align(Alignment.End)) { Text(t("Got it")) }
    }
}
