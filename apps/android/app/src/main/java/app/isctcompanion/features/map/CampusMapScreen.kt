package app.isctcompanion.features.map

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.height
import androidx.compose.material.icons.rounded.GpsFixed
import kotlin.math.abs
import app.isctcompanion.design.NoticeBanner
import app.isctcompanion.design.LocalReducedMotion
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.input.clearText
import androidx.compose.foundation.text.input.rememberTextFieldState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
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
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.isctcompanion.LocalModel
import app.isctcompanion.LocalNavigator
import app.isctcompanion.core.SyncScope
import app.isctcompanion.core.CampusMapIntent
import app.isctcompanion.core.CampusMapOpening
import app.isctcompanion.core.CampusMapScene
import app.isctcompanion.core.CampusMapSearch
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.launch
import kotlin.math.roundToInt
import app.isctcompanion.core.campusMapOpen
import app.isctcompanion.core.campusMapSend
import app.isctcompanion.core.campusMapSearch
import app.isctcompanion.core.campusMapClose

/** The page's control size, the gap between controls and its side inset. */
internal val Control = 44.dp
private val ControlGap = Theme.space4
private val PageInset = Theme.space6

/** Where opening the map stands. */
private sealed interface MapLoad {
    data object Loading : MapLoad

    /** No map file is stored yet: it comes with the data updates. */
    data object Missing : MapLoad

    data class Failed(val message: String) : MapLoad

    /** Core's page as last drawn; the answer to each intent replaces it. */
    data class Ready(val scene: CampusMapScene) : MapLoad
}

/** The card's resting heights: about 150 dp, its content up to half the
 * window, or up to the search bar. */
private enum class CardDetent { Peek, Fit, Full }

private val PeekHeight = 150.dp

/** The campus map over OpenFreeMap's base map, full screen with a close
 * button, opened on a room string or on none to browse. The page is Core's
 * and its `apply` is the reducer: this screen draws Core's scene, forwards
 * what the person does and adds what the platform has, the blue dot. There
 * is no route: the card's one way out of the app opens the maps app the
 * settings name.
 *
 * As on iOS, the search bar runs along the top and its results drop under
 * it while typing. A selection's card is one level at the bottom, resting at
 * one of three heights, dragged by its top or stepped by its grabber; the
 * position button rides on its top edge. The map's visible part is what the
 * search bar, the card and the control rail leave. */
