package app.isctcompanion.features.map

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import androidx.activity.compose.LocalActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Map
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.CampusMapCandidate
import app.isctcompanion.core.CampusMapCard
import app.isctcompanion.core.CampusMapIntent
import app.isctcompanion.core.CampusMapLevel
import app.isctcompanion.core.CampusMapLink
import app.isctcompanion.core.CampusMapScene
import app.isctcompanion.design.ActionButton
import app.isctcompanion.design.ButtonLevel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import app.isctcompanion.design.tabular
import kotlinx.coroutines.channels.Channel

/** The campus map over Google's base map, full screen with a close button.
 * The page is Core's: this screen draws Core's scene, forwards what the
 * person does and adds what the platform has, the blue dot. There is no
 * route: an entrance opens in the person's maps app. */
@Composable
fun CampusMapScreen(room: String) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val context = LocalContext.current
    val density = LocalDensity.current
    val haptics = LocalHapticFeedback.current
    var scene by remember { mutableStateOf<CampusMapScene?>(null) }
    var opened by remember { mutableStateOf(false) }
    // Intents reach Core one after another, in the order sent.
    val intents = remember { Channel<CampusMapIntent>(Channel.UNLIMITED) }
    var cardHeight by remember { mutableIntStateOf(0) }
    val showsPosition = positionAllowed()
    val statusBar = WindowInsets.statusBars.getTop(density)

    LaunchedEffect(room) {
        scene = model.campusMapOpen(room)
        opened = true
        for (intent in intents) {
            val page = scene?.page ?: continue
            val next = model.campusMapSend(page, intent) ?: continue
            // Unless a newer scene is already drawn.
            if (next.revision > (scene?.revision ?: 0uL)) scene = next
        }
    }
    // Also when left before the open finished: Core drops it.
    DisposableEffect(Unit) { onDispose { model.campusMapClose() } }
    val levelId = scene?.levelId
    var lastLevel by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(levelId) {
        // A tick when the floor changes, not when the first one appears.
        if (lastLevel != null && levelId != null) haptics.performHapticFeedback(HapticFeedbackType.SegmentTick)
        lastLevel = levelId
    }

    fun send(intent: CampusMapIntent) {
        if (scene != null) intents.trySend(intent)
    }

    Box(Modifier.fillMaxSize().background(Theme.colors.bg)) {
        FloorMapView(
            scene = scene,
            // The level picker sits at the right, the card at the bottom.
            insets = {
                with(density) {
                    MapInsets(
                        left = 24.dp.roundToPx(), top = statusBar + 56.dp.roundToPx(),
                        right = 64.dp.roundToPx(), bottom = cardHeight + 16.dp.roundToPx(),
                    )
                }
            },
            showsPosition = showsPosition,
            onCamera = { camera, framing -> send(CampusMapIntent.Camera(camera, framing)) },
            // Core ignores a tap on a floor it has already left.
            onTap = { point, level -> send(CampusMapIntent.Tap(point, level)) },
            onEntrance = { send(CampusMapIntent.Entrance(it)) },
            modifier = Modifier.fillMaxSize(),
        )
        Box(
            Modifier
                .align(Alignment.TopStart)
                .windowInsetsPadding(WindowInsets.statusBars)
                .padding(start = 16.dp, top = 4.dp)
                .size(44.dp)
                .floating(CircleShape)
                .clickable(onClickLabel = t("common.close"), role = Role.Button) { navigator.back() },
            contentAlignment = Alignment.Center,
        ) {
            Icon(Icons.Rounded.Close, t("common.close"), tint = Theme.colors.ink)
        }
        val current = scene
        if (current != null && current.levels.isNotEmpty()) {
            LevelPicker(
                current.levels, current.levelId, current.markedLevelId,
                Modifier
                    .align(Alignment.CenterEnd)
                    .padding(end = 12.dp, bottom = with(density) { cardHeight.toDp() }),
            ) { send(CampusMapIntent.Level(it)) }
        }
        Column(
            Modifier
                .align(Alignment.BottomCenter)
                .onSizeChanged { cardHeight = it.height }
                .windowInsetsPadding(WindowInsets.navigationBars)
                .padding(horizontal = 12.dp)
                .padding(bottom = 8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            val attribution = current?.attribution.orEmpty()
            if (attribution.isNotEmpty()) {
                Text(
                    attribution.joinToString(" · "),
                    style = MaterialTheme.typography.labelSmall, color = Theme.colors.muted,
                    modifier = Modifier.floating(CircleShape, elevation = 2).padding(horizontal = 8.dp, vertical = 2.dp),
                )
            }
            MapCard(
                current?.card, opened,
                openLink = { openInMaps(context, it.url) },
                choose = { send(CampusMapIntent.Choose(it.toUInt())) },
            )
        }
    }
}

/** A control floating over the map: a Material surface with a soft shadow. */
@Composable
private fun Modifier.floating(shape: Shape, elevation: Int = 6): Modifier =
    shadow(elevation.dp, shape).clip(shape).background(Theme.colors.surface)

/** A `geo:` link goes to whichever maps app the person uses; with none
 * installed nothing opens. */
private fun openInMaps(context: Context, url: String) {
    runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
}

// Card

