package app.isctcompanion.features.map

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import androidx.compose.ui.graphics.toArgb
import app.isctcompanion.R
import app.isctcompanion.core.CampusMapLayer
import app.isctcompanion.core.CampusMapLayerRole
import app.isctcompanion.core.CampusMapMark
import app.isctcompanion.core.CampusMapMarkKind
import app.isctcompanion.core.CampusMapPoint
import app.isctcompanion.design.Palette
import org.maplibre.android.maps.Style
import org.maplibre.android.style.expressions.Expression
import org.maplibre.android.style.layers.FillLayer
import org.maplibre.android.style.layers.Layer
import org.maplibre.android.style.layers.LineLayer
import org.maplibre.android.style.layers.Property
import org.maplibre.android.style.layers.PropertyFactory
import org.maplibre.android.style.layers.SymbolLayer
import org.maplibre.android.style.sources.GeoJsonSource
import org.maplibre.geojson.Feature
import org.maplibre.geojson.FeatureCollection
import org.maplibre.geojson.MultiLineString
import org.maplibre.geojson.Point
import org.maplibre.geojson.Polygon
import kotlin.math.cos
import kotlin.math.pow
import kotlin.math.roundToInt

/** Core's scene as the content of one loaded MapLibre style: every role has
 * one GeoJSON source and its style layers (line widths in metres), and the
 * marks one more, placed by MapLibre's own symbol collision. A source is
 * rewritten only when the ids of Core's layers in it change, so unchanged
 * geometry stays as it is when another part of the scene changes. The
 * features carry only Core's coordinates and ids. */
internal class SceneStyle(private val context: Context, val style: Style, palette: Palette) {
    private val density = context.resources.displayMetrics.density
    /** Per role, the ids of Core's layers its source holds now. */
    private val drawn = HashMap<CampusMapLayerRole, List<String>>()

    /** The marks as last drawn. */
    var marks: List<CampusMapMark> = emptyList()
        private set

    init {
        prepare(palette)
    }

