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
import android.location.LocationManager
import android.annotation.SuppressLint
import app.isctcompanion.core.CampusMapPoint
import kotlin.math.abs
import app.isctcompanion.design.LocalReducedMotion
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
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsFocusedAsState
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
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.input.TextFieldLineLimits
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.foundation.text.input.clearText
import androidx.compose.foundation.text.input.rememberTextFieldState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Cancel
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Map
import androidx.compose.material.icons.rounded.PhotoCamera
import androidx.compose.material.icons.rounded.MyLocation
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.BottomSheetScaffold
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SheetValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberBottomSheetScaffoldState
import androidx.compose.material3.rememberStandardBottomSheetState
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
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.layout.findRootCoordinates
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.res.painterResource
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
import app.isctcompanion.R
import app.isctcompanion.core.CampusMapCandidate
import app.isctcompanion.core.CampusMapCard
import app.isctcompanion.core.CampusMapIntent
import app.isctcompanion.core.CampusMapLevel
import app.isctcompanion.core.CampusMapCardKind
import app.isctcompanion.core.CampusMapEntranceRow
import app.isctcompanion.core.CampusMapPlace
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
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

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
                    .padding(horizontal = 8.dp, vertical = 2.dp),
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
                    MapLoad.Missing -> Notice(t("campusMap.unavailable"), t("campusMap.notDownloaded"))
                    is MapLoad.Failed -> Notice(t("campusMap.failed"), state.message) { load = MapLoad.Loading }
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
            AlertDialog(
                onDismissRequest = photoCamera.dismissFailure,
                confirmButton = { TextButton(onClick = photoCamera.dismissFailure) { Text(t("common.ok")) } },
                title = { Text(t("campusMap.photos.failed")) },
                text = { Text(message) },
            )
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

// Search bar and card

