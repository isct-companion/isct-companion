package app.isctcompanion.features.map

import android.annotation.SuppressLint
import android.content.ComponentCallbacks2
import android.content.res.Configuration
import android.graphics.PointF
import android.graphics.RectF
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
import androidx.compose.ui.Modifier
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
import app.isctcompanion.core.CampusMapCamera
import app.isctcompanion.core.CampusMapFraming
import app.isctcompanion.core.CampusMapMark
import app.isctcompanion.core.CampusMapMarkKind
import app.isctcompanion.core.CampusMapPlace
import app.isctcompanion.core.CampusMapPoint
import app.isctcompanion.core.CampusMapScene
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
import kotlin.math.roundToInt

/** What the page's own controls cover at each edge of the map, in pixels:
 * the close button, the control rail, the panel. What is left is the part
 * of the map the person can see. */
data class MapInsets(val left: Int, val top: Int, val right: Int, val bottom: Int)

/** OpenFreeMap's base map (OpenStreetMap data, no key) with Core's scene
 * drawn over it by role. Core decides every layer and mark; this view only
 * paints them with MapLibre (`SceneStyle`), moves the camera where Core
 * frames, and reports the person's own moves of it over the part of the map
 * the controls leave visible. */
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
    /** Rises when the person asked for their position and may now see it. */
    follows: Int,
    /** Where the camera settled after the person moved it. */
    onCamera: (CampusMapCamera) -> Unit,
    /** A tap off the marks, the floor drawn under it and the map's metres
     * per dp there. */
    onTap: (CampusMapPoint, String?, Double) -> Unit,
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
    SideEffect {
        painter.insets = insets
        painter.onCamera = onCamera
        painter.onTap = onTap
        painter.onSelect = onSelect
        painter.onPlaced = { placed = it }
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
            update = { painter.show(scene, palette, dark, chrome, showsPosition, follows) },
        )
        SpokenMarks(placed, onSelect)
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
    var onTap: (CampusMapPoint, String?, Double) -> Unit = { _, _, _ -> }
    var onSelect: (CampusMapPlace) -> Unit = {}
    /** The marks in the visible part and where they are, for a screen reader. */
    var onPlaced: (List<PlacedMark>) -> Unit = {}

    private val context = view.context
    private val density = context.resources.displayMetrics.density
    private var map by mutableStateOf<MapLibreMap?>(null)
    /** The view's size, which a framing needs. */
    private var size by mutableStateOf(IntSize.Zero)
    private var scene: CampusMapScene? = null
    private var palette: Palette? = null
    private var dark = false
    private var showsPosition = false
    private var follows = 0
    private var chrome = MapInsets(0, 0, 0, 0)
    /** The person asked to be followed before the blue dot was ready. */
    private var followPending = false
    /** The camera's current move is the person's: a gesture, or following
     * their position, which they asked for. */
    private var personMoving = false
    /** A first framing places the camera; later ones animate. */
    private var framed = false
    /** The theme the style was asked for, and the scene's content in the
     * style once it is loaded. */
    private var appearance: Pair<Palette, Boolean>? = null
    private var scenery: SceneStyle? = null
    /** The style the blue dot was last attached to. */
    private var positionStyle: Style? = null

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
            personMoving = reason == MapLibreMap.OnCameraMoveStartedListener.REASON_API_GESTURE || following(map)
        }
        map.addOnCameraIdleListener {
            if (personMoving) report(map)
            personMoving = false
            place()
        }
        // A double tap zooms and never arrives here; it selects nothing.
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
                    onTap(CampusMapPoint(latitude = at.latitude, longitude = at.longitude), scene?.levelId, at.distanceTo(across) / SCALE_DP)
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
        follows: Int,
    ) {
        this.scene = scene
        this.palette = palette
        this.dark = dark
        this.showsPosition = showsPosition
        this.chrome = chrome
        if (follows != this.follows) {
            this.follows = follows
            followPending = true
        }
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
        val box = LatLngBounds.from(bounds.north, bounds.east, bounds.south, bounds.west)
        val padding = intArrayOf(inset.left + margin, inset.top + margin, inset.right + margin, inset.bottom + margin)
        val update = map.getCameraForLatLngBounds(box, padding, map.cameraPosition.bearing, 0.0)
            ?.let(CameraUpdateFactory::newCameraPosition)
            ?: CameraUpdateFactory.newLatLngBounds(box, padding[0], padding[1], padding[2], padding[3])
        if (framed && animates) map.animateCamera(update, FRAMING_MS) else map.moveCamera(update)
        framed = true
    }

    private fun following(map: MapLibreMap): Boolean =
        positionStyle != null && runCatching { map.locationComponent.cameraMode != CameraMode.NONE }.getOrDefault(false)

    /** The visible part's corners on the ground: a turned map gives a
     * turned box. */
    private fun report(map: MapLibreMap) {
        if (view.width == 0 || view.height == 0) return
        val inset = insets()
        val left = inset.left.toFloat()
        val top = inset.top.toFloat()
        val right = (view.width - inset.right).toFloat().coerceAtLeast(left + 1)
        val bottom = (view.height - inset.bottom).toFloat().coerceAtLeast(top + 1)
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
                if (followPending) {
                    // The person's position stays in the middle until they
                    // move the map.
                    followPending = false
                    component.cameraMode = CameraMode.TRACKING
                }
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
        const val CONTROL_GAP_DP = 8
        const val PAGE_INSET_DP = 12
        /** Room between the visible part's edges and what a framing shows. */
        const val FRAMING_MARGIN_DP = 16
        const val FRAMING_MS = 400
        /** OpenFreeMap's quiet styles. */
        const val LIGHT_STYLE = "https://tiles.openfreemap.org/styles/positron"
        const val DARK_STYLE = "https://tiles.openfreemap.org/styles/dark"
    }
}
