package app.isctcompanion.features.map

import android.annotation.SuppressLint
import android.content.ComponentCallbacks2
import android.content.res.Configuration
import android.graphics.Paint
import android.graphics.PointF
import android.graphics.RectF
import androidx.compose.foundation.Canvas as Drawing
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.withFrameNanos
import androidx.compose.material3.Icon
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import app.isctcompanion.core.CampusMapBounds
import app.isctcompanion.core.CampusMapCamera
import app.isctcompanion.core.CampusMapFraming
import app.isctcompanion.core.CampusMapMark
import app.isctcompanion.core.CampusMapMarkKind
import app.isctcompanion.core.CampusMapPlace
import app.isctcompanion.core.CampusMapPoint
import app.isctcompanion.core.CampusMapPolygon
import app.isctcompanion.core.CampusMapScene
import app.isctcompanion.core.CampusMapStackEntrance
import app.isctcompanion.core.CampusMapStackFloor
import app.isctcompanion.R
import app.isctcompanion.design.LocalReducedMotion
import app.isctcompanion.design.Palette
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import kotlinx.coroutines.flow.first
import org.maplibre.android.MapLibre
import org.maplibre.android.camera.CameraUpdateFactory
import org.maplibre.android.geometry.LatLng
import org.maplibre.android.geometry.LatLngBounds
import org.maplibre.android.location.LocationComponentActivationOptions
import org.maplibre.android.location.modes.CameraMode
import org.maplibre.android.location.modes.RenderMode
import org.maplibre.android.maps.MapLibreMap
import org.maplibre.android.maps.MapView
import org.maplibre.android.maps.Style
import kotlin.math.cos
import kotlin.math.roundToInt
import kotlin.math.sin

/** What the page's own controls cover at each edge of the map, in pixels:
 * the close button, the control rail, the panel. What is left is the part
 * of the map the person can see. */
data class MapInsets(val left: Int, val top: Int, val right: Int, val bottom: Int)

/** OpenFreeMap's base map (OpenStreetMap data, no key) with Core's scene
 * drawn over it by role. Core decides every layer and mark; this view only
 * paints them with MapLibre (`SceneStyle`), moves the camera where Core
 * frames, and reports the person's own moves of it over the part of the map
 * the controls leave visible.
 *
 * The stack (this fork) is drawn over the map, not in it, so it needs
 * nothing of the map engine but its ground projection (as MapKit on iOS):
 * each point at a height is projected through the ground point on the same
 * line of sight ([Sight]). The floors are outlines, the floor picked with
 * its plan; the building's footprint at the street floor is the ground,
 * the floors under the street below it, seen through it, and the selected
 * room is a block in the accent over everything. The entrances keep the
 * flat map's icons, at the floor picked's height; every floor with a way in
 * is named so. */
@Composable
fun FloorMapView(
    scene: CampusMapScene?,
    /** The controls as last measured; a change moves the map's own buttons. */
    chrome: MapInsets,
    /** The same, read when a framing is applied or the camera reported: by
     * then the panel a new scene brought has been measured. */
    insets: () -> MapInsets,
    /** False while the keyboard is up: a framing waits for the panel to
     * come to rest, since it frames the part the panel leaves visible. */
    canFrame: Boolean,
    showsPosition: Boolean,
    /** Where the camera settled after the person moved it. */
    onCamera: (CampusMapCamera) -> Unit,
    /** A tap off the marks, the floor drawn under it and the map's metres
     * per dp there. */
    onTap: (CampusMapPoint, String?, Double, Boolean) -> Unit,
    onSelect: (CampusMapPlace) -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val palette = Theme.colors
    val dark = isSystemInDarkTheme()
    val animates = !LocalReducedMotion.current
    val view = remember {
        // MapLibre is initialised once before its first view.
        MapLibre.getInstance(context)
        MapView(context)
    }
    val painter = remember { FloorPainter(view) }
    var placed by remember { mutableStateOf(emptyList<PlacedMark>()) }
    var raised by remember { mutableStateOf(emptyList<RaisedEntrance>()) }
    var rooms by remember { mutableStateOf(emptyList<RaisedRoom>()) }
    var stack by remember { mutableStateOf<StackPicture?>(null) }
    SideEffect {
        painter.insets = insets
        painter.onCamera = onCamera
        painter.onTap = onTap
        painter.onSelect = onSelect
        painter.onPlaced = { placed = it }
        painter.onRaised = { raised = it }
        painter.onRooms = { rooms = it }
        painter.onStack = { stack = it }
    }
    MapLifecycle(view)
    DisposableEffect(view) {
        view.getMapAsync { painter.attach(it) }
        onDispose { painter.detach() }
    }
    // Each of Core's framings moves the camera once, when the map has a size
    // and the panel is at rest. Core never hears it landed.
    val framing = scene?.framing
    val frames by rememberUpdatedState(canFrame)
    val animated by rememberUpdatedState(animates)
    LaunchedEffect(framing?.key) {
        if (framing == null) return@LaunchedEffect
        snapshotFlow { frames && painter.hasRoom() }.first { it }
        // After this frame's layout, so the insets include the panel this
        // scene brought.
        withFrameNanos {}
        painter.frame(framing, animated)
    }
    Box(modifier) {
        AndroidView(
            factory = { view },
            modifier = Modifier.fillMaxSize(),
            update = { painter.show(scene, palette, dark, chrome, showsPosition) },
        )
        SpokenMarks(placed, onSelect)
        StackDrawing(stack)
        RaisedRooms(rooms)
        RaisedEntrances(raised, onSelect)
    }
}

