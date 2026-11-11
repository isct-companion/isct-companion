package app.isctcompanion.features.map

import android.annotation.SuppressLint
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
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
import app.isctcompanion.core.CampusMapScene
import app.isctcompanion.design.LocalReducedMotion
import app.isctcompanion.design.Palette
import app.isctcompanion.design.Theme
import org.json.JSONObject
import org.maplibre.android.MapLibre
import org.maplibre.android.camera.CameraUpdateFactory
import org.maplibre.android.geometry.LatLngBounds
import org.maplibre.android.location.LocationComponentActivationOptions
import org.maplibre.android.location.modes.CameraMode
import org.maplibre.android.location.modes.RenderMode
import org.maplibre.android.maps.MapLibreMap
import org.maplibre.android.maps.MapView
import org.maplibre.android.maps.Style
import org.maplibre.android.style.expressions.Expression
import org.maplibre.android.style.layers.FillLayer
import org.maplibre.android.style.layers.Layer
import org.maplibre.android.style.layers.LineLayer
import org.maplibre.android.style.layers.Property
import org.maplibre.android.style.layers.PropertyFactory
import org.maplibre.android.style.layers.SymbolLayer
import org.maplibre.android.style.sources.GeoJsonSource
import kotlin.math.cos
import kotlin.math.pow
import kotlin.math.roundToInt

/** What the card and the level picker cover, in pixels, kept clear when framing. */
data class MapInsets(val left: Int, val top: Int, val right: Int, val bottom: Int)

/** OpenFreeMap's base map (OpenStreetMap data, no key) with Core's scene
 * drawn over it by role. Core decides every layer and mark; this view only
 * paints them with MapLibre: one GeoJSON source and style layer per role,
 * line widths in metres, and MapLibre's own symbol collision. */
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
        // MapLibre is initialised once before its first view.
        MapLibre.getInstance(context)
        MapView(context)
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
        view.getMapAsync { painter.attach(it) }
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

/** Draws scenes on one MapLibre map and reports what the person does on it.
 * Every role has one source in the style; a source is rewritten only when
 * the ids of Core's layers in it change, so unchanged geometry stays as it
 * is when another part of the scene changes. */
private class FloorPainter(private val view: MapView) {
    var insets: () -> MapInsets = { MapInsets(0, 0, 0, 0) }
    var onCamera: (CampusMapCamera, ULong) -> Unit = { _, _ -> }
    var onTap: (CampusMapPoint, String?) -> Unit = { _, _ -> }
    var onEntrance: (String) -> Unit = {}

    private val context = view.context
    private val density = context.resources.displayMetrics.density
    private var map: MapLibreMap? = null
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
    /** The theme the style was asked for, and the style once it is loaded
     * and holds this painter's sources and layers. */
    private var appearance: Pair<Palette, Boolean>? = null
    private var style: Style? = null
    /** The style the blue dot was last attached to. */
    private var positionStyle: Style? = null
    /** Per role, the ids of Core's layers its source holds now. */
    private val drawn = HashMap<CampusMapLayerRole, List<String>>()
    private var drawnMarks: List<CampusMapMark>? = null

