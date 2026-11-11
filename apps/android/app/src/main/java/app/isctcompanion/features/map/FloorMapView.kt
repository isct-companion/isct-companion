package app.isctcompanion.features.map

import android.annotation.SuppressLint
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.Typeface
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import app.isctcompanion.R
import app.isctcompanion.core.CampusMapBounds
import app.isctcompanion.core.CampusMapCamera
import app.isctcompanion.core.CampusMapFraming
import app.isctcompanion.core.CampusMapLayer
import app.isctcompanion.core.CampusMapLayerRole
import app.isctcompanion.core.CampusMapMark
import app.isctcompanion.core.CampusMapMarkKind
import app.isctcompanion.core.CampusMapPoint
import app.isctcompanion.core.CampusMapPolygon
import app.isctcompanion.core.CampusMapScene
import app.isctcompanion.design.LocalReducedMotion
import app.isctcompanion.design.Palette
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import com.google.android.gms.maps.CameraUpdateFactory
import com.google.android.gms.maps.GoogleMap
import com.google.android.gms.maps.GoogleMapOptions
import com.google.android.gms.maps.MapView
import com.google.android.gms.maps.model.BitmapDescriptor
import com.google.android.gms.maps.model.BitmapDescriptorFactory
import com.google.android.gms.maps.model.JointType
import com.google.android.gms.maps.model.LatLng
import com.google.android.gms.maps.model.LatLngBounds
import com.google.android.gms.maps.model.MapColorScheme
import com.google.android.gms.maps.model.MapStyleOptions
import com.google.android.gms.maps.model.Marker
import com.google.android.gms.maps.model.MarkerOptions
import com.google.android.gms.maps.model.Polygon
import com.google.android.gms.maps.model.PolygonOptions
import com.google.android.gms.maps.model.Polyline
import com.google.android.gms.maps.model.PolylineOptions
import com.google.android.gms.maps.model.RoundCap
import kotlin.math.ceil
import kotlin.math.cos
import kotlin.math.pow
import kotlin.math.roundToInt

/** What the card and the level picker cover, in pixels, kept clear when framing. */
data class MapInsets(val left: Int, val top: Int, val right: Int, val bottom: Int)

/** Google's base map with Core's scene drawn over it by role. Core decides
 * every layer and mark; this view only paints them with the Maps SDK:
 * polygons and polylines per layer, markers per mark. */
@Composable
fun FloorMapView(
    scene: CampusMapScene?,
    /** Read when a framing is applied, after the card has been measured. */
    insets: () -> MapInsets,
    showsPosition: Boolean,
    /** Where the camera settled, with the last framing key applied. */
    onCamera: (CampusMapCamera, ULong) -> Unit,
    onTap: (CampusMapPoint, String?) -> Unit,
    onEntrance: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val palette = Theme.colors
    val dark = isSystemInDarkTheme()
    val animates = !LocalReducedMotion.current
    val view = remember {
        MapView(
            context,
            // No tilt, as on iOS; the toolbar would offer a route, which the
            // card's links already do.
            GoogleMapOptions().compassEnabled(true).tiltGesturesEnabled(false).mapToolbarEnabled(false),
        )
    }
    val painter = remember { FloorPainter(view) }
    SideEffect {
        painter.insets = insets
        painter.onCamera = onCamera
        painter.onTap = onTap
        painter.onEntrance = onEntrance
    }
    MapLifecycle(view)
    DisposableEffect(view) {
        view.getMapAsync(painter::attach)
        onDispose { painter.detach() }
    }
    AndroidView(
        factory = { view },
        modifier = modifier,
        update = { painter.show(scene, palette, dark, showsPosition, animates) },
    )
}

/** A `MapView` is told each lifecycle step by its host; this one lives as
 * long as the map page's destination. */