/** One floor of the stack on screen: the outer ring of each of its shapes
 * at the floor's height, in the map's pixels. Only the floor picked brings
 * its plan; the others are their outline. */
private data class FloorPicture(
    val name: String,
    val outline: List<List<Offset>>,
    val walkways: List<List<Offset>>,
    val rooms: List<List<Offset>>,
    val shown: Boolean,
    val street: Boolean,
    /** The building has an entrance on this floor: its name says so. */
    val entered: Boolean,
    /** False for a floor between two with plans: only where it is. */
    val plan: Boolean,
    /** Below the street floor, under the ground. */
    val under: Boolean,
    /** Below the floor picked. */
    val below: Boolean,
    /** Where its name goes: by its outline's right-most corner. */
    val label: Offset?,
)

/** The stack as drawn: the floors under the street, the ground (the
 * building's footprint) at the street floor, then the street floor and those above,
 * each lowest first, so a higher floor lies over a lower one. */
private data class StackPicture(
    val under: List<FloorPicture>,
    val ground: List<List<Offset>>,
    val over: List<FloorPicture>,
    /** A floor under the street is picked: the ground is seen through. */
    val seeThrough: Boolean,
    /** The building's sides: an upright from its lowest floor to its
     * highest at each end of its outline as seen. */
    val sides: List<Pair<Offset, Offset>>,
)

/** The stack, drawn over the map. Floors are flat outlines at their
 * heights: the floor picked solid with its corridors and rooms, those
 * below it faint, those above it bare outlines, and a floor without a plan
 * or under the street dashed. The ground is a plate at the street floor
 * over the floors under it, so they read as under the ground; while one of
 * them is picked the plate thins and that floor's plan is laid over it.
 * Each floor is named at its right, with "entrance" on every floor that
 * has one, the street floor's in bold. */
@Composable
private fun StackDrawing(picture: StackPicture?) {
    if (picture == null) return
    val colors = Theme.colors
    val entrance = t("campusMap.entrance")
    Drawing(Modifier.fillMaxSize()) {
        fun ring(points: List<Offset>) = Path().apply {
            moveTo(points[0].x, points[0].y)
            for (point in points.drop(1)) lineTo(point.x, point.y)
            close()
        }
        val hairline = 1.dp.toPx()
        val dashed = PathEffect.dashPathEffect(floatArrayOf(4.dp.toPx(), 3.dp.toPx()))
        fun floor(floor: FloorPicture) {
            // Floors above the one picked are outlines alone: a fill would
            // veil the plan under them.
            val fill = when {
                !floor.plan -> 0f
                floor.shown && floor.under -> 0.85f
                floor.shown -> 0.95f
                floor.below -> 0.35f
                else -> 0f
            }
            val edge = if (floor.shown) colors.ink else colors.muted
            val effect = if (!floor.plan || (floor.under && !floor.shown)) dashed else null
            for (shape in floor.outline) {
                val path = ring(shape)
                if (fill > 0f) drawPath(path, colors.surface.copy(alpha = fill))
                drawPath(
                    path,
                    edge.copy(alpha = if (floor.shown) 1f else 0.7f),
                    style = Stroke(width = if (floor.shown) 1.5f * hairline else hairline, pathEffect = effect),
                )
            }
            for (shape in floor.walkways) drawPath(ring(shape), colors.accentSoft)
            for (shape in floor.rooms) {
                val path = ring(shape)
                drawPath(path, colors.surface2)
                drawPath(path, colors.line, style = Stroke(width = hairline))
            }
        }
        for (side in picture.sides) {
            drawLine(colors.muted.copy(alpha = 0.5f), side.first, side.second, strokeWidth = hairline)
        }
        picture.under.filter { !it.shown }.forEach(::floor)
        for (shape in picture.ground) {
            val path = ring(shape)
            drawPath(path, colors.bg.copy(alpha = if (picture.seeThrough) SEE_THROUGH_GROUND else GROUND))
            drawPath(path, colors.muted, style = Stroke(width = 1.5f * hairline))
        }
        // The floor picked under the street is read through the ground: its
        // plan over the plate, lower down than the plate's own edge.
        picture.under.filter { it.shown }.forEach(::floor)
        picture.over.forEach(::floor)
        val text = Paint(Paint.ANTI_ALIAS_FLAG).apply { textSize = 12.dp.toPx() }
        for (floor in picture.under + picture.over) {
            val at = floor.label ?: continue
            text.color = when {
                floor.shown -> colors.accent
                floor.plan -> colors.ink
                else -> colors.muted
            }.toArgb()
            text.isFakeBoldText = floor.shown || floor.street
            val name = if (floor.entered) "${floor.name} · $entrance" else floor.name
            drawContext.canvas.nativeCanvas.drawText(name, at.x + 6.dp.toPx(), at.y + 4.dp.toPx(), text)
        }
    }
}

