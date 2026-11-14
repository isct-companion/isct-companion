package app.isctcompanion.features.map

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.activity.compose.LocalActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Cancel
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Layers
import androidx.compose.material.icons.rounded.Map
import androidx.compose.material.icons.rounded.MyLocation
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.CampusMapCandidate
import app.isctcompanion.core.CampusMapCard
import app.isctcompanion.core.CampusMapIntent
import app.isctcompanion.core.CampusMapLevel
import app.isctcompanion.core.CampusMapLink
import app.isctcompanion.core.CampusMapOpening
import app.isctcompanion.core.CampusMapScene
import app.isctcompanion.core.CampusMapSearch
import app.isctcompanion.core.CampusMapSearchResult
import app.isctcompanion.design.ActionButton
import app.isctcompanion.design.ButtonLevel
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import app.isctcompanion.design.tabular
import kotlinx.coroutines.channels.Channel

/** The page's control size, the gap between controls and its side inset. */
private val Control = 44.dp
private val ControlGap = 8.dp
private val PageInset = 12.dp

/** Where opening the map stands. */
private sealed interface MapLoad {
    data object Loading : MapLoad

    /** No map file is stored yet: it comes with the data updates. */
    data object Missing : MapLoad

    data class Failed(val message: String) : MapLoad

    data object Ready : MapLoad
}

/** An intent on its way to Core, and what to do with the answer. */
private class Sent(val intent: CampusMapIntent, val done: ((CampusMapScene?) -> Unit)?)

/** The campus map over OpenFreeMap's base map, full screen with a close
 * button, opened on a room string or on none to browse. The page is Core's:
 * this screen draws Core's scene, forwards what the person does and adds
 * what the platform has, the blue dot. There is no route: an entrance opens
 * in the maps app the settings name.
 *
 * Search, its results and the place card share one panel at the bottom, as
 * on iOS. The map's visible part is what the panel, the close button and
 * the control rail leave. */