@Composable
private fun MapLifecycle(view: MapView) {
    val lifecycle = LocalLifecycleOwner.current.lifecycle
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
        onDispose {
            lifecycle.removeObserver(observer)
            if (resumed) view.onPause()
            if (started) view.onStop()
            view.onDestroy()
        }
    }
}

/** One layer as the Maps SDK holds it: a shape per polygon or line. */
private class DrawnLayer(
    val role: CampusMapLayerRole,
    val widthM: Double,
    val polygons: List<Polygon>,
    val lines: List<Polyline>,
    var z: Float,
) {
    fun remove() {
        polygons.forEach { it.remove() }
        lines.forEach { it.remove() }
    }

    fun raise(to: Float) {
        if (z == to) return
        z = to
        polygons.forEach { it.zIndex = to }
        lines.forEach { it.zIndex = to }
    }
}

private class MarkIcon(val descriptor: BitmapDescriptor, val width: Int, val height: Int)

private class DrawnMark(val record: CampusMapMark, val marker: Marker, val icon: MarkIcon, val priority: Int)

/** Draws scenes on one `GoogleMap` and reports what the person does on it.
 * It keeps only what was drawn, keyed by Core's ids, so unchanged geometry
 * stays in the map when another part of the scene changes. */
private class FloorPainter(private val view: MapView) {
    var insets: () -> MapInsets = { MapInsets(0, 0, 0, 0) }
    var onCamera: (CampusMapCamera, ULong) -> Unit = { _, _ -> }
    var onTap: (CampusMapPoint, String?) -> Unit = { _, _ -> }
    var onEntrance: (String) -> Unit = {}

    private val context = view.context
    private val density = context.resources.displayMetrics.density
    private var map: GoogleMap? = null
    private var scene: CampusMapScene? = null
    private var palette: Palette? = null
    private var dark = false
    private var showsPosition = false
    private var animatesCamera = true
    private var levelId: String? = null
    private var appliedFraming = 0uL
    private var pendingFraming: CampusMapFraming? = null
    private var framed = false
    private var userMoving = false
    private var appearance: Pair<Palette, Boolean>? = null
    private val overlays = HashMap<String, DrawnLayer>()
    private val marks = HashMap<String, DrawnMark>()
    private val icons = HashMap<String, MarkIcon>()
    /** The zoom the line widths were last set for. */
    private var lineZoom = Float.NaN
    /** The zoom and bearing the marks were last laid out for. */
    private var layoutZoom = Float.NaN
    private var layoutBearing = Float.NaN