    init {
        // A framing needs a real size, which a new map does not have yet.
        view.addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ -> applyFraming() }
    }

    fun attach(map: MapLibreMap) {
        this.map = map
        // No tilt, as on iOS. The attribution button stays: it credits
        // OpenFreeMap, OpenMapTiles and OpenStreetMap.
        map.uiSettings.isTiltGesturesEnabled = false
        map.uiSettings.isLogoEnabled = false
        map.addOnCameraMoveStartedListener { reason ->
            if (reason == MapLibreMap.OnCameraMoveStartedListener.REASON_API_GESTURE) userMoving = true
        }
        map.addOnCameraIdleListener { report() }
        // A double tap zooms and never arrives here; it selects nothing.
        map.addOnMapClickListener { at ->
            val live = this.map
            if (live != null) {
                val point = live.projection.toScreenLocation(at)
                val reach = TOUCH_DP * density
                val touched = RectF(point.x - reach, point.y - reach, point.x + reach, point.y + reach)
                val entrance = live.queryRenderedFeatures(touched, ENTRANCES).firstOrNull()?.getStringProperty("entrance")
                // Anything but an entrance is a tap on the floor under it.
                if (entrance != null) {
                    onEntrance(entrance)
                } else {
                    onTap(CampusMapPoint(latitude = at.latitude, longitude = at.longitude), levelId)
                }
            }
            true
        }
        render()
    }

    /** The view is destroyed with the page; nothing drawn outlives it. */
    fun detach() {
        map = null
        style = null
    }

    fun show(scene: CampusMapScene?, palette: Palette, dark: Boolean, showsPosition: Boolean, animatesCamera: Boolean) {
        this.scene = scene
        this.palette = palette
        this.dark = dark
        this.showsPosition = showsPosition
        this.animatesCamera = animatesCamera
        render()
    }

    private fun render() {
        val map = map ?: return
        val palette = palette ?: return
        levelId = scene?.levelId
        val framing = scene?.framing
        if (framing != null && framing.key > appliedFraming && framing.key != pendingFraming?.key) {
            pendingFraming = framing
            // After this frame's layout, so the insets include the card
            // this scene brought.
            view.post { applyFraming() }
        }
        val next = palette to dark
        if (next != appearance) {
            // Theme colours are layer content too; a rare theme change loads
            // the matching base map and draws everything again.
            appearance = next
            style = null
            drawn.clear()
            drawnMarks = null
            map.setStyle(Style.Builder().fromUri(if (dark) DARK_STYLE else LIGHT_STYLE)) { loaded ->
                if (appearance == next && this.map != null) {
                    prepare(loaded, palette)
                    style = loaded
                    render()
                }
            }
            return
        }
        val style = style ?: return
        showPosition(map, style)
        drawLayers(style, scene?.layers.orEmpty())
        drawMarks(style, scene?.marks.orEmpty())
    }

    // Camera

    private fun applyFraming() {
        val map = map ?: return
        val framing = pendingFraming ?: return
        if (view.width == 0 || view.height == 0) return
        pendingFraming = null
        appliedFraming = framing.key
        val inset = insets()
        // The compass and the attribution button keep clear of the close
        // button and the card too.
        map.uiSettings.setCompassMargins(0, inset.top, (8 * density).roundToInt(), 0)
        map.uiSettings.setAttributionMargins((8 * density).roundToInt(), 0, 0, inset.bottom)
        val bounds = framing.bounds
        val update = CameraUpdateFactory.newLatLngBounds(
            LatLngBounds.from(bounds.north, bounds.east, bounds.south, bounds.west),
            inset.left, inset.top, inset.right, inset.bottom,
        )
        if (framed && animatesCamera) map.animateCamera(update, FRAMING_MS) else map.moveCamera(update)
        framed = true
    }

    private fun report() {
        val map = map ?: return
        val position = map.cameraPosition
        val target = position.target ?: return
        val region = map.projection.visibleRegion
        val corners = listOfNotNull(region.farLeft, region.farRight, region.nearLeft, region.nearRight)
        if (corners.isEmpty()) return
        val south = corners.minOf { it.latitude }
        val north = corners.maxOf { it.latitude }
        val west = corners.minOf { it.longitude }
        val east = corners.maxOf { it.longitude }
        val camera = CampusMapCamera(
            center = CampusMapPoint(latitude = target.latitude, longitude = target.longitude),
            bounds = CampusMapBounds(south = south, west = west, north = north, east = east),
            // The width the box spans at this zoom, in the 256-point tiles
            // Core's zoom counts in (MapLibre's tiles are 512, one zoom
            // level less): a turned map's box is wider than the screen, and
            // Core still reads the true zoom from it.
            widthPoints = (east - west) / 360.0 * 256.0 * 2.0.pow(position.zoom + 1.0),
            byUser = userMoving,
        )
        userMoving = false
        onCamera(camera, appliedFraming)
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

    // Style

    /** Adds this painter's images, sources and layers to a loaded base map.
     * Footprints go under the base map's labels, the floor above them, as
     * on iOS; the marks are last, and of them the ones MapLibre must keep. */
    private fun prepare(style: Style, palette: Palette) {
        for ((key, drawable) in amenityDrawables) {
            style.addImage(amenityImage(key), round(drawable, palette.ink.toArgb(), palette.surface.toArgb(), palette))
        }
        style.addImage(
            ENTRANCE_IMAGE, round(R.drawable.campus_map_entrance, palette.accent.toArgb(), palette.surface.toArgb(), palette),
        )
        style.addImage(
            ENTRANCE_SELECTED_IMAGE,
            round(R.drawable.campus_map_entrance, palette.accentInk.toArgb(), palette.accent.toArgb(), palette),
        )
        for (role in CampusMapLayerRole.entries) style.addSource(GeoJsonSource(source(role)))
        style.addSource(GeoJsonSource(MARKS))

        val labels = style.layers.firstOrNull { it is SymbolLayer }?.id
        fun under(layer: Layer) {
            if (labels != null) style.addLayerBelow(layer, labels) else style.addLayer(layer)
        }
        under(fill(CampusMapLayerRole.FOOTPRINT, palette.surface2.copy(alpha = 0.8f).toArgb()))
        under(stroke(CampusMapLayerRole.FOOTPRINT, palette.line.toArgb(), 1f))
        under(fill(CampusMapLayerRole.FOCUSED_FOOTPRINT, palette.surface.toArgb()))
        under(stroke(CampusMapLayerRole.FOCUSED_FOOTPRINT, palette.accent.toArgb(), 1.5f))
        style.addLayer(fill(CampusMapLayerRole.LEVEL_OUTLINE, palette.surface.toArgb()))
        style.addLayer(stroke(CampusMapLayerRole.LEVEL_OUTLINE, palette.line.toArgb(), 1f))
        style.addLayer(fill(CampusMapLayerRole.CLASSROOM, palette.surface2.toArgb()))
        style.addLayer(metricLine(CampusMapLayerRole.DETAIL_LINE, palette.muted.toArgb()))
        style.addLayer(metricLine(CampusMapLayerRole.WALL_LINE, palette.ink.toArgb()))
        style.addLayer(fill(CampusMapLayerRole.HIGHLIGHT, palette.accent.copy(alpha = 0.35f).toArgb()))
        style.addLayer(stroke(CampusMapLayerRole.HIGHLIGHT, palette.accent.toArgb(), 2f))

        // MapLibre places the symbols of upper layers first, so the order
        // is the priority: room labels give way to amenities, and entrances
        // and the selected room always show.
        val kind = Expression.get("kind")
        val lit = Expression.get("lit")
        style.addLayer(
            SymbolLayer("campus-labels", MARKS)
                .withFilter(Expression.all(Expression.eq(kind, Expression.literal("room")), Expression.eq(lit, Expression.literal(false))))
                .withProperties(
                    PropertyFactory.textField(Expression.get("label")),
                    PropertyFactory.textFont(arrayOf(FONT_REGULAR)),
                    PropertyFactory.textSize(10f),
                    PropertyFactory.textColor(palette.muted.toArgb()),
                ),
        )
        style.addLayer(
            SymbolLayer("campus-amenities", MARKS)
                .withFilter(Expression.eq(kind, Expression.literal("amenity")))
                .withProperties(PropertyFactory.iconImage(Expression.get("icon"))),
        )
        style.addLayer(
            SymbolLayer(ENTRANCES, MARKS)
                .withFilter(Expression.eq(kind, Expression.literal("entrance")))
                .withProperties(
                    PropertyFactory.iconImage(Expression.get("icon")),
                    PropertyFactory.iconAllowOverlap(true),
                ),
        )
        style.addLayer(
            SymbolLayer("campus-labels-lit", MARKS)
                .withFilter(Expression.all(Expression.eq(kind, Expression.literal("room")), Expression.eq(lit, Expression.literal(true))))
                .withProperties(
                    PropertyFactory.textField(Expression.get("label")),
                    PropertyFactory.textFont(arrayOf(FONT_BOLD)),
                    PropertyFactory.textSize(13f),
                    PropertyFactory.textColor(palette.accent.toArgb()),
                    PropertyFactory.textAllowOverlap(true),
                ),
        )
    }

    private fun source(role: CampusMapLayerRole) = "campus-${role.name.lowercase()}"

    private fun fill(role: CampusMapLayerRole, color: Int): Layer =
        FillLayer("${source(role)}-fill", source(role)).withProperties(
            PropertyFactory.fillColor(color),
            // The rim is the stroke layer's; a fill's own edge is a pixel.
            PropertyFactory.fillAntialias(true),
        )

    private fun stroke(role: CampusMapLayerRole, color: Int, width: Float): Layer =
        LineLayer("${source(role)}-stroke", source(role)).withProperties(
            PropertyFactory.lineColor(color),
            PropertyFactory.lineWidth(width),
            PropertyFactory.lineJoin(Property.LINE_JOIN_ROUND),
        )

    /** Lines keep their physical width as the map zooms: each feature
     * carries its width at two zoom levels (`lineFeature`), and a width
     * doubles with every level between them. */
    private fun metricLine(role: CampusMapLayerRole, color: Int): Layer =
        LineLayer("${source(role)}-line", source(role)).withProperties(
            PropertyFactory.lineColor(color),
            PropertyFactory.lineCap(Property.LINE_CAP_ROUND),
            PropertyFactory.lineJoin(Property.LINE_JOIN_ROUND),
            PropertyFactory.lineWidth(
                Expression.interpolate(
                    Expression.exponential(2f), Expression.zoom(),
                    Expression.stop(LOW_ZOOM, Expression.get("low")),
                    Expression.stop(HIGH_ZOOM, Expression.get("high")),
                ),
            ),
        )

    // Layers

    private fun drawLayers(style: Style, layers: List<CampusMapLayer>) {
        for (role in CampusMapLayerRole.entries) {
            val mine = layers.filter { it.role == role }
            val ids = mine.map { it.id }
            if (drawn[role] == ids) continue
            val features = when (role) {
                CampusMapLayerRole.DETAIL_LINE, CampusMapLayerRole.WALL_LINE -> mine.mapNotNull(::lineFeature)
                else -> mine.flatMap { layer -> layer.polygons.mapNotNull { polygonFeature(it.rings) } }
            }
            style.getSourceAs<GeoJsonSource>(source(role))?.setGeoJson(collection(features))
            drawn[role] = ids
        }
    }

    private fun collection(features: List<String>) =
        features.joinToString(",", prefix = """{"type":"FeatureCollection","features":[""", postfix = "]}")

    private fun StringBuilder.path(points: List<CampusMapPoint>) {
        append('[')
        points.forEachIndexed { index, point ->
            if (index > 0) append(',')
            append('[').append(point.longitude).append(',').append(point.latitude).append(']')
        }
        append(']')
    }

    /** One outer ring, then its holes. */
    private fun polygonFeature(rings: List<List<CampusMapPoint>>): String? {
        if ((rings.firstOrNull()?.size ?: 0) < 3) return null
        return buildString {
            append("""{"type":"Feature","properties":{},"geometry":{"type":"Polygon","coordinates":[""")
            rings.forEachIndexed { index, ring ->
                if (index > 0) append(',')
                path(ring)
            }
            append("]}}")
        }
    }

    /** One of Core's line layers, which has one physical width: its lines,
     * and that width in screen units at the two zoom levels `metricLine`
     * reads, never under a hairline. */
    private fun lineFeature(layer: CampusMapLayer): String? {
        val lines = layer.lines.filter { it.size > 1 }
        val latitude = lines.firstOrNull()?.first()?.latitude ?: return null
        fun width(zoom: Float): Double {
            val perMeter = 512.0 * 2.0.pow(zoom.toDouble()) / (EQUATOR_M * cos(Math.toRadians(latitude)).coerceAtLeast(0.01))
            return (layer.widthM * perMeter).coerceAtLeast(HAIRLINE)
        }
        return buildString {
            append("""{"type":"Feature","properties":{"low":""").append(width(LOW_ZOOM))
            append(""","high":""").append(width(HIGH_ZOOM))
            append("""},"geometry":{"type":"MultiLineString","coordinates":[""")
            lines.forEachIndexed { index, line ->
                if (index > 0) append(',')
                path(line)
            }
            append("]}}")
        }
    }

    // Marks

    private fun drawMarks(style: Style, records: List<CampusMapMark>) {
        if (drawnMarks == records) return
        drawnMarks = records
        val features = records.map { record ->
            val properties = JSONObject()
            when (val kind = record.kind) {
                is CampusMapMarkKind.Room -> {
                    properties.put("kind", "room")
                    properties.put("label", kind.label)
                    properties.put("lit", kind.highlighted)
                }
                is CampusMapMarkKind.Amenity -> {
                    properties.put("kind", "amenity")
                    properties.put("icon", amenityImage(kind.key))
                }
                is CampusMapMarkKind.Entrance -> {
                    properties.put("kind", "entrance")
                    properties.put("entrance", kind.entranceId)
                    properties.put("icon", if (kind.selected) ENTRANCE_SELECTED_IMAGE else ENTRANCE_IMAGE)
                }
            }
            """{"type":"Feature","properties":$properties,"geometry":{"type":"Point","coordinates":[${record.point.longitude},${record.point.latitude}]}}"""
        }
        style.getSourceAs<GeoJsonSource>(MARKS)?.setGeoJson(collection(features))
    }

    /** An icon on a 28 dp disc with a hairline rim, as iOS draws them. */
    private fun round(drawable: Int, tint: Int, fill: Int, palette: Palette): Bitmap {
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
        return bitmap
    }

    private fun amenityImage(key: String) = "campus-amenity-$key"

    private companion object {
        const val ICON_DP = 28
        /** How far from an entrance's centre a tap still opens it. */
        const val TOUCH_DP = 18
        const val FRAMING_MS = 400
        /** The equator's length, which MapLibre's zoom-0 tile of 512 spans. */
        const val EQUATOR_M = 40_075_016.686
        const val HAIRLINE = 0.5
        const val LOW_ZOOM = 10f
        const val HIGH_ZOOM = 24f
        const val MARKS = "campus-marks"
        const val ENTRANCES = "campus-entrances"
        const val ENTRANCE_IMAGE = "campus-entrance"
        const val ENTRANCE_SELECTED_IMAGE = "campus-entrance-selected"
        /** OpenFreeMap's quiet styles, and two fonts its glyph server has. */
        const val LIGHT_STYLE = "https://tiles.openfreemap.org/styles/positron"
        const val DARK_STYLE = "https://tiles.openfreemap.org/styles/dark"
        const val FONT_REGULAR = "Noto Sans Regular"
        const val FONT_BOLD = "Noto Sans Bold"
    }
}

/** The drawable of each amenity, by Core's amenity key; the same pictures
 * iOS holds as `CampusMap/<key>`. */
private val amenityDrawables = mapOf(
    "aed" to R.drawable.campus_map_aed,
    "calm-down" to R.drawable.campus_map_calm_down,
    "elevator" to R.drawable.campus_map_elevator,
    "lounge" to R.drawable.campus_map_lounge,
    "lounge-international" to R.drawable.campus_map_lounge_international,
    "lounge-women" to R.drawable.campus_map_lounge_women,
    "rest-room" to R.drawable.campus_map_rest_room,
    "restroom" to R.drawable.campus_map_restroom,
    "restroom-accessible" to R.drawable.campus_map_restroom_accessible,
    "restroom-female" to R.drawable.campus_map_restroom_female,
    "restroom-male" to R.drawable.campus_map_restroom_male,
    "stairs" to R.drawable.campus_map_stairs,
    "study-room" to R.drawable.campus_map_study_room,
)