@Composable
fun CampusMapScreen(room: String?) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val context = LocalContext.current
    val density = LocalDensity.current
    val haptics = LocalHapticFeedback.current
    val focus = LocalFocusManager.current
    var load by remember { mutableStateOf<MapLoad>(MapLoad.Loading) }
    var attempt by remember { mutableIntStateOf(0) }
    var scene by remember { mutableStateOf<CampusMapScene?>(null) }
    // Intents reach Core one after another, in the order sent.
    val intents = remember { Channel<Sent>(Channel.UNLIMITED) }
    var panelHeight by remember { mutableIntStateOf(0) }
    var cardExpanded by remember { mutableStateOf(true) }
    var query by remember { mutableStateOf("") }
    var searchFocused by remember { mutableStateOf(false) }
    var searchAnswer by remember { mutableStateOf<CampusMapSearch?>(null) }
    var searchFailure by remember { mutableStateOf<String?>(null) }
    var pickingResult by remember { mutableStateOf(false) }
    val position = rememberPosition()

    val searching = searchFocused || query.isNotEmpty()
    val card = scene?.card
    val showsCard = card != null && !searching

    LaunchedEffect(attempt) {
        load = MapLoad.Loading
        when (val opened = model.campusMapOpen(room)) {
            is CampusMapOpening.Ready -> {
                scene = opened.scene
                load = MapLoad.Ready
            }
            CampusMapOpening.Missing -> {
                load = MapLoad.Missing
                return@LaunchedEffect
            }
            is CampusMapOpening.Failed -> {
                load = MapLoad.Failed(opened.message)
                return@LaunchedEffect
            }
        }
        for (sent in intents) {
            val page = scene?.page ?: continue
            val next = model.campusMapSend(page, sent.intent)
            // Unless a newer scene is already drawn.
            if (next != null && next.revision > (scene?.revision ?: 0uL)) scene = next
            sent.done?.invoke(next)
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
    // A new place opens unfolded.
    LaunchedEffect(card == null) { if (card == null) cardExpanded = true }

    fun send(intent: CampusMapIntent, done: ((CampusMapScene?) -> Unit)? = null) {
        if (scene != null) intents.trySend(Sent(intent, done))
    }

    fun endSearch() {
        focus.clearFocus()
        query = ""
    }

    /** A pick on the map while searching ends the search once its card is there. */
    fun selectOnMap(intent: CampusMapIntent) = send(intent) { next ->
        if ((searchFocused || query.isNotEmpty()) && next?.card != null) {
            cardExpanded = true
            endSearch()
        }
    }

    // Asks Core for the text's places; an answer to text the person has
    // since changed is dropped (Core echoes the text it answered). The map
    // does not move while they type.
    LaunchedEffect(query) {
        searchAnswer = null
        searchFailure = null
        if (query.isBlank()) return@LaunchedEffect
        model.campusMapSearch(query).fold(
            onSuccess = { found -> if (found.query == query) searchAnswer = found },
            onFailure = { searchFailure = it.message },
        )
    }
    BackHandler(enabled = searching) { endSearch() }

    // The map's visible part: under the close button's row, beside the
    // control rail, above the panel and whatever lifts it (the keyboard, the
    // navigation bar).
    val statusBar = WindowInsets.statusBars.getTop(density)
    val keyboard = WindowInsets.ime.getBottom(density)
    val lift = maxOf(keyboard, WindowInsets.navigationBars.getBottom(density))
    val chrome = with(density) {
        MapInsets(
            left = 0,
            top = statusBar + (ControlGap / 2 + Control).roundToPx(),
            right = (PageInset + Control).roundToPx(),
            bottom = panelHeight + ControlGap.roundToPx() + lift,
        )
    }
    val gap = with(density) { ControlGap.roundToPx() }

    Box(Modifier.fillMaxSize().background(Theme.colors.bg)) {
        FloorMapView(
            scene = scene,
            chrome = chrome,
            // The panel's height as measured in this frame's layout.
            insets = { chrome.copy(bottom = panelHeight + gap + lift) },
            canFrame = keyboard == 0,
            showsPosition = position.allowed,
            follows = position.follows,
            onCamera = { camera, framing -> send(CampusMapIntent.Camera(camera, framing)) },
            // Core ignores a tap on a floor it has already left.
            onTap = { point, level -> selectOnMap(CampusMapIntent.Tap(point, level)) },
            onSelect = { selectOnMap(CampusMapIntent.Select(it)) },
            modifier = Modifier.fillMaxSize(),
        )
        Box(
            Modifier
                .align(Alignment.TopStart)
                .windowInsetsPadding(WindowInsets.statusBars)
                .padding(start = PageInset, top = ControlGap / 2)
                .size(Control)
                .floating(CircleShape)
                .clickable(onClickLabel = t("common.close"), role = Role.Button) { navigator.back() },
            contentAlignment = Alignment.Center,
        ) {
            Icon(Icons.Rounded.Close, t("common.close"), tint = Theme.colors.ink)
        }
        // The control rail: MapLibre's compass heads it, then the position
        // button, then the floors, which scroll within what is left.
        Column(
            Modifier
                .align(Alignment.TopEnd)
                .padding(
                    top = with(density) { chrome.top.toDp() } + ControlGap,
                    end = PageInset,
                    bottom = with(density) { chrome.bottom.toDp() } + ControlGap,
                )
                .width(Control),
            verticalArrangement = Arrangement.spacedBy(ControlGap),
        ) {
            val spoken = t("campusMap.position")
            Box(
                Modifier
                    .size(Control)
                    .floating(RoundedCornerShape(Theme.radiusChip))
                    .clickable(role = Role.Button) { position.locate() }
                    .semantics { contentDescription = spoken },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    Icons.Rounded.MyLocation, null,
                    tint = if (position.allowed) Theme.colors.accent else Theme.colors.ink,
                    modifier = Modifier.size(20.dp),
                )
            }
            val current = scene
            // The building's floors as a stack seen at an angle, and back.
            if (current != null && (current.levels.isNotEmpty() || current.stacked)) {
                val stacked = current.stacked
                val label = t(if (stacked) "campusMap.stackOff" else "campusMap.stackOn")
                Box(
                    Modifier
                        .size(Control)
                        .floating(RoundedCornerShape(Theme.radiusChip))
                        .background(if (stacked) Theme.colors.accent else Color.Transparent)
                        .clickable(role = Role.Button) { send(CampusMapIntent.Stack(!stacked)) }
                        .semantics {
                            contentDescription = label
                            this.selected = stacked
                        },
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        Icons.Rounded.Layers, null,
                        tint = if (stacked) Theme.colors.accentInk else Theme.colors.ink,
                        modifier = Modifier.size(20.dp),
                    )
                }
            }
            if (current != null && current.levels.isNotEmpty()) {
                LevelPicker(
                    current.levels, current.levelId, current.markedLevelId,
                    Modifier.weight(1f, fill = false),
                ) { send(CampusMapIntent.Level(it)) }
            }
        }
        Column(
            Modifier
                .align(Alignment.BottomCenter)
                .padding(bottom = with(density) { lift.toDp() } + ControlGap)
                .padding(horizontal = PageInset),
            verticalArrangement = Arrangement.spacedBy(ControlGap),
        ) {
            val attribution = scene?.attribution.orEmpty()
            if (attribution.isNotEmpty() && !searching) {
                Text(
                    attribution.joinToString(" · "),
                    style = MaterialTheme.typography.labelSmall, color = Theme.colors.muted,
                    modifier = Modifier
                        .padding(start = Control)
                        .floating(CircleShape, elevation = 2)
                        .padding(horizontal = 8.dp, vertical = 2.dp),
                )
            }
            Column(
                Modifier
                    .fillMaxWidth()
                    .onSizeChanged { panelHeight = it.height }
                    .floating(RoundedCornerShape(Theme.radiusPanel), elevation = 8)
                    // A touch on the panel stays on the panel, off the map under it.
                    .pointerInput(Unit) {},
            ) {
                if (card != null && showsCard) {
                    CardHeader(
                        card, cardExpanded,
                        fold = { cardExpanded = it },
                        close = { send(CampusMapIntent.Clear) },
                    )
                } else {
                    SearchHeader(
                        query, searching, enabled = load == MapLoad.Ready,
                        onQuery = { query = it },
                        onFocus = { searchFocused = it },
                        cancel = { endSearch() },
                    )
                }
                when (val state = load) {
                    MapLoad.Loading -> Box(Modifier.fillMaxWidth().padding(bottom = Theme.cardPadding), Alignment.Center) {
                        CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                    }
                    MapLoad.Missing -> Notice(t("campusMap.unavailable"), t("campusMap.notDownloaded"))
                    is MapLoad.Failed -> Notice(t("campusMap.failed"), state.message) { attempt += 1 }
                    MapLoad.Ready -> when {
                        searching -> Results(query, searchAnswer, searchFailure) { result ->
                            if (!pickingResult) {
                                pickingResult = true
                                send(CampusMapIntent.Select(result.place)) { next ->
                                    pickingResult = false
                                    if (next?.card != null) {
                                        cardExpanded = true
                                        endSearch()
                                    }
                                }
                            }
                        }
                        card != null && cardExpanded -> CardDetails(
                            card,
                            openLink = { openInMaps(context, it.url) },
                            choose = { send(CampusMapIntent.Choose(it.toUInt())) },
                        )
                    }
                }
            }
        }
    }
    if (position.settingsRequired) {
        AlertDialog(
            onDismissRequest = { position.settingsRequired = false },
            title = { Text(t("campusMap.locationAccessTitle")) },
            text = { Text(t("campusMap.locationAccessMessage")) },
            confirmButton = {
                TextButton(onClick = {
                    position.settingsRequired = false
                    runCatching {
                        context.startActivity(
                            Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null)),
                        )
                    }
                }) { Text(t("settings.main.openSettings")) }
            },
            dismissButton = { TextButton(onClick = { position.settingsRequired = false }) { Text(t("common.cancel")) } },
        )
    }
}