/** The selected room on one floor of the stack, in screen pixels: each
 * outer ring at its floor and at the room's top, the same points lifted. */
private data class RaisedRoom(val floors: List<List<Offset>>, val tops: List<List<Offset>>, val shown: Boolean)

/** The selected room, drawn over the map rather than in it. MapLibre's
 * extrusions share one depth buffer, so a floor in front hides a room
 * behind it whatever the layer order; laid over the map, the room shows
 * where it sits in the building through every floor. Each room is one
 * layer, so its faces overlap without darkening, at full strength on the
 * floor picked and at [OTHER_FLOOR_ROOM_OPACITY] on any other. */
@Composable
private fun RaisedRooms(rooms: List<RaisedRoom>) {
    val accent = Theme.colors.accent
    val edge = Theme.colors.surface
    for (room in rooms) {
        Drawing(
            Modifier
                .fillMaxSize()
                .graphicsLayer {
                    alpha = if (room.shown) SHOWN_ROOM_OPACITY else OTHER_FLOOR_ROOM_OPACITY
                    compositingStrategy = CompositingStrategy.Offscreen
                },
        ) {
            fun ring(points: List<Offset>) = Path().apply {
                moveTo(points[0].x, points[0].y)
                for (point in points.drop(1)) lineTo(point.x, point.y)
                close()
            }
            for ((floor, top) in room.floors.zip(room.tops)) {
                drawPath(ring(floor), accent)
                for (i in floor.indices) {
                    val j = (i + 1) % floor.size
                    drawPath(ring(listOf(floor[i], floor[j], top[j], top[i])), accent)
                }
                val roof = ring(top)
                drawPath(roof, accent)
                drawPath(roof, edge, style = Stroke(width = 1.5.dp.toPx()))
            }
        }
    }
}

/** An entrance in the stack and where its floor puts it on screen. */
private data class RaisedEntrance(val id: String, val name: String?, val selected: Boolean, val x: Int, val y: Int)

/** The stack's entrances, as the flat map draws them: MapLibre's icons lie
 * on the ground, so these are the same disc and icon laid over the map at
 * the height of the floor picked, the only floor Core gives entrances to.
 * A tap picks the entrance as on the flat map, and a screen reader reads
 * its name. */
@Composable
private fun RaisedEntrances(raised: List<RaisedEntrance>, onSelect: (CampusMapPlace) -> Unit) {
    val half = with(LocalDensity.current) { (SPOKEN_DP.dp / 2).roundToPx() }
    val colors = Theme.colors
    for (entrance in raised) {
        key(entrance.id) {
            val spoken = entrance.name ?: t("campusMap.entrance")
            Box(
                Modifier
                    .offset { IntOffset(entrance.x - half, entrance.y - half) }
                    .size(SPOKEN_DP.dp)
                    .clip(CircleShape)
                    .background(if (entrance.selected) colors.accent else colors.surface)
                    .border(1.dp, colors.line, CircleShape)
                    .clickable(role = Role.Button) { onSelect(CampusMapPlace.Entrance(entrance.id)) }
                    .semantics {
                        contentDescription = spoken
                        selected = entrance.selected
                    },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    painterResource(R.drawable.campus_map_entrance), null,
                    tint = if (entrance.selected) colors.accentInk else colors.accent,
                    modifier = Modifier.size((SPOKEN_DP - 10).dp),
                )
            }
        }
    }
}


/** A mark and where it is on screen, in the map's pixels. */
private data class PlacedMark(val mark: CampusMapMark, val x: Int, val y: Int)

/** The marks for a screen reader. MapLibre paints them into the map's one
 * surface, which says nothing; these are unpainted, untouchable boxes over
 * each mark in view that carry its name and, for a room, a building or an entrance, the
 * same pick a tap makes. A finger still reaches the map through them. */