@Composable
fun CampusMapScreen(room: String?) {
    val model = LocalModel.current
    val navigator = LocalNavigator.current
    val context = LocalContext.current
    val density = LocalDensity.current
    val haptics = LocalHapticFeedback.current
    val focus = LocalFocusManager.current
    val scope = rememberCoroutineScope()
    val reduceMotion = LocalReducedMotion.current
    var load by remember { mutableStateOf<MapLoad>(MapLoad.Loading) }
    // The map is one of Core's reference files: asked for here, the page
    // looks again each time the data or a sync run changes, and clears the
    // downloading label once the sync run settles.
    var downloading by remember { mutableStateOf(false) }
    var sawSyncChange by remember { mutableStateOf(false) }
    var snapshotAtStart by remember { mutableStateOf<Any?>(null) }
    val syncBusy = model.syncSnapshot?.attention?.busy == true
    LaunchedEffect(model.contentRevision, model.syncSnapshot) {
        if (!downloading) return@LaunchedEffect
        if (syncBusy || model.syncSnapshot !== snapshotAtStart) sawSyncChange = true
        if (load == MapLoad.Missing) load = MapLoad.Loading
        if (sawSyncChange && !syncBusy) {
            downloading = false
            sawSyncChange = false
        }
    }
    // Intents reach Core one after another, in the order sent.
    val intents = remember { Channel<CampusMapIntent>(Channel.UNLIMITED) }
    val search = rememberTextFieldState()
    val photoCamera = rememberFloorPhotoCamera()
    val searchInteraction = remember { MutableInteractionSource() }
    val searchFocused by searchInteraction.collectIsFocusedAsState()
    // Layout only, in px: the search bar's bottom edge, the card's header
    // and content, and the card's shown height.
    var barBottom by remember { mutableIntStateOf(0) }
    var headerHeight by remember { mutableIntStateOf(0) }
    var contentHeight by remember { mutableIntStateOf(0) }
    var detent by remember { mutableStateOf(CardDetent.Fit) }
    val cardHeight = remember { Animatable(0f) }
    var dragging by remember { mutableStateOf(false) }
    val position = rememberPosition()

    val scene = (load as? MapLoad.Ready)?.scene
    val query = search.text.toString()
    val searching = searchFocused || query.isNotEmpty()
    val card = scene?.card
    val showsCard = card != null && !searching

    // Opens the page; a retry sets `Loading` and opens it again.
    if (load == MapLoad.Loading) {
        LaunchedEffect(Unit) {
            load = when (val opened = model.campusMapOpen(room)) {
                is CampusMapOpening.Ready -> MapLoad.Ready(opened.scene)
                CampusMapOpening.Missing -> MapLoad.Missing
                is CampusMapOpening.Failed -> MapLoad.Failed(opened.message)
            }
        }
    }
    val page = scene?.page
    LaunchedEffect(page) {
        if (page == null) return@LaunchedEffect
        for (intent in intents) {
            val drawnBefore = (load as? MapLoad.Ready)?.scene
            val next = model.campusMapSend(page, intent) ?: continue
            // Unless a newer scene is already drawn.
            val drawn = (load as? MapLoad.Ready)?.scene
            if (drawn != null && next.revision > drawn.revision) {
                load = MapLoad.Ready(next)
                // A place picked on the map, by search or by position ticks.
                val picked = intent is CampusMapIntent.Tap || intent is CampusMapIntent.Select || intent is CampusMapIntent.Locate
                if (picked && next.card != null && next.card != drawnBefore?.card) {
                    haptics.performHapticFeedback(HapticFeedbackType.ContextClick)
                }
            }
        }
    }
    // Also when left before the open finished: Core drops it.
    DisposableEffect(Unit) { onDispose { model.campusMapClose() } }

    fun send(intent: CampusMapIntent) {
        if (scene != null) intents.trySend(intent)
    }

    fun endSearch() {
        focus.clearFocus()
        search.clearText()
    }

    // A place picked on the map while searching ends the search.
    LaunchedEffect(card) { if (card != null && searching) endSearch() }
    // Each new place opens at its content's height.
    LaunchedEffect(card?.title) { detent = CardDetent.Fit }
    // Core's places for the text; the map does not move while they type.
    val answer by produceState<Result<CampusMapSearch>?>(null, query) {
        value = null
        if (query.isNotBlank()) value = model.campusMapSearch(query)
    }
    // Back closes the open place first, then the page.
    BackHandler(enabled = showsCard) { send(CampusMapIntent.Clear) }
    BackHandler(enabled = searching) { endSearch() }

    BoxWithConstraints(Modifier.fillMaxSize().background(Theme.colors.bg)) {
        val window = constraints.maxHeight.toFloat()
        val statusBar = WindowInsets.statusBars.getTop(density)
        val keyboard = WindowInsets.ime.getBottom(density)
        val navigationBar = WindowInsets.navigationBars.getBottom(density)
        val gap = with(density) { ControlGap.toPx() }
        val control = with(density) { Control.toPx() }
        // The card's resting heights, in px.
        val content = (headerHeight + contentHeight).toFloat()
        val peek = minOf(content, maxOf(headerHeight.toFloat(), with(density) { PeekHeight.toPx() }))
        val fit = maxOf(peek, minOf(content, window * 0.5f))
        val full = maxOf(fit, minOf(content, window - barBottom - navigationBar - 3 * gap))
        val heights = mapOf(CardDetent.Peek to peek, CardDetent.Fit to fit, CardDetent.Full to full)
        val resting = heights.getValue(detent)
        LaunchedEffect(resting, showsCard) {
            if (!dragging && showsCard) {
                if (reduceMotion || cardHeight.value == 0f) cardHeight.snapTo(resting) else cardHeight.animateTo(resting, spring(stiffness = Spring.StiffnessMediumLow))
            }
        }
        val shownCard = if (showsCard) cardHeight.value else 0f
        // Above the card, or above the navigation bar when there is none.
        val bottomEdge = navigationBar + gap + (if (showsCard) shownCard + gap else 0f)
        val chrome = MapInsets(
            left = 0,
            top = (barBottom + gap).roundToInt(),
            right = (with(density) { PageInset.toPx() } + control).roundToInt(),
            bottom = (bottomEdge + control + gap).roundToInt(),
        )

        FloorMapView(
            scene = scene,
            chrome = chrome,
            insets = { chrome },
            canFrame = keyboard == 0 && !dragging,
            showsPosition = position.allowed,
            onCamera = { send(CampusMapIntent.Camera(it)) },
            // Core ignores a tap on a floor it has already left.
            onTap = { point, level, metresPerPoint, roomsOnly -> send(CampusMapIntent.Tap(point, level, metresPerPoint, roomsOnly)) },
            onSelect = { send(CampusMapIntent.Select(it)) },
            modifier = Modifier.fillMaxSize(),
        )

        // The control rail: MapLibre's compass heads it, then the floors,
        // which scroll within what the card and the position button leave.
        if (scene != null && scene.levels.isNotEmpty()) {
            Column(
                Modifier
                    .align(Alignment.TopEnd)
                    .padding(
                        top = with(density) { (barBottom + gap + control + gap).toDp() },
                        end = PageInset,
                        bottom = with(density) { (bottomEdge + control + gap).toDp() },
                    )
                    .width(Control),
            ) {
                LevelPicker(scene.levels, scene.levelId, scene.markedLevelId, Modifier.weight(1f, fill = false)) {
                    haptics.performHapticFeedback(HapticFeedbackType.SegmentTick)
                    send(CampusMapIntent.Level(it))
                }
            }
        }

        // The position button on the card's top edge, riding along with it.
        val spoken = t("campusMap.position")
        Box(
            Modifier
                .align(Alignment.BottomEnd)
                .padding(end = PageInset, bottom = with(density) { bottomEdge.toDp() })
                .size(Control)
                .floating(CircleShape)
                .clickable(role = Role.Button) { position.locate(context) { send(CampusMapIntent.Locate(it)) } }
                .semantics { contentDescription = spoken },
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                Icons.Rounded.GpsFixed, null,
                tint = if (position.allowed) Theme.colors.accent else Theme.colors.ink,
                modifier = Modifier.size(20.dp),
            )
        }
        val attribution = scene?.attribution.orEmpty()
        if (attribution.isNotEmpty() && !searching) {
            Text(
                attribution.joinToString(" · "),
                style = MaterialTheme.typography.labelSmall, color = Theme.colors.muted,
                modifier = Modifier
                    .align(Alignment.BottomStart)
                    .padding(start = PageInset, bottom = with(density) { bottomEdge.toDp() })
                    .floating(CircleShape, elevation = 2)
                    .padding(horizontal = Theme.space4, vertical = Theme.space1),
            )
        }

        // The card, or what stands in for it while the map cannot be drawn.
        val panelShape = RoundedCornerShape(Theme.radiusPanel)
        if (showsCard && card != null) {
            Column(
                Modifier
                    .align(Alignment.BottomCenter)
                    .padding(horizontal = PageInset)
                    .padding(bottom = with(density) { (navigationBar + gap).toDp() })
                    .fillMaxWidth()
                    .height(with(density) { maxOf(shownCard, control).toDp() })
                    .floating(panelShape, elevation = 8),
            ) {
                Column(
                    Modifier
                        .onSizeChanged { headerHeight = it.height }
                        .draggable(
                            orientation = Orientation.Vertical,
                            state = rememberDraggableState { delta ->
                                // Past either end the card follows the finger only a little.
                                val next = cardHeight.value - delta
                                val eased = when {
                                    next > full -> cardHeight.value - delta * 0.2f
                                    next < peek -> cardHeight.value - delta * 0.2f
                                    else -> next
                                }
                                scope.launch { cardHeight.snapTo(eased) }
                            },
                            onDragStarted = { dragging = true },
                            onDragStopped = { velocity ->
                                // Where the throw would carry it, as a scroll view coasts.
                                val projected = cardHeight.value - velocity * 0.2f
                                val nearest = heights.minBy { abs(it.value - projected) }.key
                                dragging = false
                                detent = nearest
                                val target = heights.getValue(nearest)
                                if (reduceMotion) cardHeight.snapTo(target) else cardHeight.animateTo(target, spring(stiffness = Spring.StiffnessMediumLow))
                            },
                        ),
                ) {
                    CardHeader(
                        card, folded = detent == CardDetent.Peek,
                        fold = { detent = if (detent == CardDetent.Peek) CardDetent.Fit else CardDetent.Peek },
                        close = { send(CampusMapIntent.Clear) },
                    )
                }
                Column(Modifier.verticalScroll(rememberScrollState())) {
                    Box(Modifier.onSizeChanged { contentHeight = it.height }) {
                        CardDetails(
                            card,
                            openInMaps = { openInMaps(context, card.mapsUrl) },
                            select = { send(CampusMapIntent.Select(it)) },
                            choose = { send(CampusMapIntent.Choose(it.toUInt())) },
                            photos = card.photoBuildingId?.takeIf { photoCamera.available }?.let {
                                photoCamera.uploads[it] ?: FloorPhotoUploads()
                            },
                            takePhoto = { card.photoBuildingId?.let(photoCamera.take) },
                        )
                    }
                }
            }
        } else if (load !is MapLoad.Ready && !searching) {
            Box(
                Modifier
                    .align(Alignment.BottomCenter)
                    .padding(horizontal = PageInset)
                    .padding(bottom = with(density) { (navigationBar + gap).toDp() })
                    .fillMaxWidth()
                    .floating(panelShape, elevation = 8),
            ) {
                when (val state = load) {
                    MapLoad.Loading -> Box(Modifier.fillMaxWidth().padding(Theme.cardPadding), Alignment.Center) {
                        CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                    }
                    MapLoad.Missing -> Notice(
                        t("campusMap.unavailable"), t(if (downloading) "common.downloading" else "campusMap.notDownloaded"),
                        t("campusMap.download"),
                    ) {
                        downloading = true
                        sawSyncChange = false
                        snapshotAtStart = model.syncSnapshot
                        model.refresh(SyncScope.References)
                    }
                    is MapLoad.Failed -> Notice(t("campusMap.failed"), state.message, t("common.retry")) { load = MapLoad.Loading }
                    is MapLoad.Ready -> Unit
                }
            }
        }

        // The search bar, and its results while typing.
        Column(
            Modifier
                .align(Alignment.TopStart)
                .fillMaxWidth()
                .windowInsetsPadding(WindowInsets.statusBars)
                .padding(horizontal = PageInset)
                .padding(top = ControlGap / 2),
            verticalArrangement = Arrangement.spacedBy(ControlGap),
        ) {
            Row(
                Modifier.onGloballyPositioned { barBottom = (it.positionInRoot().y + it.size.height).roundToInt() },
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(ControlGap),
            ) {
                Box(
                    Modifier
                        .size(Control)
                        .floating(CircleShape)
                        .clickable(onClickLabel = t("common.close"), role = Role.Button) { navigator.back() },
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(Icons.Rounded.Close, t("common.close"), tint = Theme.colors.ink)
                }
                SearchBar(search, searchInteraction, enabled = scene != null, Modifier.weight(1f))
                if (searching) TextButton(onClick = ::endSearch) { Text(t("common.cancel")) }
            }
            if (searching && answer != null) {
                Box(
                    Modifier
                        .fillMaxWidth()
                        .heightIn(max = with(density) { (window * 0.5f - statusBar).coerceAtLeast(control).toDp() })
                        .floating(panelShape, elevation = 8),
                ) {
                    Column(Modifier.verticalScroll(rememberScrollState())) {
                        // A second tap on a result only selects it again.
                        Results(query, answer) { result ->
                            send(CampusMapIntent.Select(result.place))
                            endSearch()
                        }
                    }
                }
            }
        }
        photoCamera.failure?.let { message ->
            NoticeBanner(Theme.colors.dangerSoft, Modifier.align(Alignment.BottomCenter).padding(Theme.gutter)) {
                Text(t("campusMap.photos.failed"), style = MaterialTheme.typography.titleSmall)
                Text(message, color = Theme.colors.ink)
                TextButton(onClick = photoCamera.dismissFailure) { Text(t("common.close")) }
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
internal fun Modifier.floating(shape: Shape, elevation: Int = 6): Modifier =
    shadow(elevation.dp, shape).clip(shape).background(Theme.colors.surface)

/** Core's link is a plain https link to the maps app the settings name; the
 * app opens it when installed, else the browser does. */
private fun openInMaps(context: Context, url: String) {
    runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
}