/** A control floating over the map: a Material surface with a soft shadow. */
@Composable
private fun Modifier.floating(shape: Shape, elevation: Int = 6): Modifier =
    shadow(elevation.dp, shape).clip(shape).background(Theme.colors.surface)

/** Core's link is a plain https link to the maps app the settings name; the
 * app opens it when installed, else the browser does. */
private fun openInMaps(context: Context, url: String) {
    runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
}

@Composable
private fun campusName(campus: String): String = when (campus) {
    "suzukakedai" -> t("campusMap.suzukakedai")
    else -> t("campusMap.ookayama")
}

// Panel

@Composable
private fun SearchHeader(
    query: String,
    searching: Boolean,
    enabled: Boolean,
    onQuery: (String) -> Unit,
    onFocus: (Boolean) -> Unit,
    cancel: () -> Unit,
) {
    Row(
        Modifier.padding(horizontal = Theme.cardPadding, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Row(
            Modifier
                .weight(1f)
                .defaultMinSize(minHeight = Control)
                .clip(RoundedCornerShape(12.dp))
                .background(Theme.colors.surface2)
                .padding(horizontal = 12.dp),
            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon(Icons.Rounded.Search, null, tint = Theme.colors.muted, modifier = Modifier.size(20.dp))
            BasicTextField(
                value = query,
                onValueChange = onQuery,
                enabled = enabled,
                singleLine = true,
                textStyle = MaterialTheme.typography.bodyLarge.copy(color = Theme.colors.ink),
                cursorBrush = SolidColor(Theme.colors.accent),
                // Room codes are capitals and digits; no word is being spelt.
                keyboardOptions = KeyboardOptions(
                    capitalization = KeyboardCapitalization.Characters, autoCorrectEnabled = false, imeAction = ImeAction.Search,
                ),
                modifier = Modifier.weight(1f).onFocusChanged { onFocus(it.isFocused) },
                decorationBox = { field ->
                    Box(contentAlignment = Alignment.CenterStart) {
                        if (query.isEmpty()) {
                            Text(
                                t("campusMap.search"), style = MaterialTheme.typography.bodyLarge, color = Theme.colors.muted,
                                maxLines = 1, overflow = TextOverflow.Ellipsis,
                            )
                        }
                        field()
                    }
                },
            )
        }
        if (searching) TextButton(onClick = cancel) { Text(t("common.cancel")) }
    }
}

/** The place's name with the button that closes the card. The grabber over
 * it folds the details away, by a tap or a drag. */
@Composable
private fun CardHeader(card: CampusMapCard, expanded: Boolean, fold: (Boolean) -> Unit, close: () -> Unit) {
    val spoken = t(if (expanded) "campusMap.foldCard" else "campusMap.unfoldCard")
    Column {
        Box(
            Modifier
                .fillMaxWidth()
                .clickable(role = Role.Button) { fold(!expanded) }
                .pointerInput(Unit) {
                    var travel = 0f
                    detectVerticalDragGestures(
                        onDragStart = { travel = 0f },
                        onDragEnd = { if (travel != 0f) fold(travel < 0) },
                    ) { _, moved -> travel += moved }
                }
                .semantics { contentDescription = spoken }
                .padding(vertical = 8.dp),
            contentAlignment = Alignment.Center,
        ) {
            Box(Modifier.size(width = 36.dp, height = 5.dp).clip(CircleShape).background(Theme.colors.muted.copy(alpha = 0.55f)))
        }
        Row(
            Modifier.padding(start = Theme.cardPadding, end = 4.dp, bottom = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                card.title ?: t("campusMap.entrance"),
                style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold, color = Theme.colors.ink,
                maxLines = 2, overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f).semantics { heading() },
            )
            Box(
                Modifier.size(Control).clip(CircleShape).clickable(role = Role.Button, onClick = close),
                contentAlignment = Alignment.Center,
            ) {
                Icon(Icons.Rounded.Cancel, t("campusMap.closeCard"), tint = Theme.colors.muted)
            }
        }
    }
}