@Composable
private fun SpokenMarks(placed: List<PlacedMark>, onSelect: (CampusMapPlace) -> Unit) {
    val half = with(LocalDensity.current) { (SPOKEN_DP.dp / 2).roundToPx() }
    for (item in placed) {
        key(item.mark.id) {
            val kind = item.mark.kind
            val spoken = when (kind) {
                is CampusMapMarkKind.Room -> kind.label
                is CampusMapMarkKind.Building -> kind.label
                is CampusMapMarkKind.Amenity -> t("campusMap.amenity.${kind.key}")
                is CampusMapMarkKind.Entrance -> kind.name ?: t("campusMap.entrance")
            }
            val place = when (kind) {
                is CampusMapMarkKind.Room -> CampusMapPlace.Unit(kind.unitId)
                is CampusMapMarkKind.Building -> CampusMapPlace.Building(kind.buildingId)
                is CampusMapMarkKind.Entrance -> CampusMapPlace.Entrance(kind.entranceId)
                is CampusMapMarkKind.Amenity -> null
            }
            val chosen = when (kind) {
                is CampusMapMarkKind.Room -> kind.highlighted
                is CampusMapMarkKind.Building -> false
                is CampusMapMarkKind.Entrance -> kind.selected
                is CampusMapMarkKind.Amenity -> false
            }
            Box(
                Modifier
                    .offset { IntOffset(item.x - half, item.y - half) }
                    .size(SPOKEN_DP.dp)
                    .semantics {
                        contentDescription = spoken
                        if (place != null) {
                            role = Role.Button
                            selected = chosen
                            onClick {
                                onSelect(place)
                                true
                            }
                        }
                    },
            )
        }
    }
}

/** The side of a mark's box for a screen reader, the size of its icon. */
private const val SPOKEN_DP = 28
/** The selected room on the floor picked. */
private const val SHOWN_ROOM_OPACITY = 0.9f
/** The selected room on a floor other than the one picked, above or below
 * it: one value for both. */
private const val OTHER_FLOOR_ROOM_OPACITY = 0.5f
/** The ground plate at the street floor, and while a floor under it is
 * picked. */
private const val GROUND = 0.8f
private const val SEE_THROUGH_GROUND = 0.35f

/** A `MapView` is told each lifecycle step by its host; this one lives as
 * long as the map page's destination. */
@Composable
private fun MapLifecycle(view: MapView) {
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val context = LocalContext.current.applicationContext
    DisposableEffect(lifecycle, view) {
        var started = false
        var resumed = false
        view.onCreate(null)
        // A new observer is walked up to the current state, so a map opened
        // on a resumed screen starts and resumes at once.
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_START -> {
                    view.onStart()
                    started = true
                }
                Lifecycle.Event.ON_RESUME -> {
                    view.onResume()
                    resumed = true
                }
                Lifecycle.Event.ON_PAUSE -> {
                    view.onPause()
                    resumed = false
                }
                Lifecycle.Event.ON_STOP -> {
                    view.onStop()
                    started = false
                }
                else -> Unit
            }
        }
        lifecycle.addObserver(observer)
        // MapLibre asks its host to pass on the system's memory warnings, so
        // it can drop what it caches. Since Android 14 they arrive as a trim
        // once the app is in the background.
        val memory = object : ComponentCallbacks2 {
            override fun onConfigurationChanged(newConfig: Configuration) = Unit

            @Suppress("OVERRIDE_DEPRECATION")
            override fun onLowMemory() = view.onLowMemory()

            override fun onTrimMemory(level: Int) {
                if (level >= ComponentCallbacks2.TRIM_MEMORY_BACKGROUND) view.onLowMemory()
            }
        }
        context.registerComponentCallbacks(memory)
        onDispose {
            context.unregisterComponentCallbacks(memory)
            lifecycle.removeObserver(observer)
            if (resumed) view.onPause()
            if (started) view.onStop()
            view.onDestroy()
        }
    }
}

/** One MapLibre map: draws scenes on it (`SceneStyle`), moves its camera to
 * Core's framings and reports what the person does on it. */
private class FloorPainter(private val view: MapView) {
    var insets: () -> MapInsets = { MapInsets(0, 0, 0, 0) }
    var onCamera: (CampusMapCamera) -> Unit = {}
    var onTap: (CampusMapPoint, String?, Double, Boolean) -> Unit = { _, _, _, _ -> }
    var onSelect: (CampusMapPlace) -> Unit = {}
    /** The marks in the visible part and where they are, for a screen reader. */
    var onPlaced: (List<PlacedMark>) -> Unit = {}
    /** The stack's entrances in view and where their floors put them. */
    var onRaised: (List<RaisedEntrance>) -> Unit = {}
    var onRooms: (List<RaisedRoom>) -> Unit = {}
    var onStack: (StackPicture?) -> Unit = {}

