package app.isctcompanion.features.print

import android.content.Intent
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.FolderOpen
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Share
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.isctcompanion.CampusPrintActivity
import app.isctcompanion.LocalNavigator
import app.isctcompanion.design.BackButton
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t

/** What the campus printer takes and the two ways in, opened from 今日's
 * print button, as on iOS: share a file to "Print via campus printer"
 * from another app, or pick one here and go straight to the print screen.
 * The picker offers only what the print screen prints, PDFs and pictures. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PrintGuideScreen() {
    val navigator = LocalNavigator.current
    val context = LocalContext.current
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri: Uri? ->
        if (uri != null) {
            // The picker's read grant is this app's, so the print screen,
            // in the same app, reads the file as it opens.
            context.startActivity(Intent(context, CampusPrintActivity::class.java).setData(uri))
        }
    }
    Column(Modifier.fillMaxSize().background(Theme.colors.bg)) {
        TopAppBar(
            title = { Text(t("campusPrint.guide.title")) },
            navigationIcon = { BackButton { navigator.back() } },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = Theme.colors.bg),
        )
        Column(
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(start = Theme.gutter, end = Theme.gutter, bottom = Theme.space10),
            verticalArrangement = Arrangement.spacedBy(Theme.space4),
        ) {
            Text(t("campusPrint.guide.intro"), fontSize = 15.sp, lineHeight = 22.sp, color = Theme.colors.muted)
            Way(Icons.Rounded.Share, t("campusPrint.guide.shareTitle")) {
                Text(t("campusPrint.guide.shareAndroid"), fontSize = 15.sp, lineHeight = 22.sp, color = Theme.colors.ink)
            }
            Way(Icons.Rounded.FolderOpen, t("campusPrint.guide.pickTitle")) {
                Text(t("campusPrint.guide.pickBody"), fontSize = 15.sp, lineHeight = 22.sp, color = Theme.colors.ink)
                Spacer(Modifier.height(Theme.space2))
                Row(
                    Modifier
                        .fillMaxWidth()
                        .height(52.dp)
                        .clip(RoundedCornerShape(Theme.radiusControl))
                        .background(Theme.colors.accent)
                        .clickable(role = Role.Button) { pick.launch(arrayOf("application/pdf", "image/*")) },
                    horizontalArrangement = Arrangement.Center,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(Icons.Rounded.FolderOpen, null, tint = Theme.colors.accentInk, modifier = Modifier.size(20.dp))
                    Spacer(Modifier.width(8.dp))
                    Text(t("campusPrint.guide.pickButton"), fontSize = 16.sp, fontWeight = FontWeight.Bold, color = Theme.colors.accentInk)
                }
            }
            Row(
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(Theme.radiusSurface))
                    .background(Theme.colors.warnSoft)
                    .padding(16.dp),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Icon(Icons.Rounded.Info, null, tint = Theme.colors.warn, modifier = Modifier.size(20.dp))
                Text(t("campusPrint.guide.formats"), fontSize = 14.sp, lineHeight = 21.sp, color = Theme.colors.ink)
            }
        }
    }
}

/** One way to print: an icon, its name and how. */
@Composable
private fun Way(icon: ImageVector, title: String, content: @Composable ColumnScope.() -> Unit) {
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Theme.radiusSurface))
            .background(Theme.colors.surface)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(
                Modifier
                    .size(40.dp)
                    .clip(RoundedCornerShape(12.dp))
                    .background(Theme.colors.accentSoft),
                contentAlignment = Alignment.Center,
            ) {
                Icon(icon, null, tint = Theme.colors.accent, modifier = Modifier.size(22.dp))
            }
            Text(title, fontSize = 16.sp, fontWeight = FontWeight.Bold, color = Theme.colors.ink)
        }
        content()
    }
}