@Composable
private fun MapCard(card: CampusMapCard?, opened: Boolean, openLink: (CampusMapLink) -> Unit, choose: (Int) -> Unit) {
    Column(
        Modifier
            .fillMaxWidth()
            .floating(RoundedCornerShape(Theme.radiusPanel), elevation = 8)
            // A touch on the card stays on the card, off the map under it.
            .pointerInput(Unit) {}
            .padding(Theme.cardPadding),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        if (card != null) {
            Text(
                card.title ?: t("campusMap.entrance"),
                style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink,
                maxLines = 2, overflow = TextOverflow.Ellipsis,
                modifier = Modifier.semantics { heading() },
            )
            Text(
                listOfNotNull(card.buildingName, card.levelName).joinToString(" · "),
                style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted,
                maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
            if (card.buildingOnly) {
                Text(t("campusMap.buildingOnly"), style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
            }
            Links(card.links, openLink)
            if (card.candidates.isNotEmpty()) Candidates(card.candidates, choose)
        } else if (opened) {
            Text(
                t("campusMap.unavailable"),
                style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink,
            )
        } else {
            CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
        }
    }
}

/** One "Open in Maps" per link, Core's first one primary. */
@Composable
private fun Links(links: List<CampusMapLink>, open: (CampusMapLink) -> Unit) {
    Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        links.forEachIndexed { index, link ->
            val name = link.name ?: t("campusMap.entrance")
            val spoken = if (link.entrance) t("campusMap.openEntranceInMaps", "name" to name) else t("campusMap.openInMaps")
            ActionButton(
                if (links.size == 1) t("campusMap.openInMaps") else name,
                onClick = { open(link) },
                modifier = Modifier.semantics { contentDescription = spoken },
                level = if (index == 0) ButtonLevel.Primary else ButtonLevel.Secondary,
                icon = Icons.Rounded.Map,
            )
        }
    }
}

/** Every room an ambiguous spelling names; one is shown at a time. */
@Composable
private fun Candidates(candidates: List<CampusMapCandidate>, choose: (Int) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(
            t("campusMap.chooseRoom"),
            style = MaterialTheme.typography.bodySmall, fontWeight = FontWeight.SemiBold, color = Theme.colors.muted,
        )
        Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            candidates.forEachIndexed { index, candidate ->
                Text(
                    "${candidate.label} · ${candidate.buildingName}",
                    style = MaterialTheme.typography.bodySmall, fontWeight = FontWeight.SemiBold, maxLines = 1,
                    color = if (candidate.chosen) Theme.colors.accentInk else Theme.colors.accent,
                    modifier = Modifier
                        .clip(CircleShape)
                        .background(if (candidate.chosen) Theme.colors.accent else Theme.colors.accentSoft)
                        .clickable(role = Role.Button) { choose(index) }
                        .semantics { selected = candidate.chosen }
                        .padding(horizontal = 10.dp, vertical = 8.dp),
                )
            }
        }
    }
}

/** Floors top floor first. A dot marks the floor the selected room is on. */
@Composable
private fun LevelPicker(
    levels: List<CampusMapLevel>,
    selected: String?,
    marked: String?,
    modifier: Modifier = Modifier,
    pick: (String) -> Unit,
) {
    Column(modifier.floating(RoundedCornerShape(Theme.radiusChip))) {
        for (level in levels) {
            val isSelected = level.id == selected
            val spoken = t("campusMap.level", "name" to level.name)
            Box(
                Modifier
                    .size(width = 44.dp, height = 40.dp)
                    .background(if (isSelected) Theme.colors.accent else Color.Transparent)
                    .clickable(role = Role.Button) { pick(level.id) }
                    .semantics {
                        contentDescription = spoken
                        this.selected = isSelected
                    },
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    level.name,
                    style = MaterialTheme.typography.bodySmall.tabular(), fontWeight = FontWeight.SemiBold,
                    color = if (isSelected) Theme.colors.accentInk else Theme.colors.ink, maxLines = 1,
                )
                if (level.id == marked) {
                    Box(
                        Modifier
                            .align(Alignment.TopEnd)
                            .padding(5.dp)
                            .size(6.dp)
                            .clip(CircleShape)
                            .background(if (isSelected) Theme.colors.accentInk else Theme.colors.accent),
                    )
                }
            }
        }
    }
}

// Position

/** Whether the map may show the blue dot. The permission is asked for when
 * the map first opens, and only while Android has no answer yet (after a
 * refusal the system settings lead on, as on iOS); the Maps SDK draws the
 * dot itself, and there is no background location. */
@Composable
private fun positionAllowed(): Boolean {
    val context = LocalContext.current
    val activity = LocalActivity.current
    val permissions = arrayOf(Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION)
    fun granted() = permissions.any { context.checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED }
    var allowed by remember { mutableStateOf(granted()) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
        allowed = result.values.any { it }
    }
    LaunchedEffect(Unit) {
        val refused = activity != null && permissions.any { activity.shouldShowRequestPermissionRationale(it) }
        if (!allowed && !refused) launcher.launch(permissions)
    }
    return allowed
}

// Entry

/** Opens the map for a room string. Nothing shows until Core can place the
 * string, which also means nothing shows before the map's file is stored. */
@Composable
fun RoomMapButton(room: String, modifier: Modifier = Modifier, label: @Composable () -> Unit) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    var placed by remember(room) { mutableStateOf(false) }
    // Again once the map's file lands (the reference source's update) or the
    // data changes.
    LaunchedEffect(room, model.contentRevision, model.syncSource("references")?.lastUpdated) {
        placed = model.campusMapHas(room)
    }
    if (placed) {
        val spoken = t("campusMap.open")
        Box(
            modifier
                .clickable(role = Role.Button) { navigator.openMap(room) }
                .semantics { contentDescription = spoken },
        ) { label() }
    }
}