/** A capsule floating over the map, as iOS draws it: no inner field box. */
@Composable
private fun SearchBar(
    state: TextFieldState,
    interaction: MutableInteractionSource,
    enabled: Boolean,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier
            .defaultMinSize(minHeight = Control)
            .floating(CircleShape)
            .padding(horizontal = 14.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Icon(Icons.Rounded.Search, null, tint = Theme.colors.muted, modifier = Modifier.size(20.dp))
        BasicTextField(
            state = state,
            enabled = enabled,
            lineLimits = TextFieldLineLimits.SingleLine,
            textStyle = MaterialTheme.typography.bodyLarge.copy(color = Theme.colors.ink),
            cursorBrush = SolidColor(Theme.colors.accent),
            // Room codes are capitals and digits; no word is being spelt.
            keyboardOptions = KeyboardOptions(
                capitalization = KeyboardCapitalization.Characters, autoCorrectEnabled = false, imeAction = ImeAction.Search,
            ),
            interactionSource = interaction,
            modifier = Modifier.weight(1f),
            decorator = { field ->
                Box(contentAlignment = Alignment.CenterStart) {
                    if (state.text.isEmpty()) {
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
}

/** The place's name with the button that closes the card, under the
 * grabber: a tap on it steps the card between its lowest height and its
 * content's; a drag anywhere here moves the card. */
@Composable
private fun CardHeader(card: CampusMapCard, folded: Boolean, fold: () -> Unit, close: () -> Unit) {
    val spoken = t(if (folded) "campusMap.unfoldCard" else "campusMap.foldCard")
    Column {
        Box(
            Modifier
                .fillMaxWidth()
                .clickable(role = Role.Button, onClick = fold)
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
                card.title,
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
private fun CardDetails(
    card: CampusMapCard,
    openInMaps: () -> Unit,
    select: (CampusMapPlace) -> Unit,
    choose: (Int) -> Unit,
    photos: FloorPhotoUploads?,
    takePhoto: () -> Unit,
) {
    Column(
        Modifier.padding(start = Theme.cardPadding, end = Theme.cardPadding, bottom = Theme.cardPadding),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        // `建物 · 大岡山`, `西9号館 · 2F · 大岡山`.
        val subtitle = listOfNotNull(
            when (card.kind) {
                CampusMapCardKind.BUILDING -> t("campusMap.kind.building")
                CampusMapCardKind.ROOM -> card.buildingName.takeIf { it != card.title }
            },
            card.levelName,
            campusName(card.campus),
        )
        Text(
            subtitle.joinToString(" · "),
            style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted,
        )
        if (card.buildingOnly) {
            Text(t("campusMap.buildingOnly"), style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
        } else if (card.noFloorPlans) {
            Text(t("campusMap.noFloorPlans"), style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)
        }
        Actions(card, openInMaps, select, photos, takePhoto)
        if (card.entrances.isNotEmpty()) Entrances(card.entrances, select)
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

/** The building (on a room's card) and the maps app: the
 * card moves the map, and only this button leaves the app. */
@Composable
private fun Actions(
    card: CampusMapCard,
    openInMaps: () -> Unit,
    select: (CampusMapPlace) -> Unit,
    photos: FloorPhotoUploads?,
    takePhoto: () -> Unit,
) {
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        // Each tap opens the camera; the label tells how the photos went.
        if (photos != null) {
            ActionButton(
                when {
                    photos.sending > 0 -> t("campusMap.photos.sending")
                    photos.sent > 0 -> t("campusMap.photos.sent", "count" to photos.sent.toString())
                    else -> t("campusMap.photos.upload")
                },
                onClick = takePhoto,
                modifier = Modifier.weight(1f),
                level = ButtonLevel.Secondary,
                icon = Icons.Rounded.PhotoCamera,
            )
        }
        card.building?.let { building ->
            ActionButton(
                card.buildingName,
                onClick = { select(building) },
                modifier = Modifier.weight(1f)
                    .semantics { contentDescription = t("campusMap.showBuilding", "name" to card.buildingName) },
                level = ButtonLevel.Secondary,
            )
        }
        ActionButton(
            t("campusMap.openInMaps"),
            onClick = openInMaps,
            modifier = Modifier.weight(1f),
            level = ButtonLevel.Secondary,
            icon = Icons.Rounded.Map,
        )
    }
}

/** Each entrance lights on the map, on its floor, in this same card;
 * "Open in Maps" then pins it. */
@Composable
private fun Entrances(rows: List<CampusMapEntranceRow>, select: (CampusMapPlace) -> Unit) {
    Column {
        Text(
            t("campusMap.entrances"),
            style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.SemiBold, color = Theme.colors.muted,
        )
        rows.forEach { row ->
            Row(
                Modifier.fillMaxWidth().heightIn(min = 48.dp)
                    .clickable { select(CampusMapPlace.Entrance(row.entranceId)) },
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    row.name ?: t("campusMap.entrance"),
                    Modifier.weight(1f),
                    style = MaterialTheme.typography.bodyLarge,
                    fontWeight = if (row.selected) FontWeight.SemiBold else null,
                    color = if (row.selected) Theme.colors.accent else Theme.colors.ink,
                    maxLines = 1,
                )
                row.levelName?.let {
                    Text(it, style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted)
                }
            }
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
private fun Results(query: String, answer: Result<CampusMapSearch>?, pick: (CampusMapSearchResult) -> Unit) {
    val found = answer?.getOrNull()
    when {
        answer != null && found == null -> Notice(t("campusMap.failed"), answer.exceptionOrNull()?.message.orEmpty())
        found == null || found.query != query -> Unit
        found.results.isEmpty() -> Text(
            t("campusMap.noResults"),
            style = MaterialTheme.typography.bodyMedium, color = Theme.colors.muted,
            modifier = Modifier.padding(start = Theme.cardPadding, end = Theme.cardPadding, bottom = Theme.cardPadding),
        )
        // About three rows: the map stays in view above the keyboard.
        else -> LazyColumn(Modifier.heightIn(max = 200.dp)) {
            items(found.results) { result -> SearchRow(result) { pick(result) } }
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

/** Floors top floor first. A dot marks the floor the selected room is on,
 * the entrance icon the floor the building is entered on from the street. A
 * tall building scrolls within the space the page leaves, keeping the floor
 * shown in view. */
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
            val state = listOfNotNull(
                t("campusMap.destinationFloor").takeIf { level.id == marked },
                t("campusMap.streetFloor").takeIf { level.street },
            ).joinToString(", ")
            val mark = if (isSelected) Theme.colors.accentInk else Theme.colors.accent
            Box(
                Modifier
                    .size(Control)
                    .background(if (isSelected) Theme.colors.accent else Color.Transparent)
                    .clickable(role = Role.Button) { pick(level.id) }
                    .semantics {
                        contentDescription = spoken
                        stateDescription = state
                        this.selected = isSelected
                    },
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    level.name,
                    style = MaterialTheme.typography.bodySmall.tabular(), fontWeight = FontWeight.SemiBold,
                    color = if (isSelected) Theme.colors.accentInk else Theme.colors.ink,
                    maxLines = 1,
                )
                if (level.id == marked) {
                    Box(
                        Modifier
                            .align(Alignment.TopEnd)
                            .padding(5.dp)
                            .size(6.dp)
                            .clip(CircleShape)
                            .background(mark),
                    )
                }
                if (level.street) {
                    Icon(
                        painterResource(R.drawable.campus_map_entrance), null,
                        tint = mark,
                        modifier = Modifier.align(Alignment.BottomStart).padding(3.dp).size(11.dp),
                    )
                }
            }
        }
    }
}

// Position

/** The blue dot's permission, asked for only when the person taps the
 * position button. Each tap then reads the position once and hands it on:
 * Core picks the building the person is at. MapLibre draws the dot itself,
 * and there is no background location. */
private class Position(private val granted: () -> Boolean, private val ask: () -> Unit) {
    var allowed by mutableStateOf(granted())

    /** Android no longer asks: the app's settings page is the way on. */
    var settingsRequired by mutableStateOf(false)

    /** The position the next grant reads, for the tap that asked. */
    var pending: ((CampusMapPoint) -> Unit)? = null

    fun locate(context: Context, located: (CampusMapPoint) -> Unit) {
        // Granted in the system settings since the page opened, perhaps.
        if (granted()) {
            allowed = true
            readOnce(context, located)
        } else {
            pending = located
            ask()
        }
    }

    /** One fix from the system's fused provider (part of AOSP since
     * Android 12), else from GPS where a device ships without it. */
    @SuppressLint("MissingPermission")
    fun readOnce(context: Context, located: (CampusMapPoint) -> Unit) {
        val manager = context.getSystemService(LocationManager::class.java) ?: return
        val provider = if (manager.hasProvider(LocationManager.FUSED_PROVIDER)) LocationManager.FUSED_PROVIDER else LocationManager.GPS_PROVIDER
        runCatching {
            manager.getCurrentLocation(provider, null, context.mainExecutor) { fix ->
                if (fix != null) located(CampusMapPoint(latitude = fix.latitude, longitude = fix.longitude))
            }
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
        val located = position.pending
        position.pending = null
        if (result.values.any { it }) {
            position.allowed = true
            located?.let { position.readOnce(context, it) }
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