/** Details below the place header, while the card is unfolded. */
@Composable
private fun CardDetails(card: CampusMapCard, openLink: (CampusMapLink) -> Unit, choose: (Int) -> Unit) {
    Column(
        Modifier.padding(start = Theme.cardPadding, end = Theme.cardPadding, bottom = Theme.cardPadding),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        // The building unless it is the title, the floor, the campus.
        val subtitle = listOfNotNull(card.buildingName.takeIf { it != card.title }, card.levelName, campusName(card.campus))
        Text(
            subtitle.joinToString(" · "),
            style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted,
        )
        if (card.buildingOnly) {
            Text(t("campusMap.buildingOnly"), style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
        } else if (card.noFloorPlans) {
            Text(t("campusMap.noFloorPlans"), style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
        }
        Links(card.links, openLink)
        if (card.candidates.isNotEmpty()) Candidates(card.candidates, choose)
    }
}

/** A state the map cannot be drawn in, with at most one way on. */
@Composable
private fun Notice(title: String, detail: String, retry: (() -> Unit)? = null) {
    Column(
        Modifier.padding(start = Theme.cardPadding, end = Theme.cardPadding, bottom = Theme.cardPadding),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
        Text(detail, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted)
        if (retry != null) ActionButton(t("common.retry"), onClick = retry, level = ButtonLevel.Secondary)
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

// Search

/** Nothing for an empty query or before Core answers the text shown. */
@Composable
private fun Results(query: String, answer: CampusMapSearch?, failure: String?, pick: (CampusMapSearchResult) -> Unit) {
    when {
        failure != null -> Notice(t("campusMap.failed"), failure)
        answer == null || answer.query != query -> Unit
        answer.results.isEmpty() -> Text(
            t("campusMap.noResults"),
            style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted,
            modifier = Modifier.padding(start = Theme.cardPadding, end = Theme.cardPadding, bottom = Theme.cardPadding),
        )
        // About three rows: the map stays in view above the keyboard.
        else -> LazyColumn(Modifier.heightIn(max = 200.dp)) {
            items(answer.results) { result -> SearchRow(result) { pick(result) } }
        }
    }
}

/** `大岡山西9号館 · 2F · 大岡山`: the building unless it is the title, the
 * floor, the campus. */
@Composable
private fun SearchRow(result: CampusMapSearchResult, pick: () -> Unit) {
    Column(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = pick)
            .defaultMinSize(minHeight = 56.dp)
            .padding(horizontal = Theme.cardPadding, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp, Alignment.CenterVertically),
    ) {
        Text(result.title, style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.SemiBold, color = Theme.colors.ink)
        val details = listOfNotNull(result.buildingName.takeIf { it != result.title }, result.levelName, campusName(result.campus))
        Text(details.joinToString(" · "), style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
        if (result.noFloorPlan) {
            Text(t("campusMap.notOnFloorPlans"), style = MaterialTheme.typography.bodySmall, color = Theme.colors.warn)
        }
    }
}

// Floors

/** Floors top floor first. A dot marks the floor the selected room is on. A
 * tall building scrolls within the space the page leaves, keeping the floor
 * shown in view. A floor with no plan (the stack lists the ones between two
 * that have one) is greyed and cannot be picked. */
@Composable
private fun LevelPicker(
    levels: List<CampusMapLevel>,
    selected: String?,
    marked: String?,
    modifier: Modifier = Modifier,
    pick: (String) -> Unit,
) {
    val scroll = rememberScrollState()
    val cell = with(LocalDensity.current) { Control.roundToPx() }
    LaunchedEffect(selected, levels.size) {
        val index = levels.indexOfFirst { it.id == selected }
        if (index >= 0) scroll.animateScrollTo((index * cell - (scroll.viewportSize - cell) / 2).coerceAtLeast(0))
    }
    Column(modifier.floating(RoundedCornerShape(Theme.radiusChip)).verticalScroll(scroll)) {
        for (level in levels) {
            val isSelected = level.id == selected
            val spoken = t("campusMap.level", "name" to level.name)
            val destination = if (level.id == marked) t("campusMap.destinationFloor") else ""
            Box(
                Modifier
                    .size(Control)
                    .background(if (isSelected) Theme.colors.accent else Color.Transparent)
                    .clickable(enabled = level.hasPlan, role = Role.Button) { pick(level.id) }
                    .semantics {
                        contentDescription = spoken
                        stateDescription = destination
                        this.selected = isSelected
                    },
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    level.name,
                    style = MaterialTheme.typography.bodySmall.tabular(), fontWeight = FontWeight.SemiBold,
                    color = when {
                        isSelected -> Theme.colors.accentInk
                        level.hasPlan -> Theme.colors.ink
                        else -> Theme.colors.muted.copy(alpha = 0.5f)
                    },
                    maxLines = 1,
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

/** The blue dot's permission, asked for only when the person taps the
 * position button; once allowed, the map follows them. MapLibre draws the
 * dot itself, and there is no background location. */
private class Position(private val granted: () -> Boolean, private val ask: () -> Unit) {
    var allowed by mutableStateOf(granted())

    /** Rises each time the person asks for their position and may see it. */
    var follows by mutableIntStateOf(0)

    /** Android no longer asks: the app's settings page is the way on. */
    var settingsRequired by mutableStateOf(false)

    fun locate() {
        // Granted in the system settings since the page opened, perhaps.
        if (granted()) {
            allowed = true
            follows += 1
        } else {
            ask()
        }
    }
}

@Composable
private fun rememberPosition(): Position {
    val context = LocalContext.current
    val activity = LocalActivity.current
    val permissions = arrayOf(Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION)
    val holder = remember { arrayOfNulls<Position>(1) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
        val position = holder[0] ?: return@rememberLauncherForActivityResult
        if (result.values.any { it }) {
            position.allowed = true
            position.follows += 1
        } else if (activity == null || permissions.none { activity.shouldShowRequestPermissionRationale(it) }) {
            // Refused for good (or restricted): the request showed nothing.
            position.settingsRequired = true
        }
    }
    return remember {
        Position(
            granted = { permissions.any { context.checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED } },
            ask = { launcher.launch(permissions) },
        ).also { holder[0] = it }
    }
}

// Entry

/** Opens the map for a room string, or with none to browse it. Nothing
 * shows until Core can place the string (with none, until a map file is
 * stored). */
@Composable
fun RoomMapButton(room: String?, modifier: Modifier = Modifier, label: @Composable () -> Unit) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    var placed by remember(room) { mutableStateOf(false) }
    // Again once the map's file lands (the reference source's update) or the
    // data changes.
    LaunchedEffect(room, model.contentRevision, model.syncSource("references")?.lastUpdated) {
        placed = model.campusMapHas(room)
    }
    if (placed) {
        val spoken = if (room == null) t("campusMap.browse") else t("campusMap.open")
        Box(
            modifier
                .clickable(role = Role.Button) { navigator.openMap(room) }
                .semantics { contentDescription = spoken },
        ) { label() }
    }
}