    private val context = view.context
    private val density = context.resources.displayMetrics.density
    private var map by mutableStateOf<MapLibreMap?>(null)
    /** The view's size, which a framing needs. */
    private var size by mutableStateOf(IntSize.Zero)
    private var scene: CampusMapScene? = null
    private var palette: Palette? = null
    private var dark = false
    private var showsPosition = false
    private var chrome = MapInsets(0, 0, 0, 0)
    /** The camera's current move is the person's own gesture. */
    private var personMoving = false
    /** Where on the ground the centre of the view was when the person's
     * move began: in the stack, a turn or tilt that leaves it in place is no
     * move to another building. */
    private var movedFrom: LatLng? = null
    /** A first framing places the camera; later ones animate. */
    private var framed = false
    /** The theme the style was asked for, and the scene's content in the
     * style once it is loaded. */
    private var appearance: Pair<Palette, Boolean>? = null
    private var scenery: SceneStyle? = null
    /** The style the blue dot was last attached to. */
    private var positionStyle: Style? = null
    /** Whether the camera is tilted for the stack. */
    private var tilted = false

    init {
        view.addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ -> size = IntSize(view.width, view.height) }
    }

    fun attach(map: MapLibreMap) {
        this.map = map
        // No tilt, as on iOS. The attribution button stays: it credits
        // OpenFreeMap, OpenMapTiles and OpenStreetMap.
        map.uiSettings.isTiltGesturesEnabled = false
        map.uiSettings.isLogoEnabled = false
        map.addOnCameraMoveStartedListener { reason ->
            personMoving = reason == MapLibreMap.OnCameraMoveStartedListener.REASON_API_GESTURE
            if (personMoving) movedFrom = map.cameraPosition.target
        }
        // The stack follows the camera as it moves.
        map.addOnCameraMoveListener { raise() }
        map.addOnCameraIdleListener {
            if (personMoving && (!tilted || panned(map))) report(map)
            personMoving = false
            place()
        }
        // A double tap zooms and never arrives here; it selects nothing. So
        // every tap here is the full one (`roomsOnly` false).
        map.addOnMapClickListener { at ->
            val point = map.projection.toScreenLocation(at)
            val reach = TOUCH_DP * density
            val touched = RectF(point.x - reach, point.y - reach, point.x + reach, point.y + reach)
            // The topmost mark under the finger, as drawn.
            val mark = map.queryRenderedFeatures(touched, *SceneStyle.TAPPABLE).firstOrNull()
            val entrance = mark?.takeIf { it.hasNonNullValueForProperty("entrance") }?.getStringProperty("entrance")
            val unit = mark?.takeIf { it.hasNonNullValueForProperty("unit") }?.getStringProperty("unit")
            val building = mark?.takeIf { it.hasNonNullValueForProperty("building") }?.getStringProperty("building")
            when {
                entrance != null -> onSelect(CampusMapPlace.Entrance(entrance))
                unit != null -> onSelect(CampusMapPlace.Unit(unit))
                building != null -> onSelect(CampusMapPlace.Building(building))
                // An amenity is not a place to pick.
                mark != null -> Unit
                else -> {
                    // The map's scale here, over a few dp across the tap.
                    val across = map.projection.fromScreenLocation(PointF(point.x + SCALE_DP * density, point.y))
                    onTap(CampusMapPoint(latitude = at.latitude, longitude = at.longitude), scene?.levelId, at.distanceTo(across) / SCALE_DP, false)
                }
            }
            true
        }
        render()
    }

    /** The view is destroyed with the page; nothing drawn outlives it. */
    fun detach() {
        map = null
        scenery = null
    }

    fun show(
        scene: CampusMapScene?,
        palette: Palette,
        dark: Boolean,
        chrome: MapInsets,
        showsPosition: Boolean,
    ) {
        this.scene = scene
        this.palette = palette
        this.dark = dark
        this.showsPosition = showsPosition
        this.chrome = chrome
        render()
    }

    /** MapLibre's compass and attribution button sit clear of the page's
     * controls: the compass heads the rail, the button rests on the panel. */
    private fun layOut(map: MapLibreMap) {
        val gap = (CONTROL_GAP_DP * density).roundToInt()
        val inset = (PAGE_INSET_DP * density).roundToInt()
        map.uiSettings.setCompassMargins(0, chrome.top - (CONTROL_DP * density).roundToInt(), inset, 0)
        map.uiSettings.setAttributionMargins(inset, 0, 0, chrome.bottom + gap)
    }

    private fun render() {
        val map = map ?: return
        val palette = palette ?: return
        layOut(map)
        tilt(map, scene?.stacked == true)
        val next = palette to dark
        if (next != appearance) {
            // Theme colours are layer content too; a rare theme change loads
            // the matching base map and draws everything again.
            appearance = next
            scenery = null
            map.setStyle(Style.Builder().fromUri(if (dark) DARK_STYLE else LIGHT_STYLE)) { loaded ->
                if (appearance == next && this.map != null) {
                    scenery = SceneStyle(context, loaded, palette)
                    render()
                }
            }
            return
        }
        val scenery = scenery ?: return
        showPosition(map, scenery.style)
        if (scenery.draw(scene?.layers.orEmpty(), scene?.marks.orEmpty())) place()
        raise()
    }

    /** Draws the stack for the camera as it is now: the floors, the ground
     * plate, the selected room and the entrances, each point through
     * [Sight]. Core gives entrances only to the floor picked. */
    private fun raise() {
        val map = map ?: return
        val current = scene
        if (current == null || !current.stacked || view.width == 0 || view.height == 0) {
            onRaised(emptyList())
            onRooms(emptyList())
            onStack(null)
            return
        }
        val sight = Sight(map, view.width, view.height)
        fun rings(polygons: List<CampusMapPolygon>, height: Double): List<List<Offset>> =
            polygons.mapNotNull { shape ->
                val ring = shape.rings.firstOrNull()?.takeIf { it.size >= 3 } ?: return@mapNotNull null
                ring.map { point -> sight.at(point, height) ?: return@mapNotNull null }
            }
        val shownBase = current.stack.firstOrNull { it.shown }?.baseM ?: 0.0
        val floors = current.stack.map { floor ->
            val outline = rings(floor.outline, floor.baseM)
            FloorPicture(
                name = floor.name,
                outline = outline,
                walkways = if (floor.shown) rings(floor.walkways, floor.baseM) else emptyList(),
                rooms = if (floor.shown) rings(floor.rooms, floor.baseM) else emptyList(),
                shown = floor.shown,
                street = floor.street,
                entered = floor.entered,
                plan = floor.hasPlan,
                under = floor.baseM < 0.0,
                below = floor.baseM < shownBase,
                label = outline.flatten().maxByOrNull { it.x },
            )
        }
        // The building's sides, at the outline's ends as seen from here.
        val corners = current.stack.flatMap { floor -> floor.outline.flatMap { it.rings.firstOrNull().orEmpty() } }
        val lowest = current.stack.firstOrNull()?.baseM ?: 0.0
        val highest = current.stack.lastOrNull()?.baseM ?: 0.0
        val seen = corners.mapNotNull { point -> sight.at(point, 0.0)?.let { point to it } }
        val sides = listOfNotNull(seen.minByOrNull { it.second.x }, seen.maxByOrNull { it.second.x }).mapNotNull { (point, _) ->
            val foot = sight.at(point, lowest) ?: return@mapNotNull null
            val head = sight.at(point, highest) ?: return@mapNotNull null
            foot to head
        }
        val ground = rings(current.stackGround, 0.0)
        onStack(
            StackPicture(
                under = floors.filter { it.under },
                ground = ground,
                over = floors.filter { !it.under },
                seeThrough = shownBase < 0.0,
                sides = sides,
            ),
        )
        val inset = insets()
        fun entrance(floor: CampusMapStackFloor, entrance: CampusMapStackEntrance): RaisedEntrance? {
            val at = sight.at(entrance.point, floor.baseM) ?: return null
            val inside = at.x >= inset.left && at.x <= view.width - inset.right &&
                at.y >= inset.top && at.y <= view.height - inset.bottom
            if (!inside) return null
            return RaisedEntrance(entrance.entranceId, entrance.name, entrance.selected, at.x.roundToInt(), at.y.roundToInt())
        }
        onRaised(current.stack.flatMap { floor -> floor.entrances.mapNotNull { entrance(floor, it) } })
        val rooms = current.stack.mapNotNull { floor ->
            val bottoms = rings(floor.highlight, floor.baseM)
            val tops = rings(floor.highlight, floor.baseM + HIGHLIGHT_M)
            if (bottoms.isEmpty() || bottoms.size != tops.size) return@mapNotNull null
            RaisedRoom(bottoms, tops, floor.shown)
        }
        onRooms(rooms)
    }

    /** The stack is seen at an angle and may be turned and tilted; a single
     * floor is seen from above, as on iOS. A framing for the stack keeps the
     * angle (`frame`). The tilt is this painter's own move, which the camera
     * reports to no one. */
    private fun tilt(map: MapLibreMap, stacked: Boolean) {
        if (stacked == tilted) return
        tilted = stacked
        map.uiSettings.isTiltGesturesEnabled = stacked
        map.animateCamera(CameraUpdateFactory.tiltTo(if (stacked) STACK_TILT else 0.0), FRAMING_MS)
    }

    // Camera

    /** The visible part leaves room for a framing's margins. */
    fun hasRoom(): Boolean {
        if (map == null) return false
        val inset = insets()
        val margin = 2 * (FRAMING_MARGIN_DP * density).roundToInt()
        return size.width - inset.left - inset.right > margin && size.height - inset.top - inset.bottom > margin
    }

    /** Moves the camera to Core's framing, within the part of the map the
     * controls leave visible, keeping the map's turn. */
    fun frame(framing: CampusMapFraming, animates: Boolean) {
        val map = map ?: return
        val inset = insets()
        val margin = (FRAMING_MARGIN_DP * density).roundToInt()
        // The camera is Core's now, not the blue dot's.
        if (positionStyle != null) runCatching { map.locationComponent.cameraMode = CameraMode.NONE }
        val bounds = framing.bounds
        if (framing.reveal) {
            reveal(map, bounds, inset, margin, animates)
            return
        }
        val box = LatLngBounds.from(bounds.north, bounds.east, bounds.south, bounds.west)
        val padding = intArrayOf(inset.left + margin, inset.top + margin, inset.right + margin, inset.bottom + margin)
        // The stack keeps its angle; a floor is framed from above.
        val update = map.getCameraForLatLngBounds(box, padding, map.cameraPosition.bearing, if (tilted) STACK_TILT else 0.0)
            ?.let(CameraUpdateFactory::newCameraPosition)
            ?: CameraUpdateFactory.newLatLngBounds(box, padding[0], padding[1], padding[2], padding[3])
        if (framed && animates) map.animateCamera(update, FRAMING_MS) else map.moveCamera(update)
        framed = true
    }

    /** Pans the least that brings `bounds` inside the visible part, at the
     * same zoom; centres them when they are larger than it. */
    private fun reveal(map: MapLibreMap, bounds: CampusMapBounds, inset: MapInsets, margin: Int, animates: Boolean) {
        val corners = listOf(
            LatLng(bounds.north, bounds.west), LatLng(bounds.north, bounds.east),
            LatLng(bounds.south, bounds.east), LatLng(bounds.south, bounds.west),
        ).map(map.projection::toScreenLocation)
        val left = (inset.left + margin).toFloat()
        val top = (inset.top + margin).toFloat()
        val right = (view.width - inset.right - margin).toFloat()
        val bottom = (view.height - inset.bottom - margin).toFloat()
        fun shift(low: Float, high: Float, from: Float, to: Float): Float = when {
            high - low > to - from -> (low + high) / 2 - (from + to) / 2
            low < from -> low - from
            high > to -> high - to
            else -> 0f
        }
        val dx = shift(corners.minOf { it.x }, corners.maxOf { it.x }, left, right)
        val dy = shift(corners.minOf { it.y }, corners.maxOf { it.y }, top, bottom)
        framed = true
        if (dx == 0f && dy == 0f) return
        if (positionStyle != null) runCatching { map.locationComponent.cameraMode = CameraMode.NONE }
        val center = map.projection.toScreenLocation(map.cameraPosition.target ?: return)
        val target = map.projection.fromScreenLocation(PointF(center.x + dx, center.y + dy))
        val update = CameraUpdateFactory.newLatLng(target)
        if (animates) map.animateCamera(update, FRAMING_MS) else map.moveCamera(update)
    }

    /** The person's move took the centre of the view somewhere else, rather
     * than turning or tilting the map about it. */
    private fun panned(map: MapLibreMap): Boolean {
        val from = movedFrom ?: return true
        val at = map.projection.toScreenLocation(from)
        val dx = at.x - view.width / 2f
        val dy = at.y - view.height / 2f
        return dx * dx + dy * dy >= (PAN_DP * density) * (PAN_DP * density)
    }

    /** The visible part's corners on the ground: a turned map gives a
     * turned box. In the stack, seen at an angle, only its near half: the
     * far half reaches far off and would weigh far more than it shows. */
    private fun report(map: MapLibreMap) {
        if (view.width == 0 || view.height == 0) return
        val inset = insets()
        val left = inset.left.toFloat()
        val bottom = (view.height - inset.bottom).toFloat().coerceAtLeast(inset.top + 1f)
        val visibleTop = inset.top.toFloat()
        val top = if (tilted) (visibleTop + bottom) / 2 else visibleTop
        val right = (view.width - inset.right).toFloat().coerceAtLeast(left + 1)
        fun at(x: Float, y: Float): CampusMapPoint {
            val ground = map.projection.fromScreenLocation(PointF(x, y))
            return CampusMapPoint(latitude = ground.latitude, longitude = ground.longitude)
        }
        onCamera(
            CampusMapCamera(
                topLeft = at(left, top),
                topRight = at(right, top),
                bottomRight = at(right, bottom),
                bottomLeft = at(left, bottom),
            ),
        )
    }

    /** MapLibre draws the blue dot itself, from Android's own location
     * service. `showsPosition` is true only once the permission is granted. */
    @SuppressLint("MissingPermission")
    private fun showPosition(map: MapLibreMap, style: Style) {
        val component = map.locationComponent
        runCatching {
            if (showsPosition) {
                if (positionStyle !== style) {
                    component.activateLocationComponent(LocationComponentActivationOptions.builder(context, style).build())
                    positionStyle = style
                    component.cameraMode = CameraMode.NONE
                    component.renderMode = RenderMode.NORMAL
                }
                if (!component.isLocationComponentEnabled) component.isLocationComponentEnabled = true
            } else if (positionStyle === style && component.isLocationComponentEnabled) {
                component.isLocationComponentEnabled = false
            }
        }
    }

    /** Where each mark is on screen now, for those in the visible part. */
    private fun place() {
        val map = map ?: return
        val inset = insets()
        val placed = scenery?.marks.orEmpty().mapNotNull { mark ->
            val at = map.projection.toScreenLocation(LatLng(mark.point.latitude, mark.point.longitude))
            val inside = at.x >= inset.left && at.x <= view.width - inset.right &&
                at.y >= inset.top && at.y <= view.height - inset.bottom
            if (inside) PlacedMark(mark, at.x.roundToInt(), at.y.roundToInt()) else null
        }
        onPlaced(placed)
    }

    private companion object {
        /** How far from a mark's centre a tap still picks it. */
        const val TOUCH_DP = 14
        /** The span across a tap that measures the map's scale there. */
        const val SCALE_DP = 20f
        /** The page's control size, gap and side inset (`CampusMapScreen`). */
        const val CONTROL_DP = 44
        val CONTROL_GAP_DP = Theme.space4.value
        val PAGE_INSET_DP = Theme.space6.value
        /** Room between the visible part's edges and what a framing shows. */
        const val FRAMING_MARGIN_DP = 16
        const val FRAMING_MS = 400
        /** How far the centre of the view must move, in the stack, for the
         * person's move to count as going somewhere. */
        const val PAN_DP = 48
        /** How the stack is seen: degrees from straight down. */
        const val STACK_TILT = 55.0
        /** The selected room's height in the stack, metres: a block tall
         * enough to read from any angle. */
        const val HIGHLIGHT_M = 3.0
        /** OpenFreeMap's quiet styles. */
        const val LIGHT_STYLE = "https://tiles.openfreemap.org/styles/positron"
        const val DARK_STYLE = "https://tiles.openfreemap.org/styles/dark"
    }
}