    /** Footprints go under the base map's labels, the floor above them, as
     * on iOS; the marks are last, and of them the ones MapLibre must keep. */
    private fun prepare(palette: Palette) {
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
        // is the priority: building names give way to everything, room
        // labels to amenities, and entrances and the selected room always
        // show.
        val kind = Expression.get("kind")
        val lit = Expression.get("lit")
        style.addLayer(
            SymbolLayer(BUILDINGS, MARKS)
                .withFilter(Expression.eq(kind, Expression.literal("building")))
                .withProperties(
                    PropertyFactory.textField(Expression.get("label")),
                    PropertyFactory.textFont(arrayOf(FONT_BOLD)),
                    PropertyFactory.textSize(11f),
                    PropertyFactory.textColor(palette.ink.toArgb()),
                    PropertyFactory.textHaloColor(palette.surface.toArgb()),
                    PropertyFactory.textHaloWidth(1.5f),
                ),
        )
        style.addLayer(
            SymbolLayer(LABELS, MARKS)
                .withFilter(Expression.all(Expression.eq(kind, Expression.literal("room")), Expression.eq(lit, Expression.literal(false))))
                .withProperties(
                    PropertyFactory.textField(Expression.get("label")),
                    PropertyFactory.textFont(arrayOf(FONT_REGULAR)),
                    PropertyFactory.textSize(10f),
                    PropertyFactory.textColor(palette.muted.toArgb()),
                ),
        )
        style.addLayer(
            SymbolLayer(AMENITIES, MARKS)
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
            SymbolLayer(LIT_LABELS, MARKS)
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

    // Drawing

    /** Rewrites what changed; true when the marks did. */
    fun draw(layers: List<CampusMapLayer>, marks: List<CampusMapMark>): Boolean {
        for (role in CampusMapLayerRole.entries) {
            val mine = layers.filter { it.role == role }
            val ids = mine.map { it.id }
            if (drawn[role] == ids) continue
            val features = when (role) {
                CampusMapLayerRole.DETAIL_LINE, CampusMapLayerRole.WALL_LINE -> mine.mapNotNull(::lineFeature)
                else -> mine.flatMap { layer -> layer.polygons.mapNotNull { polygonFeature(it.rings) } }
            }
            style.getSourceAs<GeoJsonSource>(source(role))?.setGeoJson(FeatureCollection.fromFeatures(features))
            drawn[role] = ids
        }
        if (this.marks == marks) return false
        this.marks = marks
        style.getSourceAs<GeoJsonSource>(MARKS)?.setGeoJson(FeatureCollection.fromFeatures(marks.map(::markFeature)))
        return true
    }

    private fun path(points: List<CampusMapPoint>) = points.map { Point.fromLngLat(it.longitude, it.latitude) }

    /** One outer ring, then its holes. */
    private fun polygonFeature(rings: List<List<CampusMapPoint>>): Feature? {
        if ((rings.firstOrNull()?.size ?: 0) < 3) return null
        return Feature.fromGeometry(Polygon.fromLngLats(rings.map(::path)))
    }

    /** One of Core's line layers, which has one physical width: its lines,
     * and that width in screen units at the two zoom levels `metricLine`
     * reads, never under a hairline. */
    private fun lineFeature(layer: CampusMapLayer): Feature? {
        val lines = layer.lines.filter { it.size > 1 }
        val latitude = lines.firstOrNull()?.first()?.latitude ?: return null
        fun width(zoom: Float): Double {
            val perMeter = 512.0 * 2.0.pow(zoom.toDouble()) / (EQUATOR_M * cos(Math.toRadians(latitude)).coerceAtLeast(0.01))
            return (layer.widthM * perMeter).coerceAtLeast(HAIRLINE)
        }
        return Feature.fromGeometry(MultiLineString.fromLngLats(lines.map(::path))).apply {
            addNumberProperty("low", width(LOW_ZOOM))
            addNumberProperty("high", width(HIGH_ZOOM))
        }
    }

    private fun markFeature(mark: CampusMapMark): Feature =
        Feature.fromGeometry(Point.fromLngLat(mark.point.longitude, mark.point.latitude)).apply {
            when (val kind = mark.kind) {
                is CampusMapMarkKind.Room -> {
                    addStringProperty("kind", "room")
                    addStringProperty("unit", kind.unitId)
                    addStringProperty("label", kind.label)
                    addBooleanProperty("lit", kind.highlighted)
                }
                is CampusMapMarkKind.Amenity -> {
                    addStringProperty("kind", "amenity")
                    addStringProperty("icon", amenityImage(kind.key))
                }
                is CampusMapMarkKind.Building -> {
                    addStringProperty("kind", "building")
                    addStringProperty("building", kind.buildingId)
                    addStringProperty("label", kind.label)
                }
                is CampusMapMarkKind.Entrance -> {
                    addStringProperty("kind", "entrance")
                    addStringProperty("entrance", kind.entranceId)
                    addStringProperty("icon", if (kind.selected) ENTRANCE_SELECTED_IMAGE else ENTRANCE_IMAGE)
                }
            }
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

    companion object {
        /** The mark layers a tap looks in, topmost first, as drawn. */
        val TAPPABLE = arrayOf(LIT_LABELS, ENTRANCES, AMENITIES, LABELS, BUILDINGS)
        private const val LIT_LABELS = "campus-labels-lit"
        private const val ENTRANCES = "campus-entrances"
        private const val AMENITIES = "campus-amenities"
        private const val LABELS = "campus-labels"
        private const val BUILDINGS = "campus-buildings"
        private const val MARKS = "campus-marks"
        private const val ICON_DP = 28
        /** The equator's length, which MapLibre's zoom-0 tile of 512 spans. */
        private const val EQUATOR_M = 40_075_016.686
        private const val HAIRLINE = 0.5
        private const val LOW_ZOOM = 10f
        private const val HIGH_ZOOM = 24f
        private const val ENTRANCE_IMAGE = "campus-entrance"
        private const val ENTRANCE_SELECTED_IMAGE = "campus-entrance-selected"
        /** Two fonts OpenFreeMap's glyph server has. */
        private const val FONT_REGULAR = "Noto Sans Regular"
        private const val FONT_BOLD = "Noto Sans Bold"
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
