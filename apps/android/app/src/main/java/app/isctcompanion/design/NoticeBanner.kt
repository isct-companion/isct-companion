package app.isctcompanion.design

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
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalUriHandler
import app.isctcompanion.core.AnnouncementLevel
import app.isctcompanion.core.AnnouncementView

@Composable
fun NoticeBanner(
    color: Color,
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    Column(
        modifier.fillMaxWidth().clip(RoundedCornerShape(Theme.radiusSurface))
            .background(color).padding(Theme.space7),
        verticalArrangement = Arrangement.spacedBy(Theme.space3),
    ) { content() }
}

@Composable
fun AnnouncementBanners(items: List<AnnouncementView>) {
    val first = items.firstOrNull() ?: return
    var expanded by remember { mutableStateOf(false) }
    AnnouncementBanner(first)
    if (items.size > 1) {
        TextButton(onClick = { expanded = !expanded }) {
            Text(t("announcements.more", "count" to items.size - 1))
        }
        if (expanded) items.drop(1).forEach { AnnouncementBanner(it) }
    }
}

@Composable
private fun AnnouncementBanner(item: AnnouncementView) {
    val uriHandler = LocalUriHandler.current
    NoticeBanner(if (item.level == AnnouncementLevel.IMPORTANT) Theme.colors.dangerSoft else Theme.colors.surface) {
        Text(item.title, style = MaterialTheme.typography.titleSmall, color = Theme.colors.ink)
        Text(item.body, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.ink)
        item.link?.let { link ->
            TextButton(onClick = { uriHandler.openUri(link.url) }) { Text(link.label) }
        }
    }
}