/** Where a point at a height shows on screen, as MapLibre's camera sees it.
 * MapLibre projects only the ground, so a point at height `h` above ground
 * point `g` is projected through the ground point on the same line of
 * sight: seen from altitude `A` over ground point `c`, that is
 * `c + (g - c) · A / (A - h)`. Below the ground (`h` < 0) it falls short of
 * `g`, towards the camera, as it should. MapLibre's camera stands 1.5 view
 * heights from the centre of the view (its field of view, 2·atan(1/3)),
 * tilted from straight down by the camera's tilt and facing its bearing;
 * this view gives the map no padding, so the centre of the view is the
 * camera's. iOS has the camera's altitude and position from MapKit. */
private class Sight(map: MapLibreMap, width: Int, height: Int) {
    private val projection = map.projection
    private val target: LatLng
    private val eastM: Double
    private val altitude: Double
    private val cameraX: Double
    private val cameraY: Double

    init {
        val centre = PointF(width / 2f, height / 2f)
        target = projection.fromScreenLocation(centre)
        // Across the centre of the view a pixel spans the map's own scale.
        val metresPerPixel = target.distanceTo(projection.fromScreenLocation(PointF(centre.x + SCALE_PX, centre.y))) / SCALE_PX
        val distance = CAMERA_DISTANCE * height * metresPerPixel
        val tilt = Math.toRadians(map.cameraPosition.tilt)
        val bearing = Math.toRadians(map.cameraPosition.bearing)
        altitude = distance * cos(tilt)
        // Behind the centre of the view, against the way the camera faces.
        val back = distance * sin(tilt)
        cameraX = -back * sin(bearing)
        cameraY = -back * cos(bearing)
        eastM = cos(Math.toRadians(target.latitude)) * METRES_PER_DEGREE_LNG
    }

    /** `null` for a point at or above the camera, which no view shows. */
    fun at(point: CampusMapPoint, height: Double): Offset? {
        if (altitude <= 0.0 || height >= altitude * 0.9) return null
        val x = (point.longitude - target.longitude) * eastM
        val y = (point.latitude - target.latitude) * METRES_PER_DEGREE_LAT
        val scale = altitude / (altitude - height)
        val groundX = cameraX + (x - cameraX) * scale
        val groundY = cameraY + (y - cameraY) * scale
        val screen = projection.toScreenLocation(
            LatLng(target.latitude + groundY / METRES_PER_DEGREE_LAT, target.longitude + groundX / eastM),
        )
        return Offset(screen.x, screen.y)
    }

    private companion object {
        /** MapLibre's camera from the centre of the view, in view heights. */
        const val CAMERA_DISTANCE = 1.5
        /** Pixels across the centre that measure the map's scale. */
        const val SCALE_PX = 100f
        const val METRES_PER_DEGREE_LAT = 110_574.0
        const val METRES_PER_DEGREE_LNG = 111_320.0
    }
}