    init {
        // A framing needs a real size, which a new map does not have yet.
        view.addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ -> applyFraming() }
    }

    fun attach(map: GoogleMap) {
        this.map = map
        // Google's own indoor plans and 3D buildings would sit under ours.
        map.setIndoorEnabled(false)
        map.isBuildingsEnabled = false
        map.uiSettings.isMyLocationButtonEnabled = false
        map.uiSettings.isIndoorLevelPickerEnabled = false
        map.setOnCameraMoveStartedListener { reason ->
            if (reason == GoogleMap.OnCameraMoveStartedListener.REASON_GESTURE) userMoving = true
        }
        map.setOnCameraMoveListener {
            // Panning moves every mark alike; zooming and turning do not.
            val position = map.cameraPosition
            if (position.zoom != layoutZoom || position.bearing != layoutBearing) layoutMarks()
        }
        map.setOnCameraIdleListener {
            resizeLines()
            layoutMarks()
            report()
        }
        // A double tap zooms and never arrives here; it selects nothing.
        map.setOnMapClickListener { at -> onTap(CampusMapPoint(latitude = at.latitude, longitude = at.longitude), levelId) }
        map.setOnMarkerClickListener { marker ->
            val record = (marker.tag as? String)?.let { marks[it] }?.record
            if (record != null) {
                when (val kind = record.kind) {
                    is CampusMapMarkKind.Entrance -> onEntrance(kind.entranceId)
                    // A label or an amenity is not a target of its own: the
                    // tap goes to the floor under it.
                    else -> onTap(record.point, levelId)
                }
            }
            // Consumed: no info window, and the camera stays.
            true
        }
        render()
    }

    fun detach() {
        val map = map ?: return
        map.setOnCameraMoveStartedListener(null)
        map.setOnCameraMoveListener(null)
        map.setOnCameraIdleListener(null)
        map.setOnMapClickListener(null)
        map.setOnMarkerClickListener(null)
        this.map = null
    }

    fun show(scene: CampusMapScene?, palette: Palette, dark: Boolean, showsPosition: Boolean, animatesCamera: Boolean) {
        this.scene = scene
        this.palette = palette
        this.dark = dark
        this.showsPosition = showsPosition
        this.animatesCamera = animatesCamera
        render()
    }

    @SuppressLint("MissingPermission")
    private fun render() {
        val map = map ?: return
        val palette = palette ?: return
        // `showsPosition` is true only once the permission is granted.
        if (map.isMyLocationEnabled != showsPosition) runCatching { map.isMyLocationEnabled = showsPosition }
        prepareAppearance(map, palette)
        drawLayers(map, scene?.layers.orEmpty(), palette)
        drawMarks(map, scene?.marks.orEmpty(), palette)
        levelId = scene?.levelId
        val framing = scene?.framing
        if (framing != null && framing.key > appliedFraming && framing.key != pendingFraming?.key) {
            pendingFraming = framing
            // After this frame's layout, so the insets include the card
            // this scene brought.
            view.post { applyFraming() }
        }
    }

    // Camera

    private fun applyFraming() {
        val map = map ?: return
        val framing = pendingFraming ?: return
        if (view.width == 0 || view.height == 0) return
        val inset = insets()
        // The padding keeps the framed box clear of the card and the level
        // picker, and moves Google's logo and compass clear of them too.
        map.setPadding(inset.left, inset.top, inset.right, inset.bottom)
        val bounds = framing.bounds
        val update = CameraUpdateFactory.newLatLngBounds(
            LatLngBounds(LatLng(bounds.south, bounds.west), LatLng(bounds.north, bounds.east)), 0,
        )
        try {
            if (framed && animatesCamera) map.animateCamera(update) else map.moveCamera(update)
        } catch (_: IllegalStateException) {
            // The map itself is not laid out yet; the next layout tries again.
            return
        }
        pendingFraming = null
        appliedFraming = framing.key
        framed = true
    }

    private fun report() {
        val map = map ?: return
        val position = map.cameraPosition
        val box = map.projection.visibleRegion.latLngBounds
        val west = box.southwest.longitude
        val east = box.northeast.longitude
        val span = if (east >= west) east - west else east - west + 360.0
        val camera = CampusMapCamera(
            center = CampusMapPoint(latitude = position.target.latitude, longitude = position.target.longitude),
            bounds = CampusMapBounds(south = box.southwest.latitude, west = west, north = box.northeast.latitude, east = east),
            // The width the box spans at this zoom, in the 256-point tiles
            // Core's zoom counts in: a turned map's box is wider than the
            // screen, and Core still reads the true zoom from it.
            widthPoints = span / 360.0 * 256.0 * 2.0.pow(position.zoom.toDouble()),
            byUser = userMoving,
        )
        userMoving = false
        onCamera(camera, appliedFraming)
    }

    // Layers

    private fun drawLayers(map: GoogleMap, layers: List<CampusMapLayer>, palette: Palette) {
        val wanted = layers.mapTo(HashSet()) { it.id }
        val gone = overlays.keys.filter { it !in wanted }
        for (id in gone) overlays.remove(id)?.remove()
        val lineWidth = map.cameraPosition.let { position -> { meters: Double -> linePixels(meters, position.zoom, position.target.latitude) } }
        layers.forEachIndexed { index, layer ->
            // Bottom to top, as Core lists them.
            val z = index.toFloat()
            val drawn = overlays[layer.id]
            if (drawn != null) {
                drawn.raise(z)
                return@forEachIndexed
            }
            overlays[layer.id] = when (layer.role) {
                CampusMapLayerRole.DETAIL_LINE, CampusMapLayerRole.WALL_LINE -> {
                    val color = (if (layer.role == CampusMapLayerRole.WALL_LINE) palette.ink else palette.muted).toArgb()
                    val width = lineWidth(layer.widthM)
                    val lines = layer.lines.filter { it.size > 1 }.map { line ->
                        map.addPolyline(
                            PolylineOptions().addAll(line.map { it.latLng }).color(color).width(width).zIndex(z)
                                .startCap(RoundCap()).endCap(RoundCap()).jointType(JointType.ROUND),
                        )
                    }
                    DrawnLayer(layer.role, layer.widthM, emptyList(), lines, z)
                }
                else -> {
                    val style = polygonStyle(layer.role, palette)
                    val polygons = layer.polygons.mapNotNull { polygon -> polygon.options(style, z)?.let(map::addPolygon) }
                    DrawnLayer(layer.role, layer.widthM, polygons, emptyList(), z)
                }
            }
        }
        lineZoom = map.cameraPosition.zoom
    }

    private class PolygonStyle(val fill: Int, val stroke: Int, val strokeWidth: Float)

    private fun polygonStyle(role: CampusMapLayerRole, palette: Palette): PolygonStyle {
        val clear = android.graphics.Color.TRANSPARENT
        return when (role) {
            CampusMapLayerRole.FOOTPRINT ->
                PolygonStyle(palette.surface2.copy(alpha = 0.8f).toArgb(), palette.line.toArgb(), density)
            CampusMapLayerRole.FOCUSED_FOOTPRINT ->
                PolygonStyle(palette.surface.toArgb(), palette.accent.toArgb(), 1.5f * density)
            CampusMapLayerRole.LEVEL_OUTLINE ->
                PolygonStyle(palette.surface.toArgb(), palette.line.toArgb(), density)
            CampusMapLayerRole.CLASSROOM -> PolygonStyle(palette.surface2.toArgb(), clear, 0f)
            CampusMapLayerRole.HIGHLIGHT ->
                PolygonStyle(palette.accent.copy(alpha = 0.35f).toArgb(), palette.accent.toArgb(), 2f * density)
            CampusMapLayerRole.DETAIL_LINE, CampusMapLayerRole.WALL_LINE -> PolygonStyle(clear, clear, 0f)
        }
    }

    /** One outer ring, then its holes. */
    private fun CampusMapPolygon.options(style: PolygonStyle, z: Float): PolygonOptions? {
        val outer = rings.firstOrNull()?.takeIf { it.size > 2 } ?: return null
        val options = PolygonOptions().addAll(outer.map { it.latLng })
            .fillColor(style.fill).strokeColor(style.stroke).strokeWidth(style.strokeWidth)
            .strokeJointType(JointType.ROUND).zIndex(z)
        for (hole in rings.drop(1)) if (hole.size > 2) options.addHole(hole.map { it.latLng })
        return options
    }

    /** A line's width on screen: its physical width at this zoom, with a
     * hairline floor. The Maps SDK measures polylines in pixels, so the
     * widths follow the zoom each time the camera settles. */
    private fun linePixels(meters: Double, zoom: Float, latitude: Double): Float {
        val pixelsPerMeter = 256.0 * density * 2.0.pow(zoom.toDouble()) /
            (EQUATOR_M * cos(Math.toRadians(latitude)).coerceAtLeast(0.01))
        return (meters * pixelsPerMeter).toFloat().coerceAtLeast((0.5f * density).coerceAtLeast(1f))
    }

    private fun resizeLines() {
        val map = map ?: return
        val position = map.cameraPosition
        if (position.zoom == lineZoom) return
        lineZoom = position.zoom
        for (layer in overlays.values) {
            if (layer.lines.isEmpty()) continue
            val width = linePixels(layer.widthM, position.zoom, position.target.latitude)
            layer.lines.forEach { it.width = width }
        }
    }

    // Marks

    private fun drawMarks(map: GoogleMap, records: List<CampusMapMark>, palette: Palette) {
        val wanted = records.mapTo(HashSet()) { it.id }
        val gone = marks.keys.filter { it !in wanted }
        for (id in gone) marks.remove(id)?.marker?.remove()
        var changed = gone.isNotEmpty()
        for (record in records) {
            val old = marks[record.id]
            if (old?.record == record) continue
            old?.marker?.remove()
            marks.remove(record.id)
            val icon = icon(record.kind, palette) ?: continue
            val priority = priority(record.kind)
            val marker = map.addMarker(
                MarkerOptions().position(record.point.latLng).icon(icon.descriptor).anchor(0.5f, 0.5f)
                    .zIndex(priority.toFloat()).title(spoken(record.kind)),
            ) ?: continue
            marker.tag = record.id
            marks[record.id] = DrawnMark(record, marker, icon, priority)
            changed = true
        }
        if (changed) layoutMarks()
    }

    /** Which marks the screen has room for. MapKit drops colliding
     * annotations itself; the Maps SDK's markers have no such rule without a
     * cloud map id, so this does the same by priority: highlighted rooms and
     * entrances always show, then amenities, then room labels. */
    private fun layoutMarks() {
        val map = map ?: return
        val position = map.cameraPosition
        layoutZoom = position.zoom
        layoutBearing = position.bearing
        val projection = map.projection
        val kept = ArrayList<Rect>()
        // By id within a priority, so the same marks win at every zoom.
        for (mark in marks.values.sortedWith(compareByDescending<DrawnMark> { it.priority }.thenBy { it.record.id })) {
            val at = projection.toScreenLocation(mark.marker.position)
            val box = Rect(
                at.x - mark.icon.width / 2, at.y - mark.icon.height / 2,
                at.x + (mark.icon.width + 1) / 2, at.y + (mark.icon.height + 1) / 2,
            )
            val shows = mark.priority >= REQUIRED || kept.none { Rect.intersects(it, box) }
            if (shows) kept.add(box)
            if (mark.marker.isVisible != shows) mark.marker.isVisible = shows
        }
    }

    private fun priority(kind: CampusMapMarkKind): Int = when (kind) {
        is CampusMapMarkKind.Room -> if (kind.highlighted) 3 else 0
        is CampusMapMarkKind.Amenity -> 1
        is CampusMapMarkKind.Entrance -> if (kind.selected) 4 else REQUIRED
    }

    /** What a screen reader says for a mark. */
    private fun spoken(kind: CampusMapMarkKind): String = when (kind) {
        is CampusMapMarkKind.Room -> kind.label
        is CampusMapMarkKind.Amenity -> t("campusMap.amenity.${kind.key}")
        is CampusMapMarkKind.Entrance -> kind.name ?: t("campusMap.entrance")
    }

    private fun icon(kind: CampusMapMarkKind, palette: Palette): MarkIcon? = when (kind) {
        // Each label is its own picture; the round icons repeat.
        is CampusMapMarkKind.Room -> label(kind.label, kind.highlighted, palette)
        is CampusMapMarkKind.Amenity -> amenityDrawable(kind.key)?.let { drawable ->
            icons.getOrPut("amenity/${kind.key}") { round(drawable, palette.ink.toArgb(), palette.surface.toArgb(), palette) }
        }
        is CampusMapMarkKind.Entrance -> icons.getOrPut("entrance/${kind.selected}") {
            if (kind.selected) {
                round(R.drawable.campus_map_entrance, palette.accentInk.toArgb(), palette.accent.toArgb(), palette)
            } else {
                round(R.drawable.campus_map_entrance, palette.accent.toArgb(), palette.surface.toArgb(), palette)
            }
        }
    }

    /** A room's code or name; the selected room's is larger and in the accent. */
    private fun label(text: String, highlighted: Boolean, palette: Palette): MarkIcon {
        val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            textSize = (if (highlighted) 13f else 10f) * density
            typeface = Typeface.create(Typeface.DEFAULT, if (highlighted) 700 else 500, false)
            color = (if (highlighted) palette.accent else palette.muted).toArgb()
        }
        val metrics = paint.fontMetrics
        val width = ceil(paint.measureText(text)).toInt().coerceAtLeast(1)
        val height = ceil(metrics.descent - metrics.ascent).toInt().coerceAtLeast(1)
        val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
        Canvas(bitmap).drawText(text, 0f, -metrics.ascent, paint)
        return MarkIcon(BitmapDescriptorFactory.fromBitmap(bitmap), width, height)
    }

    /** An icon on a 28 dp disc with a hairline rim, as iOS draws them. */
    private fun round(drawable: Int, tint: Int, fill: Int, palette: Palette): MarkIcon {
        val side = (ICON_DP * density).roundToInt()
        val bitmap = Bitmap.createBitmap(side, side, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        val center = side / 2f
        paint.color = fill
        canvas.drawCircle(center, center, center - density / 2, paint)
        paint.style = Paint.Style.STROKE
        paint.strokeWidth = density
        paint.color = palette.line.toArgb()
        canvas.drawCircle(center, center, center - density / 2, paint)
        val inset = (5 * density).roundToInt()
        context.getDrawable(drawable)?.mutate()?.apply {
            setTint(tint)
            setBounds(inset, inset, side - inset, side - inset)
            draw(canvas)
        }
        return MarkIcon(BitmapDescriptorFactory.fromBitmap(bitmap), side, side)
    }

    // Appearance

    /** Theme colours are layer content too; a rare theme change redraws
     * everything rather than restyling each shape. */
    private fun prepareAppearance(map: GoogleMap, palette: Palette) {
        val next = palette to dark
        if (next == appearance) return
        appearance = next
        overlays.values.forEach { it.remove() }
        overlays.clear()
        marks.values.forEach { it.marker.remove() }
        marks.clear()
        icons.clear()
        map.setMapColorScheme(if (dark) MapColorScheme.DARK else MapColorScheme.LIGHT)
        // The campus is the subject: no shops and sights around it.
        map.setMapStyle(MapStyleOptions(BASE_STYLE))
    }

    private val CampusMapPoint.latLng: LatLng
        get() = LatLng(latitude, longitude)

    private companion object {
        const val ICON_DP = 28
        /** From this priority up a mark shows whatever it overlaps. */
        const val REQUIRED = 2
        /** The equator's length, which a zoom-0 tile spans. */
        const val EQUATOR_M = 40_075_016.686
        const val BASE_STYLE = """[{"featureType":"poi","stylers":[{"visibility":"off"}]}]"""
    }
}

/** The drawable of an amenity, by Core's amenity key; the same 14 pictures
 * iOS holds as `CampusMap/<key>`. */
private fun amenityDrawable(key: String): Int? = when (key) {
    "aed" -> R.drawable.campus_map_aed
    "calm-down" -> R.drawable.campus_map_calm_down
    "elevator" -> R.drawable.campus_map_elevator
    "lounge" -> R.drawable.campus_map_lounge
    "lounge-international" -> R.drawable.campus_map_lounge_international
    "lounge-women" -> R.drawable.campus_map_lounge_women
    "rest-room" -> R.drawable.campus_map_rest_room
    "restroom" -> R.drawable.campus_map_restroom
    "restroom-accessible" -> R.drawable.campus_map_restroom_accessible
    "restroom-female" -> R.drawable.campus_map_restroom_female
    "restroom-male" -> R.drawable.campus_map_restroom_male
    "stairs" -> R.drawable.campus_map_stairs
    "study-room" -> R.drawable.campus_map_study_room
    else -> null
}
